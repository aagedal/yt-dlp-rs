//! Upstream `MGTVIE` (`yt_dlp/extractor/mgtv.py`): the player API's `atc`
//! PM2 token, the `getSource` stream list and each stream's info URL as
//! `m3u8_native` formats. Subtitles are only fetched when requested, which
//! is not supported natively yet.
use super::{
    Extractor, dict_format, dict_info, direct, download_json, download_json_note, json_ld,
};
use crate::{Error, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::hash::{BuildHasher, Hasher};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

pub struct Mgtv;

const NAME: &str = "MangoTV";
const KEY: &str = "MGTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:w(?:ww)?\.)?mgtv\.com/[bv]/(?:[^/]+/)*(?P<id>\d+)\.html").unwrap()
});
static TBR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_(\d+)_mp4/").unwrap());

/// `_RESOLUTIONS`: a stream name's (height label, `WxH`) pair.
const RESOLUTIONS: [(&str, (&str, &str)); 4] = [
    ("标清", ("480p", "854x480")),
    ("高清", ("540p", "960x540")),
    ("超清", ("720p", "1280x720")),
    ("蓝光", ("1080p", "1920x1080")),
];

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MangoTV {what} are not supported natively"))
}

/// `uuid.uuid4()` in text form; the RNG is std's per-process random state.
fn uuid_text() -> String {
    let mut hex = String::new();
    for index in 0..2u64 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(index);
        hex.push_str(&format!("{:016x}", hasher.finish()));
    }
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The `tk2` token: `urlsafe_b64encode(...)[::-1]`.
fn tk2() -> String {
    let clit = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let text = format!("did={}|pno=1030|ver=0.3.0301|clit={clit}", uuid_text());
    let mut bytes = base64::engine::general_purpose::URL_SAFE
        .encode(text)
        .into_bytes();
    bytes.reverse();
    String::from_utf8(bytes).unwrap_or_default()
}

/// A player API JSON document. A `401` (`e.cause.response`) is read for its
/// `code` and `msg`: `40005` is a geo restriction, others are expected errors.
fn player_api(client: &Client, url: &Url, id: &str) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
    let mut response = direct::request_expecting(client, url, &[], &[401])?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    let json: Value = crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {id}: Failed to parse JSON (caused by {error})"
        ))
    })?;
    if status == 401 {
        if json.get("code").and_then(Value::as_i64) == Some(40005) {
            return Err(crate::geo::restricted(
                &format!(
                    "[{NAME}] {id}: This video is not available from your location due to geo restriction"
                ),
                &[],
            ));
        }
        let message = json
            .get("msg")
            .ok_or_else(|| super::clyp::key_error(id, "msg"))?;
        let message = match message {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        return Err(Error::Reported(message));
    }
    Ok(json)
}

/// `traverse_obj(obj, key)` as a string value.
fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

/// `int_or_none(value)` for a JSON value (`None` for non-numeric text).
fn int_value(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

impl Extractor for Mgtv {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let token = tk2();
        let api_url = super::update_url_query(
            &Url::parse("https://pcweb.api.mgtv.com/player/video")?,
            &[("tk2", &token), ("video_id", &video_id), ("type", "pch5")],
        );
        let api_data = player_api(client, &api_url, &video_id)?
            .get("data")
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&video_id, "data"))?;
        let pm2 = api_data
            .get("atc")
            .and_then(|atc| atc.get("pm2"))
            .and_then(Value::as_str)
            .ok_or_else(|| super::clyp::key_error(&video_id, "pm2"))?
            .to_owned();

        let source_url = super::update_url_query(
            &Url::parse("https://tinker.glb.mgtv.com/player/getSource")?,
            &[
                ("_support", "10000000"),
                ("tk2", &token),
                ("pm2", &pm2),
                ("video_id", &video_id),
                ("type", "pch5"),
                ("auth_mode", "1"),
                ("cxid", ""),
                ("supportMse", "1"),
                ("src", "intelmgtv"),
                // The `abroad` cookie is not read; the site's default applies.
                ("abroad", "10"),
                ("allowedRC", "1"),
            ],
        );
        let stream_data = download_json(client, &source_url, NAME, &video_id)?
            .get("data")
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&video_id, "data"))?;
        let stream_domain = stream_data
            .get("stream_domain")
            .and_then(Value::as_array)
            .and_then(|domains| {
                domains
                    .iter()
                    .filter_map(|domain| domain.as_str())
                    .find_map(json_ld::url_or_none)
            });

        let mut formats = Vec::new();
        let streams = stream_data
            .get("stream")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|stream| stream.get("url").is_some_and(super::nuxt::truthy));
        for (idx, stream) in streams.enumerate() {
            let stream_name = ["name", "standardName", "barName"]
                .iter()
                .find_map(|key| text(stream.get(*key)))
                .map(str::to_owned);
            let scale_16_9 = stream.get("scale").and_then(Value::as_str) == Some("16:9");
            let resolution = stream_name.as_deref().and_then(|name| {
                RESOLUTIONS
                    .iter()
                    .find(|(label, _)| *label == name)
                    .map(|(_, (height, size))| if scale_16_9 { *size } else { *height })
            });
            let stream_url = text(stream.get("url")).unwrap_or_default();
            let info_url = match &stream_domain {
                Some(domain) => Url::parse(domain)
                    .and_then(|base| base.join(stream_url))
                    .ok(),
                None => Url::parse(stream_url).ok(),
            };
            // `_download_json(..., fatal=False)`: a failed request skips the stream.
            let note = format!(
                "Downloading video info for format {}",
                resolution.or(stream_name.as_deref()).unwrap_or("None")
            );
            let Some(info) = info_url.and_then(|info_url| {
                download_json_note(client, &info_url, NAME, &video_id, &note).ok()
            }) else {
                continue;
            };
            let Some(format_url) = info
                .get("info")
                .and_then(Value::as_str)
                .and_then(json_ld::url_or_none)
            else {
                continue;
            };
            // `int_or_none(stream.get('filebitrate') or _search_regex(...))`.
            let bitrate = match stream.get("filebitrate") {
                Some(value) if super::nuxt::truthy(value) => Some(value.clone()),
                _ => TBR
                    .captures(&format_url)
                    .map(|found| Value::from(found[1].to_owned())),
            };
            let tbr = bitrate.as_ref().and_then(int_value);
            let (width, height) = resolution
                .map(super::html5::parse_resolution)
                .unwrap_or((None, None));
            let mut fields = vec![
                (
                    "format_id",
                    Value::from(tbr.map_or_else(|| idx.to_string(), |tbr| tbr.to_string())),
                ),
                ("url", Value::from(format_url)),
                ("ext", "mp4".into()),
                ("tbr", tbr.map_or(Value::Null, Value::from)),
                (
                    "vcodec",
                    stream.get("videoFormat").cloned().unwrap_or(Value::Null),
                ),
                (
                    "acodec",
                    stream.get("audioFormat").cloned().unwrap_or(Value::Null),
                ),
            ];
            if let Some(width) = width {
                fields.push(("width", width.into()));
            }
            if let Some(height) = height {
                fields.push(("height", height.into()));
            }
            fields.extend([
                ("protocol", "m3u8_native".into()),
                (
                    "http_headers",
                    serde_json::json!({ "Referer": url.as_str() }),
                ),
                ("format_note", stream_name.map_or(Value::Null, Value::from)),
            ]);
            let mut format = dict_format(fields, NAME, &video_id)?;
            // `int_or_none` tbr values print as integers.
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }

        // `extract_subtitles` only fetches tracks when they were requested.
        if super::subtitles_requested() {
            return Err(unsupported("subtitles"));
        }

        let info = api_data.get("info").and_then(Value::as_object);
        let info_text = |key: &str| info.and_then(|info| info.get(key));
        let mut fields: Vec<(&str, Value)> =
            vec![("id", video_id.clone().into()), ("formats", Value::Null)];
        if let Some(title) = text(info_text("title")) {
            fields.push(("title", title.trim().into()));
        }
        if let Some(description) = info_text("desc").and_then(Value::as_str) {
            fields.push(("description", description.into()));
        }
        let duration = super::streamable::int_or_none(info_text("duration"));
        if !duration.is_null() {
            fields.push(("duration", duration));
        }
        if let Some(thumbnail) = text(info_text("thumb")).and_then(json_ld::url_or_none) {
            fields.push(("thumbnail", thumbnail.into()));
        }
        fields.push(("subtitles", Value::Null));
        Ok(Some(dict_info(url, (NAME, KEY), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.mgtv.com/b/340/3116640.html"));
        assert!(suitable(
            "https://w.mgtv.com/b/427837/15588271.html?fpa=33318"
        ));
        assert!(!suitable("https://www.mgtv.com/"));
    }

    #[test]
    fn tk2_is_reversed_base64() {
        let mut bytes = tk2().into_bytes();
        bytes.reverse();
        let decoded = base64::engine::general_purpose::URL_SAFE
            .decode(String::from_utf8(bytes).unwrap())
            .unwrap();
        let text = String::from_utf8(decoded).unwrap();
        assert!(text.starts_with("did="));
        assert!(text.contains("|pno=1030|ver=0.3.0301|clit="));
    }
}
