//! Upstream `ShowRoomLiveIE` and `ShowRoomVodIE` (`yt_dlp/extractor/showroomlive.py`):
//! a room's `api/room/status` and profile, its `live/streaming_url` HLS master
//! for a live stream, and an episode page's `episode-data` with its
//! `api/episode/streaming_url` for a VOD. Upcoming livestreams (whose start
//! time is rendered in the local time zone) are not ported.
use super::{
    Extractor, download_json, download_webpage, extract_m3u8_fatal_master, update_url_query,
};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct ShowRoomLive;
pub struct ShowRoomVod;

const LIVE_NAME: &str = "showroom:live";
const LIVE_KEY: &str = "ShowRoomLive";
const VOD_NAME: &str = "showroom:vod";
const VOD_KEY: &str = "ShowRoomVod";
const API: &str = "https://www.showroom-live.com/api";

static LIVE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?showroom-live\.com/r/(?P<id>[\w-]+)").unwrap()
});
static VOD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?showroom-live\.com/episode/watch\?(?:[^#]+&)?id=(?P<id>\w+)")
        .unwrap()
});
static EPISODE_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<[a-z0-9]+\b[^>]*\bid=["']episode-data["'][^>]*>"#).unwrap()
});

/// `_VALID_URL` of the live room pages.
pub fn live_suitable(url: &str) -> bool {
    LIVE_URL.is_match(url)
}

/// `_VALID_URL` of the VOD episode pages.
pub fn vod_suitable(url: &str) -> bool {
    VOD_URL.is_match(url)
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::String(text) => Some(Value::from(text.as_str())),
        Value::Number(number) => Some(Value::from(number.to_string())),
        Value::Bool(flag) => Some(Value::from(flag.to_string())),
        _ => None,
    }
}

/// `int_or_none`.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `clean_html` of a string, dropped when empty (`filter`).
fn clean_text(value: Option<&Value>) -> Option<Value> {
    let text = crate::html::clean_html(value?.as_str()?);
    (!text.is_empty()).then(|| Value::from(text))
}

/// `url_or_none`.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `require('m3u8 URL')`.
fn require_m3u8(id: &str, name: &'static str, url: Option<String>) -> Result<String> {
    url.ok_or_else(|| {
        Error::Reported(format!(
            "[{name}] {id}: Unable to extract m3u8 URL; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })
}

/// `_extract_m3u8_formats(url, video_id, 'mp4')`.
fn hls_formats(
    client: &Client,
    manifest: &str,
    name: &'static str,
    id: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let manifest = Url::parse(manifest)?;
    let mut info = extract_m3u8_fatal_master(client, &manifest, name, id, None, false)?;
    for format in &mut info.formats {
        format.ext = "mp4".into();
    }
    Ok(info.formats)
}

fn get_json(
    client: &Client,
    path: &str,
    query: &[(&str, &str)],
    id: &str,
    name: &'static str,
) -> Result<Value> {
    let url = update_url_query(&Url::parse(&format!("{API}/{path}"))?, query);
    download_json(client, &url, name, id)
}

impl Extractor for ShowRoomLive {
    fn key(&self) -> &'static str {
        LIVE_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = LIVE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let broadcaster_id = found["id"].to_owned();
        crate::screen::set_extractor(LIVE_NAME);
        crate::screen::screen(&format!(
            "[{LIVE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let room_status = get_json(
            client,
            "room/status",
            &[("room_url_key", broadcaster_id.as_str())],
            &broadcaster_id,
            LIVE_NAME,
        )?;
        let start_timestamp = int_or_none(room_status.get("started_at"));
        let is_live = room_status.get("is_live").is_some_and(super::nuxt::truthy);
        if !is_live {
            if start_timestamp.is_some() {
                return Err(Error::UnsupportedHtml5(
                    "ShowRoom upcoming livestreams are not supported natively".into(),
                ));
            }
            return Err(Error::Reported(format!(
                "[{LIVE_NAME}] {broadcaster_id}: The channel is not currently live"
            )));
        }

        let room_id = str_or_none(room_status.get("room_id"));
        let room_id_text = room_id
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let room_profile = get_json(
            client,
            "room/profile",
            &[("room_id", room_id_text.as_str())],
            &broadcaster_id,
            LIVE_NAME,
        )?;
        let room_name = ["room_name", "main_name"]
            .into_iter()
            .find_map(|key| clean_text(room_profile.get(key)));

        let streaming = get_json(
            client,
            "live/streaming_url",
            &[("room_id", room_id_text.as_str())],
            &broadcaster_id,
            LIVE_NAME,
        )?;
        let m3u8_url = streaming
            .get("streaming_url_list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("hls_all"))
            .find_map(|entry| url_or_none(entry.get("url")));
        let m3u8_url = require_m3u8(&broadcaster_id, LIVE_NAME, m3u8_url)?;
        let formats = hls_formats(client, &m3u8_url, LIVE_NAME, &broadcaster_id)?;

        let mut fields: Vec<(&str, Value)> = Vec::new();
        if let Some(name) = room_name.clone() {
            fields.push(("title", name.clone()));
            fields.push(("channel", name));
        }
        fields.push(("channel_id", Value::from(broadcaster_id.as_str())));
        fields.push(("formats", Value::Null));
        fields.push(("is_live", Value::Bool(true)));
        if let Some(timestamp) = start_timestamp {
            fields.push(("release_timestamp", Value::from(timestamp)));
        }
        // `**traverse_obj(room_profile, {...})`, after the keys above.
        let profile = [
            ("id", str_or_none(room_profile.get("live_id"))),
            (
                "channel_follower_count",
                int_or_none(room_profile.get("follower_num")).map(Value::from),
            ),
            (
                "channel_is_verified",
                room_profile
                    .get("is_official")
                    .map(|value| Value::Bool(super::nuxt::truthy(value))),
            ),
            ("description", clean_text(room_profile.get("description"))),
            (
                "genres",
                clean_text(room_profile.get("genre_name")).map(|genre| Value::from(vec![genre])),
            ),
            ("tags", {
                let tags: Vec<Value> = room_profile
                    .get("live_tags")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|tag| clean_text(tag.get("name")))
                    .collect();
                (!tags.is_empty()).then(|| Value::from(tags))
            }),
            (
                "thumbnail",
                ["image_square", "image"]
                    .into_iter()
                    .find_map(|key| url_or_none(room_profile.get(key)))
                    .map(Value::from),
            ),
            (
                "view_count",
                int_or_none(room_profile.get("view_num")).map(Value::from),
            ),
        ];
        for (key, value) in profile {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        Ok(Some(ExtractResult::Video(super::dict_info(
            url,
            (LIVE_NAME, LIVE_KEY),
            fields,
            formats,
        ))))
    }
}

impl Extractor for ShowRoomVod {
    fn key(&self) -> &'static str {
        VOD_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VOD_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let episode_id = found["id"].to_owned();
        crate::screen::set_extractor(VOD_NAME);
        crate::screen::screen(&format!(
            "[{VOD_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, VOD_NAME, &episode_id)?;
        // `find_element(id='episode-data')`, its `data-episode` JSON dict.
        let episode_data: Map<String, Value> = EPISODE_DATA
            .find(&webpage)
            .map(|tag| crate::html_elements::extract_attributes(tag.as_str()))
            .and_then(|attributes| {
                attributes
                    .get("data-episode")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .and_then(|json| serde_json::from_str::<Value>(&json).ok())
            .and_then(|value| match value {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .unwrap_or_default();

        let streaming = get_json(
            client,
            "episode/streaming_url",
            &[("episode_id", episode_id.as_str())],
            &episode_id,
            VOD_NAME,
        )?;
        let m3u8_url = url_or_none(streaming.pointer("/streaming_url_list/hls_all/hls_all"));
        let m3u8_url = require_m3u8(&episode_id, VOD_NAME, m3u8_url)?;
        let formats = hls_formats(client, &m3u8_url, VOD_NAME, &episode_id)?;

        let mut fields: Vec<(&str, Value)> = vec![("id", Value::from(episode_id.as_str()))];
        fields.push(("formats", Value::Null));
        if let Some(thumbnail) = url_or_none(streaming.get("thumbnail_url")) {
            fields.push(("thumbnail", Value::from(thumbnail)));
        }
        let series = episode_data.get("series");
        let episode = [
            ("title", clean_text(episode_data.get("title"))),
            ("description", clean_text(episode_data.get("description"))),
            (
                "duration",
                int_or_none(episode_data.get("video_time")).map(Value::from),
            ),
            (
                "timestamp",
                int_or_none(episode_data.get("display_started_at")).map(Value::from),
            ),
            (
                "series",
                clean_text(series.and_then(|series| series.get("name"))),
            ),
            (
                "series_id",
                str_or_none(series.and_then(|series| series.get("id"))),
            ),
        ];
        for (key, value) in episode {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        Ok(Some(ExtractResult::Video(super::dict_info(
            url,
            (VOD_NAME, VOD_KEY),
            fields,
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_match_upstream_valid_url() {
        assert!(live_suitable(
            "https://www.showroom-live.com/r/48_Yui_Oguri"
        ));
        assert!(!live_suitable(
            "https://www.showroom-live.com/episode/watch?id=214"
        ));
        assert!(vod_suitable(
            "https://www.showroom-live.com/episode/watch?id=214"
        ));
        assert!(vod_suitable(
            "https://showroom-live.com/episode/watch?foo=1&id=214"
        ));
        assert!(!vod_suitable(
            "https://www.showroom-live.com/r/48_Yui_Oguri"
        ));
    }
}
