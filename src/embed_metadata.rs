//! Upstream `FFmpegMetadataPP` metadata options for represented fields.
//! Chapters do not exist in the represented subset; info JSON attachment is
//! not implemented.

use crate::{StreamKind, VideoInfo};
use serde_json::Value;

/// Python `str(value)` for JSON scalars.
fn py_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        Value::Number(number) => match (number.as_i64(), number.as_u64(), number.as_f64()) {
            (Some(n), _, _) => n.to_string(),
            (_, Some(n), _) => n.to_string(),
            (_, _, Some(f)) if f.fract() == 0.0 && f.abs() < 1e16 => format!("{f:.1}"),
            (_, _, Some(f)) => f.to_string(),
            _ => number.to_string(),
        },
        other => other.to_string(),
    }
}

/// `FFmpegMetadataPP._options(ext)` followed by `_get_metadata_opts(info)`.
pub(crate) fn args(info: &VideoInfo, ext: &str) -> Vec<String> {
    let value = serde_json::to_value(info).unwrap_or(Value::Null);
    let dict = value.as_object().cloned().unwrap_or_default();
    let mut args: Vec<String> = ["-map", "0", "-dn", "-ignore_unknown"]
        .into_iter()
        .map(Into::into)
        .collect();
    if ext == "m4a" {
        args.extend(["-vn", "-acodec", "copy"].map(String::from));
    } else {
        args.extend(["-c", "copy"].map(String::from));
    }
    let mut common: Vec<(String, String)> = Vec::new();
    let mut add = |names: &[&str], keys: &[&str]| {
        let found = std::iter::once("meta_")
            .chain(keys.iter().copied())
            .find_map(|key| dict.get(key).filter(|value| !value.is_null()));
        // Like upstream, the emptiness check precedes joining lists.
        let Some(found) = found.filter(|value| value.as_str() != Some("")) else {
            return;
        };
        let joined = match found {
            Value::Array(items) => items.iter().map(py_str).collect::<Vec<_>>().join(", "),
            other => py_str(other),
        }
        .replace('\0', "");
        for name in names {
            match common.iter_mut().find(|(existing, _)| existing == name) {
                Some(entry) => entry.1 = joined.clone(),
                None => common.push(((*name).into(), joined.clone())),
            }
        }
    };
    add(&["title"], &["track", "title"]);
    add(&["date"], &["upload_date"]);
    add(&["description", "synopsis"], &["description"]);
    add(&["purl", "comment"], &["webpage_url"]);
    add(&["track"], &["track_number"]);
    add(
        &["artist"],
        &[
            "artist",
            "artists",
            "creator",
            "creators",
            "uploader",
            "uploader_id",
        ],
    );
    add(&["composer"], &["composer", "composers"]);
    add(&["genre"], &["genre", "genres", "categories", "tags"]);
    add(&["album"], &["album", "series"]);
    add(&["album_artist"], &["album_artist", "album_artists"]);
    add(&["disc"], &["disc_number"]);
    add(&["show"], &["series"]);
    add(&["season_number"], &["season_number"]);
    add(&["episode_id"], &["episode", "episode_id"]);
    add(&["episode_sort"], &["episode_number"]);
    args.extend(["-write_id3v1", "1"].map(String::from));
    for (name, value) in common {
        args.push("-metadata".into());
        args.push(format!("{name}={value}"));
    }
    // Per-stream languages for the selected streams (two for muxed formats).
    let selected: Vec<(StreamKind, Option<&str>)> = if info.requested_formats.is_empty() {
        let language = info
            .formats
            .iter()
            .find(|format| Some(&format.format_id) == info.format_id.as_ref())
            .and_then(|format| format.language.as_deref());
        vec![(info.stream_kind, language)]
    } else {
        info.requested_formats
            .iter()
            .map(|format| (format.stream_kind, format.language.as_deref()))
            .collect()
    };
    let mut stream = 0;
    for (kind, language) in selected {
        let count = if kind == StreamKind::Standalone { 2 } else { 1 };
        let language = language.unwrap_or("");
        let language = if language.chars().count() == 3 {
            language.to_lowercase()
        } else {
            let lower = language.to_lowercase();
            crate::embed_subtitles::short_to_long(&lower)
                .map_or_else(|| language.to_owned(), str::to_owned)
        };
        for index in stream..stream + count {
            if !language.is_empty() {
                args.push(format!("-metadata:s:{index}"));
                args.push(format!("language={language}"));
            }
        }
        stream += count;
    }
    args
}

/// The output muxer for the processed container.
pub(crate) fn muxer(ext: &str) -> &str {
    match ext {
        "mkv" | "mka" => "matroska",
        "m4a" => "ipod",
        "aac" => "adts",
        "opus" | "ogg" | "oga" => "ogg",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_options_follow_upstream_key_precedence() {
        let info: VideoInfo = serde_json::from_value(serde_json::json!({
            "id": "v", "title": "Title", "ext": "mp4", "url": "https://e.invalid/v.mp4",
            "webpage_url": "https://e.invalid/watch", "extractor": "x", "extractor_key": "X",
            "direct": false, "upload_date": "20080529", "description": "Desc\u{0}ription",
            "uploader_id": "@Handle", "tags": [], "format_id": "18",
            "formats": [{"format_id": "18", "ext": "mp4", "url": "https://e.invalid/v.mp4", "language": "en"}],
        }))
        .unwrap();
        assert_eq!(
            args(&info, "mp4"),
            [
                "-map",
                "0",
                "-dn",
                "-ignore_unknown",
                "-c",
                "copy",
                "-write_id3v1",
                "1",
                "-metadata",
                "title=Title",
                "-metadata",
                "date=20080529",
                "-metadata",
                "description=Description",
                "-metadata",
                "synopsis=Description",
                "-metadata",
                "purl=https://e.invalid/watch",
                "-metadata",
                "comment=https://e.invalid/watch",
                "-metadata",
                "artist=@Handle",
                "-metadata",
                "genre=",
                "-metadata:s:0",
                "language=eng",
                "-metadata:s:1",
                "language=eng",
            ]
        );
        assert_eq!(&args(&info, "m4a")[4..7], ["-vn", "-acodec", "copy"]);
    }
}
