//! Upstream `PartiVideoIE` and `PartiLivestreamIE`
//! (`yt_dlp/extractor/parti.py`): Parti recordings from the profile API's
//! `livestream_recording` HLS playlist, and live channels from the creator's
//! user ID and channel info (`UserNotLive` when offline).
use super::streamable::int_or_none;
use super::{Extractor, dict_info};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct PartiVideo;
pub struct PartiLivestream;

const VIDEO: &str = "parti:video";
const VIDEO_KEY: &str = "PartiVideo";
const LIVE: &str = "parti:livestream";
const LIVE_KEY: &str = "PartiLivestream";

static VIDEO_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?parti\.com/video/(?P<id>\d+)").unwrap());
/// `parti\.com/(?!video/)(?P<id>[\w/-]+)`: the lookahead is checked in
/// [`live_id`].
static LIVE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?parti\.com/(?P<id>[\w/-]+)").unwrap());

/// `PartiVideoIE._match_id`.
pub fn video_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

fn live_id(url: &str) -> Option<String> {
    LIVE_URL
        .captures(url)
        .map(|found| found["id"].to_owned())
        .filter(|id| !id.starts_with("video/"))
}

/// `PartiLivestreamIE._match_id`.
pub fn livestream_suitable(url: &str) -> bool {
    live_id(url).is_some()
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Parti {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `PartiBaseIE._call_api`.
fn call_api(
    client: &Client,
    name: &'static str,
    path: &str,
    id: &str,
    note: &str,
) -> Result<Value> {
    let api = Url::parse(&format!(
        "https://prod-api.parti.com/parti_v2/profile/{path}"
    ))
    .map_err(|_| unsupported("API paths that do not form URLs"))?;
    super::download_json_note_headers(
        client,
        &api,
        name,
        id,
        note,
        &[
            ("Origin", "https://parti.com"),
            ("Referer", "https://parti.com/"),
        ],
    )
}

/// `data[key]` of a JSON object response.
fn get<'a>(data: &'a Value, key: &str, temp_id: &str) -> Result<&'a Value> {
    match data {
        Value::Object(map) => map.get(key).ok_or_else(|| key_error(temp_id, key)),
        _ => Err(unsupported("API responses that are not objects")),
    }
}

/// `traverse_obj(data, (*path, {str}))`.
fn string(data: &Value, path: &str) -> Value {
    match data.pointer(path) {
        Some(Value::String(text)) => text.clone().into(),
        _ => Value::Null,
    }
}

/// `traverse_obj(data, (*path, {url_or_none}))`.
fn url_value(data: &Value, path: &str) -> Value {
    data.pointer(path)
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map_or(Value::Null, Value::from)
}

/// The traversed fields that are not `None`, in template order.
fn present(fields: Vec<(&'static str, Value)>) -> Vec<(&'static str, Value)> {
    fields
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .collect()
}

impl Extractor for PartiVideo {
    fn key(&self) -> &'static str {
        VIDEO_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(VIDEO, url);
        let data = call_api(
            client,
            VIDEO,
            &format!("get_livestream_channel_info/recent/{video_id}"),
            &video_id,
            "Downloading JSON metadata",
        )?;
        let recording = match get(&data, "livestream_recording", &video_id)? {
            Value::String(recording) => recording,
            _ => return Err(unsupported("recordings without a string URL")),
        };
        let manifest = super::videopress::urljoin(Some("https://media.parti.com/"), recording)
            .and_then(|manifest| Url::parse(&manifest).ok())
            .ok_or_else(|| unsupported("recordings without a URL"))?;
        let stream =
            super::extract_m3u8_fatal_master(client, &manifest, VIDEO, &video_id, None, false)?;
        let mut formats = stream.formats;
        for format in &mut formats {
            format.ext = "mp4".into();
        }
        // `traverse_obj(data, ('category_name', {str}, filter, all))`.
        let categories = match data.get("category_name") {
            Some(Value::String(name)) if !name.is_empty() => Value::from(vec![name.clone()]),
            _ => Value::Array(Vec::new()),
        };
        let mut fields = vec![("id", Value::from(video_id)), ("formats", Value::Null)];
        fields.extend(present(vec![
            ("title", string(&data, "/event_title")),
            ("channel", string(&data, "/user_name")),
            ("thumbnail", url_value(&data, "/event_file")),
            ("categories", categories),
            ("timestamp", int_or_none(data.get("event_start_ts"))),
        ]));
        Ok(Some(dict_info(url, (VIDEO, VIDEO_KEY), fields, formats)))
    }
}

impl Extractor for PartiLivestream {
    fn key(&self) -> &'static str {
        LIVE_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(creator_slug) = live_id(url.as_str()) else {
            return Ok(None);
        };
        start(LIVE, url);
        let encoded = creator_slug.replace('/', "%23");
        let user = call_api(
            client,
            LIVE,
            &format!("user_id_from_name/{encoded}"),
            &creator_slug,
            "Fetching user ID",
        )?;
        let creator_id = match get(&user, "user_id", &creator_slug)? {
            Value::String(id) => id.clone(),
            Value::Number(id) if id.is_i64() || id.is_u64() => id.to_string(),
            _ => return Err(unsupported("user IDs that are not strings or integers")),
        };
        let feed = call_api(
            client,
            LIVE,
            &format!("get_livestream_channel_info/{creator_id}"),
            &creator_id,
            "Fetching user profile feed",
        )?;
        let data = get(&feed, "channel_info", &creator_slug)?;
        if data.pointer("/channel/is_live") != Some(&Value::Bool(true)) {
            return Err(Error::Reported(format!(
                "[{LIVE}] {creator_id}: The channel is not currently live"
            )));
        }
        let channel = get(data, "channel", &creator_slug)?;
        let playback = match get(channel, "playback_url", &creator_slug)? {
            Value::String(playback) => playback,
            _ => return Err(unsupported("playback URLs that are not strings")),
        };
        let manifest = Url::parse(playback).map_err(|_| unsupported("relative playback URLs"))?;
        let stream =
            super::extract_m3u8_fatal_live(client, &manifest, LIVE, &creator_slug, None, false)?;
        let timestamp = data
            .pointer("/stream/start_time")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);
        let mut fields = vec![
            ("id", Value::from(creator_slug)),
            ("formats", Value::Null),
            ("is_live", Value::Bool(true)),
        ];
        fields.extend(present(vec![
            ("title", string(data, "/livestream_event_info/event_name")),
            (
                "description",
                string(data, "/livestream_event_info/event_description"),
            ),
            (
                "thumbnail",
                url_value(data, "/livestream_event_info/livestream_preview_file"),
            ),
            ("timestamp", timestamp),
            (
                "view_count",
                int_or_none(data.pointer("/stream/viewer_count")),
            ),
        ]));
        Ok(Some(dict_info(
            url,
            (LIVE, LIVE_KEY),
            fields,
            stream.formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(video_suitable("https://parti.com/video/66284"));
        assert!(!livestream_suitable("https://parti.com/video/66284"));
        assert!(livestream_suitable("https://parti.com/247CryptoTracker"));
        assert_eq!(
            live_id("https://www.parti.com/creator/parti/Capt_Robs").as_deref(),
            Some("creator/parti/Capt_Robs")
        );
    }
}
