//! Upstream `PornTopIE` (`yt_dlp/extractor/txxx.py`): a PornTop video page's
//! `schemaJson` JSON-LD, and the `initPlayer` call's base64 JSON of its
//! `video_url` MP4 files.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct PornTop;

const NAME: &str = "PornTop";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(concat!(
        r"^https?://(?P<host>(?:www\.)?porntop\.com)/video/(?P<id>\d+)",
        r"(?:/(?P<display_id>[^/?]+))?",
    ))
    .unwrap()
});
static SCHEMA_JSON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bschemaJson\s*=\s*(\{[^<]+?VideoObject[^<]+\});").unwrap());
static INIT_PLAYER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.initPlayer\(.*}}},\s*'(?P<json_b64c>[^']+)'").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `decode_base64`: PornTop's Cyrillic look-alike letters, `,` and `.` as
/// `/` and `+` and `~` as `=`, then Python's lenient base64 decode.
fn decode_base64(text: &str) -> Option<String> {
    let mut cleaned = String::with_capacity(text.len());
    for character in text.chars() {
        let mapped = match character {
            '\u{0405}' => 'S',
            '\u{0406}' => 'I',
            '\u{0408}' => 'J',
            '\u{0410}' => 'A',
            '\u{0412}' => 'B',
            '\u{0415}' => 'E',
            '\u{041a}' => 'K',
            '\u{041c}' => 'M',
            '\u{041d}' => 'H',
            '\u{041e}' => 'O',
            '\u{0420}' => 'P',
            '\u{0421}' => 'C',
            '\u{0425}' => 'X',
            ',' => '/',
            '.' => '+',
            '~' => '=',
            other => other,
        };
        // Python's b64decode drops characters outside the alphabet.
        if mapped.is_ascii_alphanumeric() || matches!(mapped, '+' | '/' | '=') {
            cleaned.push(mapped);
        }
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// `variadic(video['format'])[0].lstrip('_')`, or `None` when absent.
fn format_id(video: &Value) -> Option<String> {
    let first = match video.get("format")? {
        Value::Array(items) => items.first()?.clone(),
        other => other.clone(),
    };
    first
        .as_str()
        .map(|text| text.trim_start_matches('_').to_owned())
}

/// `get_formats(host, video_file)`: one MP4 per non-empty `video_url`, with
/// the list index as its `quality`.
fn get_formats(
    host: &str,
    video_file: &[Value],
    video_id: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let base = Url::parse(&format!("https://{host}"))?;
    let mut formats = Vec::new();
    for (index, video) in video_file.iter().enumerate() {
        let Some(encoded) = video
            .get("video_url")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let decoded = decode_base64(encoded).ok_or_else(|| {
            crate::Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract video_url; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let url = base.join(&decoded)?;
        formats.push(dict_format(
            vec![
                ("url", url.as_str().into()),
                (
                    "format_id",
                    format_id(video).map_or(Value::Null, Value::from),
                ),
                ("quality", (index as i64).into()),
            ],
            NAME,
            video_id,
        )?);
    }
    Ok(formats)
}

impl Extractor for PornTop {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = found["id"].to_owned();
        let display_id = found.name("display_id").map(|id| id.as_str().to_owned());
        let host = found["host"].to_owned();
        let webpage = download_webpage(client, url, NAME, &video_id)?;

        // `_search_json(r'\bschemaJson\s*=', ..., transform_source=js_to_json,
        // contains_pattern=...)` followed by `_json_ld(..., fatal=True)`.
        let json_ld_value = SCHEMA_JSON
            .captures(&webpage)
            .and_then(|found| {
                let code = crate::js_to_json::js_to_json(&found[1]);
                serde_json::from_str::<Value>(&code).ok()
            })
            .ok_or_else(|| {
                crate::Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract JSON-LD; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let json_ld = super::json_ld::json_ld(&json_ld_value);

        let encoded = INIT_PLAYER
            .captures(&webpage)
            .map(|found| found["json_b64c"].to_owned())
            .ok_or_else(|| {
                crate::Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract json_urls; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let video_file: Value = decode_base64(&encoded)
            .and_then(|text| serde_json::from_str(&text).ok())
            .ok_or_else(|| {
                crate::Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to parse json_urls; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let video_file = video_file.as_array().cloned().unwrap_or_default();
        let formats = get_formats(&host, &video_file, &video_id)?;

        // `merge_dicts({'id': ..., 'display_id': ..., 'age_limit': 18,
        // 'formats': ...}, json_ld)`: the first dict's keys win.
        let mut fields: Vec<(String, Value)> = vec![("id".into(), video_id.as_str().into())];
        if let Some(display_id) = display_id {
            fields.push(("display_id".into(), display_id.into()));
        }
        fields.push(("age_limit".into(), 18.into()));
        fields.push(("formats".into(), Value::Null));
        for (key, value) in json_ld {
            if !fields.iter().any(|(known, _)| *known == key) {
                fields.push((key, value));
            }
        }
        let fields: Vec<(&str, Value)> = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://porntop.com/video/101569/triple-threat-with-lia-lor-malena-morgan-and-dani-daniels/"
        ));
        assert!(suitable("https://www.porntop.com/video/101569"));
        assert!(!suitable("https://porntop.com/category/101569"));
    }

    #[test]
    fn base64_uses_porntop_alphabet() {
        // "ab" in base64 is "YWI=", written with the site's `,` `.` `~`.
        assert_eq!(decode_base64("YWI~").as_deref(), Some("ab"));
    }
}
