//! Upstream `VidioIE` and `VidioPremierIE` (`yt_dlp/extractor/vidio.py`): a
//! video's API record and its HLS manifest (through the anonymous API key
//! from `www.vidio.com/auth`), and a premier series' playlist entries as
//! `VidioPremier` URL results. Premier-exclusive videos, live streams and
//! login are not supported natively.
use super::clyp::key_error;
use super::{Extractor, dict_info, download_json_headers, extract_m3u8_fatal_master};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Vidio;
pub struct VidioPremier;

const NAME: &str = "Vidio";
const PREMIER: &str = "VidioPremier";
const AUTH_URL: &str = "https://www.vidio.com/auth";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https?://(?:www\.)?vidio\.com/(?:watch|embed)/(?P<id>\d+)-(?P<display_id>[^/?#&]+)",
    )
    .unwrap()
});
static PREMIER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?vidio\.com/premier/(?P<id>\d+)/(?P<display_id>[^/?#&]+)")
        .unwrap()
});

/// `VidioIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `VidioPremierIE.suitable`.
pub fn premier_suitable(url: &str) -> bool {
    PREMIER_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Vidio {what} are not supported natively"))
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `_initialize_pre_login`: the anonymous API key every API call sends.
fn api_key(client: &Client) -> Result<String> {
    let auth = super::post_json_note_headers(
        client,
        &Url::parse(AUTH_URL)?,
        NAME,
        "",
        "Downloading JSON metadata",
        &[],
        String::new(),
    )?;
    auth.get("api_key")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| key_error("", "api_key"))
}

/// `_call_api(url, video_id)`.
fn call_api(client: &Client, key: &str, url: &str, id: &str) -> Result<Value> {
    download_json_headers(
        client,
        &Url::parse(url)?,
        NAME,
        id,
        &[
            ("Content-Type", "application/vnd.api+json"),
            ("X-API-KEY", key),
        ],
    )
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `strip_or_none`.
fn strip_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => Value::from(text.trim()),
        _ => Value::Null,
    }
}

/// `try_get(data, lambda y: y[key + 's'][0], dict) or {}`.
fn first_of<'a>(data: &'a Value, key: &str) -> &'a Value {
    static EMPTY: Value = Value::Null;
    data.get(format!("{key}s"))
        .and_then(|list| list.get(0))
        .filter(|item| item.is_object())
        .unwrap_or(&EMPTY)
}

/// `url_result(target, ie=ie_key)` as a playlist entry.
fn url_entry(target: &str, ie_key: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ie_key.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": ie_key,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for Vidio {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        let key = api_key(client)?;
        start(NAME, url);

        let data = call_api(
            client,
            &key,
            &format!("https://api.vidio.com/videos/{video_id}"),
            &display_id,
        )?;
        let video = data
            .get("videos")
            .and_then(|videos| videos.get(0))
            .ok_or_else(|| key_error(&display_id, "videos"))?;
        let title = video
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&display_id, "title"))?
            .trim()
            .to_owned();
        if video.get("is_premium").is_some_and(truthy) {
            return Err(unsupported(
                "premium videos (they need the account's interactions stream)",
            ));
        }

        let hls_url = data
            .get("clips")
            .and_then(|clips| clips.get(0))
            .and_then(|clip| clip.get("hls_url"))
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&display_id, "hls_url"))?;
        let manifest = Url::parse(hls_url)?;
        let stream = extract_m3u8_fatal_master(client, &manifest, NAME, &display_id, None, true)?;

        let channel = first_of(&data, "channel");
        let user = first_of(&data, "user");
        let username = user.get("username").and_then(Value::as_str);
        let count = |key: &str| super::streamable::int_or_none(video.get(format!("total_{key}")));
        let fields = vec![
            ("id", Value::from(video_id.as_str())),
            ("display_id", Value::from(display_id.as_str())),
            ("title", Value::from(title)),
            ("description", strip_or_none(video.get("description"))),
            (
                "thumbnail",
                video
                    .get("image_url_medium")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "duration",
                super::streamable::int_or_none(video.get("duration")),
            ),
            ("like_count", count("likes")),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("uploader", user.get("name").cloned().unwrap_or(Value::Null)),
            (
                "timestamp",
                video
                    .get("created_at")
                    .and_then(Value::as_str)
                    .and_then(crate::dates::parse_iso8601)
                    .map_or(Value::Null, Value::from),
            ),
            ("uploader_id", username.map_or(Value::Null, Value::from)),
            (
                "uploader_url",
                username.map_or(Value::Null, |name| {
                    Value::from(format!("https://www.vidio.com/@{name}"))
                }),
            ),
            (
                "channel",
                channel.get("name").cloned().unwrap_or(Value::Null),
            ),
            (
                "channel_id",
                str_or_none(channel.get("id")).map_or(Value::Null, Value::from),
            ),
            ("view_count", count("view_count")),
            ("dislike_count", count("dislikes")),
            ("comment_count", count("comments")),
            (
                "tags",
                video.get("tag_list").cloned().unwrap_or(Value::Null),
            ),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, stream.formats);
        info.subtitles = stream.subtitles;
        Ok(Some(info))
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Null => false,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

impl Extractor for VidioPremier {
    fn key(&self) -> &'static str {
        PREMIER
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let (bare, smuggled) = super::brightcove::unsmuggle(url.as_str());
        let Some(found) = PREMIER_URL.captures(&bare) else {
            return Ok(None);
        };
        if smuggled.contains_key("url") {
            // The entries' follow-up API pages (`_playlist_entries`).
            return Err(unsupported("premier playlist pages"));
        }
        let playlist_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        let page = Url::parse(&bare)?;
        start(PREMIER, &page);
        let key = api_key(client)?;

        let playlist = call_api(
            client,
            &key,
            &format!("https://api.vidio.com/content_profiles/{playlist_id}/playlists"),
            &display_id,
        )?;
        let mut entries = Vec::new();
        for item in playlist
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let related = item
                .get("relationships")
                .and_then(|relationships| relationships.get("videos"))
                .and_then(|videos| videos.get("links"))
                .and_then(|links| links.get("related"))
                .cloned()
                .unwrap_or(Value::Null);
            let name = item
                .get("attributes")
                .and_then(|attributes| attributes.get("name"))
                .cloned()
                .unwrap_or(Value::Null);
            let data = json!({
                "url": related,
                "id": item.get("id").cloned().unwrap_or(Value::Null),
                "title": name,
            });
            let target = super::brightcove::smuggle(page.as_str(), &data);
            entries.push(url_entry(&target, PREMIER)?);
        }

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: String::new(),
            webpage_url: page,
            extractor: PREMIER.into(),
            extractor_key: PREMIER.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.vidio.com/watch/165683-dj_ambred-booyah-live-2015"
        ));
        assert!(suitable(
            "https://www.vidio.com/embed/7115874-fakta-temuan-suspek-cacar-monyet-di-jawa-tengah"
        ));
        assert!(!suitable("https://www.vidio.com/live/204-sctv"));
        assert!(premier_suitable(
            "https://www.vidio.com/premier/2885/badai-pasti-berlalu"
        ));
        assert!(!premier_suitable("https://www.vidio.com/premier/2885/"));
    }
}
