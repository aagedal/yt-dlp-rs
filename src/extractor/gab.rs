//! Upstream `GabIE` (`yt_dlp/extractor/gab.py`): the `gab.com/api/v1`
//! status JSON, each video/gifv attachment's original and playable MP4 as
//! formats, and a playlist when the post has more than one video.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Gab;

const NAME: &str = "Gab";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?gab\.com/[^/]+/posts/(?P<id>\d+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Gab {what} are not supported natively"))
}

/// `int_or_none(value, scale=SCALE)`.
fn int_or_none(value: Option<&Value>, scale: i64) -> Value {
    let value = match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    };
    value.map_or(Value::Null, |value| Value::from(value.div_euclid(scale)))
}

/// Python truthiness.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
    }
}

fn get(map: &Map<String, Value>, key: &str) -> Value {
    map.get(key).cloned().unwrap_or(Value::Null)
}

fn optional_str(value: Option<&Value>, what: &str) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(unsupported(what)),
    }
}

/// One attachment's entry dict.
fn entry(
    url: &Url,
    post_id: &str,
    idx: usize,
    data: &Map<String, Value>,
    media: &Map<String, Value>,
) -> Result<VideoInfo> {
    let metadata = media
        .get("meta")
        .and_then(Value::as_object)
        .ok_or_else(|| unsupported("video attachments without metadata"))?;
    let audio_encode = optional_str(
        metadata.get("audio_encode"),
        "audio encodings that are not strings",
    )?;
    let acodec = crate::codecs::parse(audio_encode.as_deref())
        .map(|codecs| codecs.acodec)
        .map_or(Value::Null, Value::String);
    let audio_bitrate = optional_str(
        metadata.get("audio_bitrate"),
        "audio bitrates that are not strings",
    )?
    .unwrap_or_default();
    let asr = int_or_none(
        Some(&Value::String(
            audio_bitrate
                .split(' ')
                .next()
                .unwrap_or_default()
                .to_owned(),
        )),
        1,
    );
    let fps = get(metadata, "fps");
    let empty = Map::new();
    let mut formats = Vec::new();
    for (media_key, meta_key) in [("url", "original"), ("source_mp4", "playable")] {
        let Some(f_url) = media.get(media_key).filter(|value| truthy(value)) else {
            continue;
        };
        let Value::String(f_url) = f_url else {
            return Err(unsupported("media URLs that are not strings"));
        };
        let f = match metadata.get(meta_key) {
            Some(Value::Object(f)) if !f.is_empty() => f,
            _ => &empty,
        };
        let mut map = Map::new();
        map.insert("url".into(), f_url.clone().into());
        map.insert("width".into(), get(f, "width"));
        map.insert("height".into(), get(f, "height"));
        map.insert("tbr".into(), int_or_none(f.get("bitrate"), 1000));
        map.insert("acodec".into(), acodec.clone());
        map.insert("asr".into(), asr.clone());
        map.insert("fps".into(), fps.clone());
        let keys: Vec<String> = map.keys().cloned().collect();
        let ext = super::direct::determine_ext(f_url).unwrap_or_else(|| "unknown_video".into());
        map.insert("ext".into(), ext.into());
        // Processing numbers the formats without an ID once sorted.
        map.insert("format_id".into(), "".into());
        let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
            .map_err(|_| unsupported("format fields of unexpected types"))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    let author = data
        .get("account")
        .and_then(Value::as_object)
        .ok_or_else(|| unsupported("posts without an account"))?;
    let display_name = match author.get("display_name") {
        Some(Value::String(name)) => name.clone(),
        _ => return Err(unsupported("accounts without a string display name")),
    };
    let timestamp = optional_str(
        data.get("created_at"),
        "creation dates that are not strings",
    )?
    .and_then(|date| crate::dates::unified_timestamp(&date, false))
    .map_or(Value::Null, Value::from);
    let description = optional_str(data.get("content"), "contents that are not strings")?
        .map(|content| crate::html::clean_html(&content))
        .map_or(Value::Null, Value::String);
    let duration = match metadata.get("duration").filter(|value| truthy(value)) {
        Some(duration) => Some(duration.clone()),
        None => optional_str(metadata.get("length"), "lengths that are not strings")?
            .and_then(|length| crate::parse_duration(&length))
            .map(|duration| serde_json::to_value(duration).unwrap_or(Value::Null)),
    };
    let no_formats = formats.is_empty();
    let first = formats
        .first()
        .map_or_else(|| url.clone(), |format| format.url.clone());
    let mut info = super::direct::build_at(first.clone(), &first, String::new());
    info.formats = formats;
    info.direct = None;
    info.id = format!("{post_id}-{idx}");
    info.title = format!("{display_name} on Gab");
    info.webpage_url = url.clone();
    info.extractor = NAME.into();
    info.extractor_key = NAME.into();
    if no_formats {
        info.extra
            .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
    }
    let mut keys = vec!["id", "title", "timestamp", "formats", "description"];
    crate::metadata_parser::set_field(&mut info, "timestamp", timestamp);
    crate::metadata_parser::set_field(&mut info, "description", description);
    if let Some(duration) = duration {
        crate::metadata_parser::set_field(&mut info, "duration", duration);
        keys.push("duration");
    }
    for (key, value) in [
        ("like_count", get(data, "favourites_count")),
        ("comment_count", get(data, "replies_count")),
        ("repost_count", get(data, "reblogs_count")),
        ("uploader", get(author, "username")),
        ("uploader_id", get(author, "id")),
        ("uploader_url", get(author, "url")),
    ] {
        crate::metadata_parser::set_field(&mut info, key, value);
        keys.push(key);
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!("dict;{}", keys.join(",")),
    ))
}

impl Extractor for Gab {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let post_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!("https://gab.com/api/v1/statuses/{post_id}"))?;
        let data = download_json(client, &api, NAME, &post_id)?;
        let data = data
            .as_object()
            .ok_or_else(|| unsupported("status responses that are not objects"))?;
        let attachments = data
            .get("media_attachments")
            .and_then(Value::as_array)
            .ok_or_else(|| unsupported("posts without a media attachment list"))?;
        let mut entries = Vec::new();
        for (idx, media) in attachments.iter().enumerate() {
            let media = media
                .as_object()
                .ok_or_else(|| unsupported("attachments that are not objects"))?;
            if !matches!(
                media.get("type").and_then(Value::as_str),
                Some("video" | "gifv")
            ) {
                continue;
            }
            entries.push(entry(url, &post_id, idx, data, media)?);
        }
        if entries.len() > 1 {
            let mut extra = Map::new();
            extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                "playlist-dict;id".into(),
            );
            extra.insert("title".into(), Value::Null);
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id: post_id,
                title: String::new(),
                webpage_url: url.clone(),
                extractor: NAME.into(),
                extractor_key: NAME.into(),
                nested: Vec::new(),
                pending: None,
                entries,
                extra,
            })));
        }
        match entries.pop() {
            Some(info) => Ok(Some(ExtractResult::Video(info))),
            None => Err(unsupported("posts without video attachments")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://gab.com/ACT1TV/posts/107163961867310434"));
        assert!(suitable("https://www.gab.com/x/posts/1"));
        assert!(!suitable("https://gab.com/ACT1TV"));
    }
}
