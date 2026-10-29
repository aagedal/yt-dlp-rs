//! Upstream `ChilloutzoneIE` (`yt_dlp/extractor/chilloutzone.py`): the
//! page's base64 `cozVidData` JSON as its MP4 `mediaUrl`, or a YouTube or
//! Vimeo url result when the native video takes priority.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Chilloutzone;

const NAME: &str = "Chilloutzone";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?chilloutzone\.net/video/(?P<id>[\w-]+)\.html").unwrap()
});
static VIDEO_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var cozVidData\s*=\s*"([^"]+)""#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Chilloutzone {what} are not supported natively"))
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// `int_or_none(value)`: `int()` of numbers (truncated) and integer strings.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().replace('_', "").parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

/// Python's lenient `base64.b64decode`: characters outside the alphabet are
/// discarded before decoding.
fn b64decode(text: &str) -> Option<Vec<u8>> {
    let cleaned: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .ok()
}

impl Extractor for Chilloutzone {
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
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `_html_search_regex` cleans the match.
        let b64_data = VIDEO_DATA
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract video data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let decoded = b64decode(&b64_data)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(|| unsupported("video data that do not decode"))?;
        let info: Value = crate::json::from_str(&decoded).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let Value::Object(info) = info else {
            return Err(unsupported("video data that are not objects"));
        };

        let video_url = info.get("mediaUrl").filter(|value| truthy(value));
        let native_platform = info.get("nativePlatform").filter(|value| truthy(value));
        if let Some(native_platform) = native_platform
            && info.get("sourcePriority").and_then(Value::as_str) == Some("native")
        {
            let native_video_id = info.get("nativeVideoId").ok_or_else(|| {
                Error::Reported(format!(
                    "{video_id}: An extractor error has occurred. (caused by KeyError('nativeVideoId')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
            match native_platform.as_str() {
                Some("youtube") => {
                    let Value::String(native_video_id) = native_video_id else {
                        return Err(unsupported("native video IDs that are not strings"));
                    };
                    return Ok(Some(super::follow_url(url, native_video_id.clone())));
                }
                Some("vimeo") => {
                    let native_video_id = match native_video_id {
                        Value::String(id) => id.clone(),
                        Value::Number(id) => id.to_string(),
                        _ => return Err(unsupported("native video IDs that are not strings")),
                    };
                    return Ok(Some(super::follow_url(
                        url,
                        format!("https://vimeo.com/{native_video_id}"),
                    )));
                }
                _ => {}
            }
        } else if video_url.is_none() {
            // `url_result(url, 'Generic')`.
            return Err(unsupported(
                "pages without a media URL (a Generic url result)",
            ));
        }
        let Some(Value::String(video_url)) = video_url else {
            return Err(unsupported("media URLs that are not strings"));
        };

        // Processing's `sanitize_url`.
        let video_url = super::proto_relative_http(video_url);
        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("url", Value::from(video_url.clone())),
            ("ext", Value::from("mp4")),
        ];
        // `traverse_obj` drops `None` values.
        match info.get("title") {
            None | Some(Value::Null) => {}
            Some(Value::String(title)) => fields.push(("title", title.clone().into())),
            Some(_) => return Err(unsupported("titles that are not strings")),
        }
        if let Some(Value::String(description)) = info.get("description") {
            fields.push(("description", crate::html::clean_html(description).into()));
        }
        for (key, source) in [
            ("duration", "videoLength"),
            ("width", "videoWidth"),
            ("height", "videoHeight"),
        ] {
            if let Some(value) = int_or_none(info.get(source)) {
                fields.push((key, value.into()));
            }
        }
        // The single format is the info dict itself.
        let mut format_fields = serde_json::Map::new();
        format_fields.insert("format_id".into(), "".into());
        format_fields.insert("url".into(), video_url.into());
        format_fields.insert("ext".into(), "mp4".into());
        for (key, value) in &fields {
            if matches!(*key, "width" | "height") {
                format_fields.insert((*key).into(), value.clone());
            }
        }
        let mut format: crate::VideoFormat =
            serde_json::from_value(Value::Object(format_fields))
                .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "url-only");
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("untyped-dict;{}", keys.join(","));
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            dict_info(url, (NAME, NAME), fields, vec![format]),
            &shape,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.chilloutzone.net/video/enemene-meck-alle-katzen-weg.html"
        ));
        assert!(suitable("http://chilloutzone.net/video/icon-blending.html"));
        assert!(!suitable("https://www.chilloutzone.net/video/"));
    }

    #[test]
    fn lenient_base64_and_truncated_ints() {
        assert_eq!(
            b64decode("eyJ\nhIjox fQ==").as_deref(),
            Some(&b"{\"a\":1}"[..])
        );
        assert_eq!(int_or_none(Some(&serde_json::json!(24.4506))), Some(24));
        assert_eq!(int_or_none(Some(&serde_json::json!("640"))), Some(640));
    }
}
