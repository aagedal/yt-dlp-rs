//! Upstream `NFLArticleIE` (`yt_dlp/extractor/nfl.py`) and the
//! `NFLBaseIE` video-config handling it shares: a team or nfl.com article's
//! `video-config-*` JSON scripts, each `playlist[0]` resolved through the NFL
//! play API (an anonymous device token, then the asset's HLS `accessUrl`).
//! Direct `m3u8` items are followed as well. NFLIE (video pages), the NFL+
//! extractors and the account-login path (`_get_account_info`) are not ported.
use super::clyp::key_error;
use super::{Extractor, dict_info, download_webpage, post_json_note_headers};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::hash::{BuildHasher, Hasher};
use std::sync::LazyLock;
use url::Url;

pub struct NflArticle;

const NAME: &str = "nfl.com:article";
const KEY: &str = "NFLArticle";
const CLIENT_KEY: &str = "4cFUW6DmwJpzT9L7LrG3qRAcABG5s04g";
const CLIENT_SECRET: &str = "CZuvCL49d9OwfGsR";
const TOKEN_URL: &str = "https://api.nfl.com/identity/v3/token";
const ASSET_URL: &str = "https://api.nfl.com/play/v1/asset";

/// `NFLBaseIE._VALID_URL_BASE`.
const VALID_URL_BASE: &str = r"https?://(?:www\.)?(?:(?:nfl|buffalobills|miamidolphins|patriots|newyorkjets|baltimoreravens|bengals|clevelandbrowns|steelers|houstontexans|colts|jaguars|(?:titansonline|tennesseetitans)|denverbroncos|(?:kc)?chiefs|raiders|chargers|dallascowboys|giants|philadelphiaeagles|(?:redskins|washingtonfootball)|chicagobears|detroitlions|packers|vikings|atlantafalcons|panthers|neworleanssaints|buccaneers|azcardinals|(?:stlouis|the)rams|49ers|seahawks)\.com|.+?\.clubs\.nfl\.com)/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(r"^{VALID_URL_BASE}news/(?P<id>[^/#?&]+)")).unwrap()
});
static VIDEO_CONFIG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<script[^>]+id="[^"]*video-config-[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}[^"]*"[^>]*>\s*(\{.+\});?\s*</script>"#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("NFL {what} are not supported natively"))
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

/// A random UUID-shaped device ID (upstream draws a fresh `uuid4()` per run).
fn device_id() -> String {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(std::process::id().into());
    let high = hasher.finish();
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(high);
    let low = hasher.finish();
    let hex = format!("{high:016x}{low:016x}");
    format!(
        "{}-{}-4{}-8{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

/// `_get_auth_token`: an anonymous identity token, fetched once per run.
fn auth_token(client: &Client, cache: &mut Option<String>) -> Result<String> {
    if let Some(token) = cache {
        return Ok(token.clone());
    }
    let device_info = json!({
        "model": "desktop",
        "version": "Chrome",
        "osName": "Windows",
        "osVersion": "10.0",
    });
    let body = json!({
        "clientKey": CLIENT_KEY,
        "clientSecret": CLIENT_SECRET,
        "deviceId": device_id(),
        "deviceInfo": base64::engine::general_purpose::STANDARD
            .encode(device_info.to_string()),
        "networkType": "other",
        "peacockUUID": "undefined",
    });
    let token = post_json_note_headers(
        client,
        &Url::parse(TOKEN_URL)?,
        NAME,
        "",
        "Downloading access token",
        &[("Content-Type", "application/json")],
        body.to_string(),
    )?;
    let access = token
        .get("accessToken")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error("", "accessToken"))?
        .to_owned();
    *cache = Some(access.clone());
    Ok(access)
}

/// `_extract_video(mcp_id, is_live)`.
fn extract_video(
    client: &Client,
    cache: &mut Option<String>,
    mcp_id: &str,
    is_live: bool,
    page: &Url,
) -> Result<VideoInfo> {
    let token = auth_token(client, cache)?;
    let bearer = format!("Bearer {token}");
    let asset = post_json_note_headers(
        client,
        &Url::parse(&format!("{ASSET_URL}/{mcp_id}"))?,
        NAME,
        mcp_id,
        "Downloading JSON metadata",
        &[
            ("Authorization", bearer.as_str()),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
        ],
        json!({"init": true, "live": is_live}).to_string(),
    )?;
    let access_url = asset
        .get("accessUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(mcp_id, "accessUrl"))?;
    let manifest = Url::parse(access_url)?;
    let stream =
        super::extract_m3u8_fatal_master(client, &manifest, NAME, mcp_id, Some("hls"), true)?;
    let mut formats = stream.formats;
    for format in &mut formats {
        if format.ext.is_empty() || format.ext == "unknown_video" {
            format.ext = "mp4".into();
        }
    }

    let metadata = asset.get("metadata");
    let event = metadata.and_then(|metadata| metadata.get("event"));
    let text = |key: &str| {
        event
            .and_then(|event| event.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let title = text("def_title").or_else(|| {
        event
            .and_then(|event| event.get("friendlyName"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    let description = text("def_description");
    let duration = super::streamable::int_or_none(event.and_then(|event| event.get("duration")));
    let thumbnails: Vec<Value> = metadata
        .and_then(|metadata| metadata.get("thumbnails"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("url")?.as_str())
                .filter_map(super::json_ld::url_or_none)
                .map(|url| json!({ "url": url }))
                .collect()
        })
        .unwrap_or_default();

    let mut fields: Vec<(&str, Value)> = vec![
        ("id", mcp_id.into()),
        ("formats", Value::Null),
        ("subtitles", Value::Null),
        ("is_live", is_live.into()),
        ("_old_archive_ids", json!([format!("anvato {mcp_id}")])),
    ];
    if let Some(title) = title {
        fields.push(("title", title.into()));
    }
    if let Some(description) = description {
        fields.push(("description", description.into()));
    }
    if !duration.is_null() {
        fields.push(("duration", duration));
    }
    fields.push(("thumbnails", Value::Array(thumbnails)));
    let mut info = dict_info(page, (NAME, KEY), fields, formats);
    info.subtitles = stream.subtitles;
    Ok(info)
}

/// `_parse_video_config(video_config, display_id)`.
fn parse_video_config(
    client: &Client,
    cache: &mut Option<String>,
    video_config: &str,
    display_id: &str,
    page: &Url,
) -> Result<VideoInfo> {
    let config: Value = serde_json::from_str(video_config).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {display_id}: Failed to parse JSON ({error})"
        ))
    })?;
    let is_live = config.get("live").is_some_and(truthy);
    let item = config
        .get("playlist")
        .ok_or_else(|| key_error(display_id, "playlist"))?
        .get(0)
        .ok_or_else(|| unsupported("video configs without a playlist item"))?;
    if let Some(mcp_id) = item.get("mcpID").filter(|value| truthy(value)) {
        let mcp_id = match mcp_id {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        return extract_video(client, cache, &mcp_id, is_live, page);
    }

    let id = match item.get("id").filter(|value| truthy(value)) {
        Some(Value::String(id)) => id.clone(),
        Some(other) => other.to_string(),
        None => item
            .get("entityId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| key_error(display_id, "entityId"))?,
    };
    let item_url = item
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(display_id, "url"))?;
    let manifest = Url::parse(item_url)?;
    if super::direct::determine_ext(item_url).as_deref() != Some("m3u8") {
        return Err(unsupported("direct media items without an HLS manifest"));
    }
    let stream = super::extract_m3u8_fatal_master(client, &manifest, NAME, &id, None, false)?;
    let mut formats = stream.formats;
    for format in &mut formats {
        if format.ext.is_empty() || format.ext == "unknown_video" {
            format.ext = "mp4".into();
        }
    }

    let image = ["imageSrc", "posterImage"]
        .iter()
        .filter_map(|key| item.get(*key)?.as_str())
        .find_map(super::json_ld::url_or_none);
    let mut fields: Vec<(&str, Value)> = vec![
        ("id", id.clone().into()),
        ("formats", Value::Null),
        ("subtitles", Value::Null),
    ];
    if let Some(title) = item.get("title").and_then(Value::as_str) {
        fields.push(("title", title.into()));
    }
    if let Some(description) = item.get("description").and_then(Value::as_str) {
        fields.push(("description", crate::html::clean_html(description).into()));
    }
    fields.push(("is_live", is_live.into()));
    let thumbnails = match image {
        Some(image) => {
            let ext = super::direct::determine_ext(&image).unwrap_or_else(|| "jpg".into());
            json!([{ "url": image, "ext": ext }])
        }
        None => Value::Null,
    };
    fields.push(("thumbnails", thumbnails));
    let mut info = dict_info(page, (NAME, KEY), fields, formats);
    info.subtitles = stream.subtitles;
    Ok(info)
}

impl Extractor for NflArticle {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;

        // `clean_html(get_element_by_class('nfl-c-article__title', webpage))`
        // or the OpenGraph / Twitter title.
        let heading =
            crate::html_elements::elements_text_and_html_by_class("nfl-c-article__title", &webpage)
                .ok()
                .and_then(|elements| elements.into_iter().next())
                .map(|(_, html)| crate::html::clean_html(&html))
                .filter(|title| !title.is_empty());
        let title = heading.or_else(|| {
            ["og:title", "twitter:title"]
                .iter()
                .find_map(|property| crate::html::html_search_meta(&webpage, property))
        });

        let mut cache = None;
        let mut entries = Vec::new();
        for captures in VIDEO_CONFIG.captures_iter(&webpage) {
            entries.push(parse_video_config(
                client,
                &mut cache,
                &captures[1],
                &display_id,
                url,
            )?);
        }

        let mut extra = Map::new();
        let keys = if title.is_some() { "id,title" } else { "id" };
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: KEY.into(),
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
            "https://www.buffalobills.com/news/the-only-thing-we-ve-earned-is-the-noise-bills-coaches-discuss-handling-rising-e"
        ));
        assert!(suitable(
            "https://www.chiefs.clubs.nfl.com/news/some-article"
        ));
        assert!(!suitable(
            "https://www.buffalobills.com/video/buffalo-bills-military-recognition-week-14"
        ));
        assert!(!suitable(
            "https://www.nfl.com/videos/baker-mayfield-s-game-changing-plays"
        ));
    }

    #[test]
    fn device_ids_are_uuid_shaped() {
        let id = device_id();
        assert_eq!(id.len(), 36);
        assert_eq!(id.matches('-').count(), 4);
    }
}
