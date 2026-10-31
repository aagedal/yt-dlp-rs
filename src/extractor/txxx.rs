//! Upstream `PornTopIE` and `TxxxIE` (`yt_dlp/extractor/txxx.py`): a PornTop
//! video page's `schemaJson` JSON-LD and the `initPlayer` call's base64 JSON
//! of its `video_url` MP4 files, and the Txxx network's video API (the file
//! list and the video's metadata JSON) with the same base64 MP4 URLs.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct PornTop;
pub struct Txxx;

const NAME: &str = "PornTop";
const TXXX_NAME: &str = "Txxx";
/// `TxxxIE._DOMAINS` (regex alternation).
const TXXX_DOMAINS: &str = r"hclips\.com|hdzog\.com|hdzog\.tube|hotmovs\.com|hotmovs\.tube|inporn\.com|privatehomeclips\.com|tubepornclassic\.com|txxx\.com|txxx\.tube|upornia\.com|upornia\.tube|vjav\.com|vjav\.tube|vxxx\.com|voyeurhit\.com|voyeurhit\.tube";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(concat!(
        r"^https?://(?P<host>(?:www\.)?porntop\.com)/video/(?P<id>\d+)",
        r"(?:/(?P<display_id>[^/?]+))?",
    ))
    .unwrap()
});
static TXXX_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^https?://(?:www\.)?(?P<host>{TXXX_DOMAINS})/(?:videos?[/-]|embed/)(?P<id>\d+)(?:/(?P<display_id>[^/?#]+))?"
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
    name: &'static str,
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
                "[{name}] {video_id}: Unable to extract video_url; {}",
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
            name,
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
        let formats = get_formats(NAME, &host, &video_file, &video_id)?;

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

/// `int_or_none(value)` for a JSON field.
fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

/// `TxxxIE._call_api(url, video_id, fatal, ...)` with its `error` check.
fn call_api(
    client: &Client,
    url: &Url,
    video_id: &str,
    note: &str,
    headers: &[(&str, &str)],
) -> Result<Value> {
    let content =
        super::download_json_note_headers(client, url, TXXX_NAME, video_id, note, headers)?;
    if let Some(error) = content.get("error").filter(|error| !is_falsy(error)) {
        let text = match error {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        return Err(crate::Error::Reported(format!("Txxx said: {text}")));
    }
    Ok(content)
}

/// Python's falsiness for a JSON value.
fn is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
    }
}

/// `txxx_suitable`: `TxxxIE.suitable`.
pub fn txxx_suitable(url: &str) -> bool {
    TXXX_URL.is_match(url)
}

impl Extractor for Txxx {
    fn key(&self) -> &'static str {
        TXXX_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = TXXX_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let host = found["host"].to_owned();
        let display_id = found
            .name("display_id")
            .map_or(Value::Null, |display| Value::from(display.as_str()));
        crate::screen::set_extractor(TXXX_NAME);
        crate::screen::screen(&format!(
            "[{TXXX_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let referer = url.as_str();
        let headers = [("Referer", referer), ("X-Requested-With", "XMLHttpRequest")];
        let file_url = Url::parse(&format!(
            "https://{host}/api/videofile.php?video_id={video_id}&lifetime=8640000"
        ))?;
        let video_file = call_api(
            client,
            &file_url,
            &video_id,
            "Downloading video file info",
            &headers,
        )?;
        // `slug = f'{int(1E6 * (id // 1E6))}/{1000 * (id // 1000)}'`.
        // The URL matches digits only.
        let id: i64 = video_id.parse().unwrap_or_default();
        let slug = format!("{}/{}", 1_000_000 * (id / 1_000_000), 1000 * (id / 1000));
        let info_url = Url::parse(&format!(
            "https://{host}/api/json/video/86400/{slug}/{video_id}.json"
        ))?;
        let video_info = call_api(
            client,
            &info_url,
            &video_id,
            "Downloading video info",
            &headers,
        )?;
        let video = video_info.get("video");
        let field = |path: &str| video.and_then(|video| video.pointer(path));
        let duration = match field("/duration")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
        {
            Some(number) => json!(number),
            None => Value::Null,
        };
        let thumbnail = field("/thumbsrc")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map_or(Value::Null, Value::from);
        let files: Vec<Value> = video_file.as_array().cloned().unwrap_or_default();
        let formats = get_formats(TXXX_NAME, &host, &files, &video_id)?;
        Ok(Some(dict_info(
            url,
            (TXXX_NAME, TXXX_NAME),
            vec![
                ("id", video_id.as_str().into()),
                ("display_id", display_id),
                ("title", field("/title").cloned().unwrap_or(Value::Null)),
                (
                    "uploader",
                    field("/user/username").cloned().unwrap_or(Value::Null),
                ),
                ("duration", duration),
                ("view_count", int_or_none(field("/statistics/viewed"))),
                ("like_count", int_or_none(field("/statistics/likes"))),
                ("dislike_count", int_or_none(field("/statistics/dislikes"))),
                ("age_limit", 18.into()),
                ("thumbnail", thumbnail),
                ("formats", Value::Null),
            ],
            formats,
        )))
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

    #[test]
    fn txxx_urls_follow_upstream_valid_url() {
        assert!(txxx_suitable(
            "https://txxx.com/videos/16574965/digital-desire-malena-morgan/"
        ));
        assert!(txxx_suitable("https://vxxx.com/video-68925/"));
        assert!(!txxx_suitable("https://porntop.com/video/1234/x"));
    }
}
