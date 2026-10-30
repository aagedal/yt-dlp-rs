//! Upstream `DacastVODIE` and `DacastPlaylistIE` (`yt_dlp/extractor/dacast.py`):
//! a VOD's `playback.dacast.com` access JSON and its HLS manifest (with the
//! `uspaes` key signature), and a playlist's contents as VOD url entries.
//! The embed-page discovery (`_extract_embed_urls`) and the CDN's random 403
//! retries are not ported; a 403 from the manifest is an error.
use super::{
    Extractor, dict_info, direct, download_json, download_json_note, download_webpage,
    update_url_query,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

pub struct DacastVod;
pub struct DacastPlaylist;

const VOD_NAME: &str = "DacastVOD";
const PLAYLIST_NAME: &str = "DacastPlaylist";
const API_INFO_URL: &str = "https://playback.dacast.com/content/info";
const API_ACCESS_URL: &str = "https://playback.dacast.com/content/access";
const DEFAULT_SIGNING_SECRET: &str = "hGDtqMKYVeFdofrAfFmBcrsakaZELajI";

static VOD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://iframe\.dacast\.com/vod/(?P<user_id>[\w-]+)/(?P<id>[\w-]+)").unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://iframe\.dacast\.com/playlist/(?P<user_id>[\w-]+)/(?P<id>[\w-]+)")
        .unwrap()
});
static SIGNING_SECRET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bUSP_SIGNING_SECRET\s*=\s*(?:"([^"]+)"|'([^']+)')"#).unwrap());

/// `DacastVODIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VOD_URL.is_match(url)
}

/// `DacastPlaylistIE._match_valid_url`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `DacastBaseIE._get_url_from_id`: a `user-vod-media` content id as a URL.
fn vod_url(content_id: &str) -> Result<Url> {
    let Some((user_id, media_id)) = content_id.split_once("-vod-") else {
        return Err(Error::UnsupportedHtml5(format!(
            "Dacast content id {content_id} is not a VOD content id"
        )));
    };
    Ok(Url::parse(&format!(
        "https://iframe.dacast.com/vod/{user_id}/{media_id}"
    ))?)
}

/// `traverse_obj(url, ({parse_qs}, 'uss_token', {'signedKey': -1}))`.
fn uss_token(url: &Url) -> Option<String> {
    url.query_pairs()
        .filter(|(key, value)| key == "uss_token" && !value.is_empty())
        .last()
        .map(|(_, value)| value.into_owned())
}

/// `url_result(url, ie, id, title)` as a playlist entry (`entry` keeps the
/// upstream key order: `ie_key`, `id`, `title`, `_type`, `url`).
fn url_entry(target: &Url, id: &str, title: Option<&str>) -> VideoInfo {
    let mut info = direct::build_at(target.clone(), target, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = id.to_owned();
    let mut entry = Map::new();
    entry.insert("ie_key".into(), VOD_NAME.into());
    entry.insert("id".into(), id.into());
    if let Some(title) = title {
        entry.insert("title".into(), title.into());
        info.title = title.to_owned();
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.as_str().into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target.as_str(),
            "force_videoid": null,
            "fields": {},
            "ie_key": VOD_NAME,
            "extractor": VOD_NAME,
            "entry": Value::Object(entry),
        }),
    );
    info
}

/// `_download_json(..., expected_status=403)`: the access JSON, whose error
/// responses are parsed too.
fn access_json(client: &Client, url: &Url, id: &str) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{VOD_NAME}] {id}: Downloading access JSON"));
    let mut response =
        direct::request_expecting(client, url, &[], &[403]).map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: VOD_NAME,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            },
            error => error,
        })?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{VOD_NAME}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_usp_signing_secret`: the player's rotating secret, or the fallback.
fn signing_secret(client: &Client, id: &str) -> String {
    let Ok(url) = Url::parse("https://player.dacast.com/js/player.js") else {
        return DEFAULT_SIGNING_SECRET.to_owned();
    };
    download_webpage(client, &url, VOD_NAME, id)
        .ok()
        .and_then(|script| {
            SIGNING_SECRET
                .captures(&script)
                .and_then(|found| found.get(1).or_else(|| found.get(2)))
                .map(|secret| secret.as_str().to_owned())
        })
        .unwrap_or_else(|| DEFAULT_SIGNING_SECRET.to_owned())
}

/// Upstream `float_or_none`.
fn float_or_none(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

impl Extractor for DacastVod {
    fn key(&self) -> &'static str {
        VOD_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VOD_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let user_id = found["user_id"].to_owned();
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(VOD_NAME);
        crate::screen::screen(&format!(
            "[{VOD_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let content_id = format!("{user_id}-vod-{video_id}");
        let mut query = vec![("contentId", content_id.as_str()), ("provider", "universe")];
        let token = uss_token(url);
        if let Some(token) = token.as_deref() {
            query.push(("signedKey", token));
        }

        // `_download_json(..., fatal=False)`: a failure is a warning.
        let info_url = update_url_query(&Url::parse(API_INFO_URL)?, &query);
        let info = match download_json(client, &info_url, VOD_NAME, &video_id) {
            Ok(info) => Some(info),
            Err(error) => {
                crate::screen::warning(&format!(
                    "[{VOD_NAME}] {video_id}: Unable to download JSON metadata: {}",
                    crate::upstream_error_text(&error)
                ));
                None
            }
        };

        let access_url = update_url_query(&Url::parse(API_ACCESS_URL)?, &query);
        let access = access_json(client, &access_url, &video_id)?;
        if let Some(error) = access.get("error").and_then(Value::as_str) {
            if matches!(error, "Broadcaster has been blocked" | "Content is offline") {
                return Err(Error::Reported(format!("[{VOD_NAME}] {video_id}: {error}")));
            }
            return Err(Error::Reported(format!(
                "[{VOD_NAME}] {video_id}: Dacast API says \"{error}\"; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let hls_url = access
            .get("hls")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Reported(format!(
                "[{VOD_NAME}] {video_id}: An extractor error has occurred. (caused by KeyError('hls')); {}",
                crate::BUG_REPORTS_MESSAGE
            )))?
            .to_owned();

        let mut hls_aes = Value::Null;
        if hls_url.contains("DRM_EXT") {
            crate::no_formats::raise_expected(
                VOD_NAME,
                Some(&video_id),
                None,
                "This video is DRM protected",
            )?;
        } else if hls_url.contains("/uspaes/") {
            // Ref: https://player.dacast.com/js/player.js
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs());
            let secret = signing_secret(client, &video_id);
            let digest =
                crate::sha1::digest(format!("{}{ts}{secret}", 10_413_792_000 - ts).as_bytes());
            let signature: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            hls_aes = json!({
                "uri": format!("https://keys.dacast.com/uspaes/{video_id}.key?s={signature}&ts={ts}"),
            });
        }

        let manifest = Url::parse(&hls_url)?;
        let formats = super::extract_m3u8_fatal_master(
            client,
            &manifest,
            VOD_NAME,
            &video_id,
            Some("hls"),
            false,
        )?
        .formats;

        let content = info.as_ref().and_then(|info| info.get("contentInfo"));
        let mut fields = vec![
            ("id", video_id.clone().into()),
            ("uploader_id", user_id.clone().into()),
            ("formats", Value::Null),
            ("hls_aes", hls_aes),
        ];
        // `traverse_obj(info, ('contentInfo', {...}))`: missing values are dropped.
        if let Some(title) = content
            .and_then(|content| content.get("title"))
            .filter(|value| !value.is_null())
        {
            fields.push(("title", title.clone()));
        }
        if let Some(duration) = content
            .and_then(|content| content.get("duration"))
            .and_then(float_or_none)
        {
            fields.push(("duration", json!(duration)));
        }
        if let Some(thumbnail) = content
            .and_then(|content| content.get("thumbnailUrl"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            fields.push(("thumbnail", thumbnail.into()));
        }
        Ok(Some(dict_info(url, (VOD_NAME, VOD_NAME), fields, formats)))
    }
}

impl Extractor for DacastPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let user_id = found["user_id"].to_owned();
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(PLAYLIST_NAME);
        crate::screen::screen(&format!(
            "[{PLAYLIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let content_id = format!("{user_id}-playlist-{playlist_id}");
        let api = update_url_query(
            &Url::parse(API_INFO_URL)?,
            &[("contentId", content_id.as_str()), ("provider", "universe")],
        );
        let body = download_json_note(
            client,
            &api,
            PLAYLIST_NAME,
            &playlist_id,
            "Downloading playlist JSON",
        )?;
        let info = body.get("contentInfo").ok_or_else(|| {
            Error::Reported(format!(
                "[{PLAYLIST_NAME}] {playlist_id}: An extractor error has occurred. (caused by KeyError('contentInfo')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let mut entries = Vec::new();
        let contents = info
            .get("features")
            .and_then(|features| features.get("playlist"))
            .and_then(|playlist| playlist.get("contents"))
            .and_then(Value::as_array);
        for video in contents.into_iter().flatten() {
            let Some(id) = video
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let title = video.get("title").and_then(Value::as_str);
            entries.push(url_entry(&vod_url(id)?, id, title));
        }
        let title = info.get("title").and_then(Value::as_str).map(str::to_owned);
        let mut extra = Map::new();
        let keys = if title.is_some() { "id,title" } else { "id" };
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_NAME.into(),
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
            "https://iframe.dacast.com/vod/acae82153ef4d7a7344ae4eaa86af534/1c6143e3-5a06-371d-8695-19b96ea49090"
        ));
        assert!(!suitable(
            "https://iframe.dacast.com/playlist/943bb1ab3c03695ba85330d92d6d226e/b632eb053cac17a9c9a02bcfc827f2d8"
        ));
        assert!(playlist_suitable(
            "https://iframe.dacast.com/playlist/943bb1ab3c03695ba85330d92d6d226e/b632eb053cac17a9c9a02bcfc827f2d8"
        ));
    }

    #[test]
    fn content_ids_become_vod_urls() {
        assert_eq!(
            vod_url("943bb1ab3c03695ba85330d92d6d226e-vod-164da396-15bf")
                .unwrap()
                .as_str(),
            "https://iframe.dacast.com/vod/943bb1ab3c03695ba85330d92d6d226e/164da396-15bf"
        );
    }
}
