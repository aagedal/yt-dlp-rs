//! Info JSON sidecars: upstream's clean `sanitize_info`, `.info.json` naming
//! and atomic writes without overwriting existing files.
use crate::{Error, PlaylistCounts, PlaylistInfo, Result, VideoInfo};
use serde_json::{Map, Value};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

/// Keys removed by upstream's default `clean_infojson`, besides `__` keys and nulls.
const PRIVATE_KEYS: &[&str] = &[
    "requested_downloads",
    "requested_formats",
    "requested_subtitles",
    "requested_entries",
    "entries",
    "filepath",
    "_filename",
    "filename",
    "infojson_filename",
    "original_url",
    "playlist_autonumber",
];

/// Result of an info JSON write; existing files are kept as upstream does by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoJsonWrite {
    Written,
    AlreadyPresent,
}

/// Upstream `replace_extension(name, "info.json", ext)`: replace the final
/// extension when it matches `ext` (or any extension when `ext` is absent),
/// otherwise append. Extension splitting follows Python's `os.path.splitext`.
pub fn info_json_path(path: &Path, ext: Option<&str>) -> PathBuf {
    let Some(text) = path.to_str() else {
        let mut name = path.as_os_str().to_owned();
        name.push(".info.json");
        return name.into();
    };
    // Split the text directly so literal template separators are preserved.
    let start = text
        .rfind(std::path::is_separator)
        .map_or(0, |index| index + 1);
    let name = &text[start..];
    let stem = match splitext(name) {
        Some((stem, real)) if ext.is_none_or(|ext| real == ext) => stem,
        _ => name,
    };
    format!("{}{stem}.info.json", &text[..start]).into()
}

/// Python `splitext` on one component: the last dot preceded by a non-dot.
fn splitext(name: &str) -> Option<(&str, &str)> {
    let dot = name.rfind('.')?;
    name[..dot]
        .bytes()
        .any(|byte| byte != b'.')
        .then(|| (&name[..dot], &name[dot + 1..]))
}

/// The cleaned video info dictionary, with upstream's `_type` and `epoch` defaults.
/// `_version` is omitted because this is not an upstream release.
pub fn video_info_json(info: &VideoInfo, epoch: u64) -> Result<Value> {
    finish(to_value(info)?, "video", epoch)
}

/// The cleaned playlist dictionary written before its entries are processed.
pub fn playlist_info_json(
    info: &PlaylistInfo,
    counts: PlaylistCounts,
    epoch: u64,
) -> Result<Value> {
    let mut value = to_value(info)?;
    if let Value::Object(map) = &mut value {
        map.insert("playlist_count".into(), counts.playlist_count.into());
    }
    finish(value, "playlist", epoch)
}

fn to_value(value: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| Error::InfoJson(error.to_string()))
}

fn finish(value: Value, kind: &str, epoch: u64) -> Result<Value> {
    let Value::Object(mut map) = clean(value) else {
        return Err(Error::InfoJson("info is not a JSON object".into()));
    };
    map.entry("epoch").or_insert(epoch.into());
    map.entry("_type").or_insert(kind.into());
    Ok(Value::Object(map))
}

fn clean(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(key, value)| {
                    !value.is_null()
                        && !key.starts_with("__")
                        && !PRIVATE_KEYS.contains(&key.as_str())
                })
                .map(|(key, value)| (key, clean(value)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(clean).collect()),
        value => value,
    }
}

/// Python `json.dump(obj, ensure_ascii=False)` bytes: `", "` and `": "` separators.
pub fn to_python_json(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, PythonFormatter);
    serde::Serialize::serialize(value, &mut serializer)
        .map_err(|error| Error::InfoJson(error.to_string()))?;
    Ok(bytes)
}

struct PythonFormatter;

impl serde_json::ser::Formatter for PythonFormatter {
    fn begin_array_value<W: ?Sized + Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
    fn begin_object_key<W: ?Sized + Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
    fn begin_object_value<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b": ")
    }
}

/// Write `value` to `path` unless a file already exists there. Parent directories
/// are created, data is synced in a sibling temporary file, and publication
/// never replaces a concurrently created destination.
pub fn write_info_json(path: &Path, value: &Value) -> Result<InfoJsonWrite> {
    write_info_json_with(path, value, false)
}

/// Like [`write_info_json`]; with `overwrite`, an existing regular file is
/// atomically replaced (upstream's default `overwrites` for metadata files).
pub fn write_info_json_with(path: &Path, value: &Value, overwrite: bool) -> Result<InfoJsonWrite> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if overwrite && metadata.is_file() => {}
        Ok(_) if std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) => {
            return Ok(InfoJsonWrite::AlreadyPresent);
        }
        Ok(_) => {
            return Err(Error::InfoJson(format!(
                "destination is not a regular file: {}",
                path.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let bytes = to_python_json(value)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".yt-dlp-rs-").suffix(".info.json.tmp");
    // Like upstream, publish with the default 0666 mode reduced by the umask.
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
    let mut temporary = builder.tempfile_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    if overwrite {
        temporary.persist(path).map_err(|error| error.error)?;
        return Ok(InfoJsonWrite::Written);
    }
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(InfoJsonWrite::Written),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            Err(Error::OutputExists(path.into()))
        }
        Err(error) => Err(error.error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn info_json_names_follow_replace_extension() {
        let cases = [
            ("a/Title [id].mp4", Some("mp4"), "a/Title [id].info.json"),
            ("Title.webm", Some("mp4"), "Title.webm.info.json"),
            ("Title", Some("mp4"), "Title.info.json"),
            ("Title [x].NA", None, "Title [x].info.json"),
            ("Title.", None, "Title.info.json"),
            ("Title.", Some(""), "Title.info.json"),
            (".hidden", None, ".hidden.info.json"),
            ("..foo", Some("foo"), "..foo.info.json"),
            ("a..mp4", Some("mp4"), "a..info.json"),
            ("dir.d/name", None, "dir.d/name.info.json"),
            ("dir/", None, "dir/.info.json"),
        ];
        for (path, ext, expected) in cases {
            assert_eq!(
                info_json_path(Path::new(path), ext),
                PathBuf::from(expected),
                "{path}"
            );
        }
    }

    #[test]
    fn cleaning_removes_private_keys_and_nulls_recursively_and_adds_defaults() {
        let value = json!({
            "id": "x", "requested_subtitles": {"en": {}}, "__private": 1,
            "playlist_autonumber": 2, "nested": [{"filepath": "p", "keep": null, "ok": 1}],
            "playlist_index": 3,
        });
        let value = finish(value, "video", 7).unwrap();
        assert_eq!(
            value,
            json!({"id": "x", "nested": [{"ok": 1}], "playlist_index": 3, "epoch": 7, "_type": "video"})
        );
        let order: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(order, ["id", "nested", "playlist_index", "epoch", "_type"]);
    }

    #[test]
    fn python_json_separators_and_unescaped_unicode() {
        let value = json!({"t": "Ép\u{1}\"/", "a": [1, 2.5, true], "o": {}});
        assert_eq!(
            String::from_utf8(to_python_json(&value).unwrap()).unwrap(),
            r#"{"t": "Ép\u0001\"/", "a": [1, 2.5, true], "o": {}}"#
        );
    }

    #[test]
    fn writes_are_atomic_and_never_replace_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/x.info.json");
        assert_eq!(
            write_info_json(&path, &json!({"a": 1})).unwrap(),
            InfoJsonWrite::Written
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"a": 1}"#);
        assert_eq!(
            write_info_json(&path, &json!({"a": 2})).unwrap(),
            InfoJsonWrite::AlreadyPresent
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"a": 1}"#);
        let entries: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(entries.len(), 1);
        std::fs::create_dir(directory.path().join("dir.info.json")).unwrap();
        assert!(write_info_json(&directory.path().join("dir.info.json"), &json!({})).is_err());
    }

    #[test]
    fn saved_upstream_cases_match() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/info_json/cases.json")).unwrap();
        for case in fixture["filenames"].as_array().unwrap() {
            let path = info_json_path(
                Path::new(case["filename"].as_str().unwrap()),
                case["ext"].as_str(),
            );
            assert_eq!(path.to_str().unwrap(), case["expected"], "{}", case["name"]);
        }
        for case in fixture["playlist_names"].as_array().unwrap() {
            let playlist: PlaylistInfo = serde_json::from_value(json!({
                "entries": [],
                "id": case["playlist"]["id"], "title": case["playlist"]["title"],
                "webpage_url": case["playlist"]["webpage_url"],
                "extractor": case["playlist"]["extractor"],
                "extractor_key": case["playlist"]["extractor_key"],
            }))
            .unwrap();
            let counts = PlaylistCounts {
                playlist_count: case["playlist_count"].as_u64().unwrap() as usize,
                n_entries: case["n_entries"].as_u64().unwrap() as usize,
                last_playlist_index: case["last_playlist_index"].as_u64().unwrap() as usize,
            };
            let options = crate::output::TemplateOptions {
                na_placeholder: case["na_placeholder"].as_str().unwrap().into(),
                ..Default::default()
            };
            let name = crate::output::render_playlist_template_with_options(
                case["template"].as_str().unwrap(),
                &playlist,
                counts,
                &options,
            )
            .unwrap();
            let path = info_json_path(&name, None);
            assert_eq!(path.to_str().unwrap(), case["expected"], "{}", case["name"]);
        }
        for case in fixture["sanitize"].as_array().unwrap() {
            let value = finish(case["info"].clone(), "video", 0).unwrap();
            assert_eq!(value, case["expected"], "{}", case["name"]);
            let keys = |value: &Value| {
                value
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
            };
            assert_eq!(keys(&value), keys(&case["expected"]), "{}", case["name"]);
        }
        for case in fixture["json_text"].as_array().unwrap() {
            let text = String::from_utf8(to_python_json(&case["value"]).unwrap()).unwrap();
            assert_eq!(text, case["expected"], "{}", case["name"]);
        }
        // Source-only: upstream's _version describes its own release.
        let version = &fixture["source_only"][0];
        let value = finish(version["info"].clone(), "video", 0).unwrap();
        assert!(
            value
                .get(version["upstream_extra_key"].as_str().unwrap())
                .is_none()
        );
    }
}

/// Upstream `sanitize_info(info, remove_private_keys=True)`: drop `None`
/// values, `__` keys and the request/file bookkeeping keys, recursively.
fn sanitize_loaded(value: Value) -> Value {
    const PRIVATE: &[&str] = &[
        "requested_downloads",
        "requested_formats",
        "requested_subtitles",
        "requested_entries",
        "entries",
        "filepath",
        "_filename",
        "filename",
        "infojson_filename",
        "original_url",
        "playlist_autonumber",
    ];
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(key, value)| {
                    !value.is_null() && !key.starts_with("__") && !PRIVATE.contains(&key.as_str())
                })
                .map(|(key, value)| (key, sanitize_loaded(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(sanitize_loaded).collect()),
        other => other,
    }
}

/// Fill fields this port requires from an upstream-written dictionary:
/// `direct` stays absent unless saved, a format's stream kind follows `vcodec`/
/// `acodec` `"none"`, and a missing top-level `url`/`ext` (a merged
/// selection) comes from the last format, to be replaced by format selection.
fn normalize_loaded(map: &mut serde_json::Map<String, Value>) -> Result<()> {
    if let Some(Value::Array(formats)) = map.get_mut("formats") {
        for format in formats.iter_mut().filter_map(Value::as_object_mut) {
            let protocol = format
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or("https");
            if !matches!(protocol, "http" | "https") && !format.contains_key("fragments") {
                return Err(Error::InfoJson(format!(
                    "loaded format protocol {protocol:?} is unsupported"
                )));
            }
            if !format.contains_key("stream_kind") {
                let none = |key: &str| format.get(key).and_then(Value::as_str) == Some("none");
                let kind = match (none("vcodec"), none("acodec")) {
                    (false, true) => Some("video_only"),
                    (true, false) => Some("audio_only"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    format.insert("stream_kind".into(), kind.into());
                }
            }
        }
    }
    for key in ["url", "ext"] {
        if !map.contains_key(key)
            && let Some(value) = map
                .get("formats")
                .and_then(Value::as_array)
                .and_then(|formats| formats.last())
                .and_then(|format| format.get(key))
                .cloned()
        {
            map.insert(key.into(), value);
        }
    }
    Ok(())
}

/// Upstream `download_with_info_file`: one dictionary or a list of them,
/// sanitized like upstream's default `clean_infojson`. Videos keep their
/// formats for reselection; playlists lose their entries.
pub fn load_info_json(path: &std::path::Path) -> Result<Vec<crate::ExtractResult>> {
    let bytes = std::fs::read(path).map_err(|error| Error::InfoJson(error.to_string()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::InfoJson("info JSON is not valid UTF-8".into()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|error| Error::InfoJson(error.to_string()))?;
    let items = match value {
        Value::Array(items) => items,
        other => vec![other],
    };
    items
        .into_iter()
        .map(|item| {
            let Value::Object(mut map) = sanitize_loaded(item) else {
                return Err(Error::InfoJson("info is not a JSON object".into()));
            };
            let kind = map
                .get("_type")
                .and_then(Value::as_str)
                .unwrap_or("video")
                .to_owned();
            match kind.as_str() {
                "video" => {
                    normalize_loaded(&mut map)?;
                    let mut info: crate::VideoInfo = serde_json::from_value(Value::Object(map))
                        .map_err(|error| Error::InfoJson(error.to_string()))?;
                    info.metadata.fill_common_fields();
                    info.metadata.sanitize_thumbnails();
                    Ok(crate::ExtractResult::Video(info))
                }
                "playlist" => {
                    map.insert("entries".into(), Value::Array(Vec::new()));
                    serde_json::from_value(Value::Object(map))
                        .map(crate::ExtractResult::Playlist)
                        .map_err(|error| Error::InfoJson(error.to_string()))
                }
                other => Err(Error::InfoJson(format!(
                    "loading {other:?} info is unsupported"
                ))),
            }
        })
        .collect()
}

#[cfg(test)]
mod load_tests {
    use super::*;
    use serde_json::json;

    fn load(value: Value) -> Result<Vec<crate::ExtractResult>> {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&value).unwrap()).unwrap();
        load_info_json(file.path())
    }

    #[test]
    fn upstream_dictionaries_are_sanitized_and_normalized() {
        let loaded = load(json!({
            "id": "v", "title": "T", "ext": "mp4", "webpage_url": "https://e.invalid/w",
            "extractor": "x", "extractor_key": "X", "__private": 1, "filename": "f.mp4",
            "requested_formats": [{"format_id": "nope"}], "description": null,
            "formats": [
                {"format_id": "a", "url": "https://e.invalid/a", "ext": "m4a", "vcodec": "none", "acodec": "mp4a"},
                {"format_id": "v", "url": "https://e.invalid/v", "ext": "mp4", "vcodec": "avc1", "acodec": "none", "protocol": "https"},
            ],
        }))
        .unwrap();
        let crate::ExtractResult::Video(info) = &loaded[0] else {
            panic!("video expected");
        };
        assert!(!info.is_direct());
        assert_eq!(info.url.as_str(), "https://e.invalid/v");
        assert!(info.requested_formats.is_empty());
        assert_eq!(info.formats[0].stream_kind, crate::StreamKind::AudioOnly);
        assert_eq!(info.formats[1].stream_kind, crate::StreamKind::VideoOnly);
        let error = load(json!({
            "id": "v", "title": "T", "ext": "mp4", "webpage_url": "https://e.invalid/w",
            "extractor": "x", "extractor_key": "X",
            "formats": [{"format_id": "h", "url": "https://e.invalid/h.m3u8", "ext": "mp4", "protocol": "m3u8_native"}],
        }))
        .unwrap_err();
        assert!(error.to_string().contains("\"m3u8_native\" is unsupported"));
        assert!(load(json!({"_type": "url", "url": "https://e.invalid/"})).is_err());
        assert!(load(json!([1])).is_err());
    }
}
