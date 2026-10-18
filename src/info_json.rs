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
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        let mut name = path.as_os_str().to_owned();
        name.push(".info.json");
        return name.into();
    };
    let stem = match splitext(name) {
        Some((stem, real)) if ext.is_none_or(|ext| real == ext) => stem,
        _ => name,
    };
    path.with_file_name(format!("{stem}.info.json"))
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
    match std::fs::symlink_metadata(path) {
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
