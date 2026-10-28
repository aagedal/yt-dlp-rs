//! Upstream `SteamIE`, `SteamCommunityIE` and `SteamCommunityBroadcastIE`
//! (`yt_dlp/extractor/steam.py`): a store app's trailer carousel (with the
//! age-check cookies) as a playlist of HLS/DASH trailers, a community file's
//! YouTube video as a `url` result, and a community broadcast's live HLS.
//!
//! The app's trailers are pending entries, extracted as the playlist is
//! processed like upstream's generator.
use super::{Extractor, dict_info, download_json, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Steam;
pub struct SteamCommunity;
pub struct SteamCommunityBroadcast;

const NAME: &str = "Steam";
const COMMUNITY_NAME: &str = "SteamCommunity";
const BROADCAST_NAME: &str = "SteamCommunityBroadcast";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://store\.steampowered\.com(?:/agecheck)?/app/(?P<id>\d+)/?(?:[^?/#]+/?)?(?:[?#]|$)",
    )
    .unwrap()
});
static COMMUNITY_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?steamcommunity\.com/sharedfiles/filedetails(?:/?\?(?:[^#]+&)?id=|/)(?P<id>\d+)").unwrap()
});
static BROADCAST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?steamcommunity\.com/broadcast/watch/(?P<id>\d+)").unwrap()
});
static FLASHVARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+rgMovieFlashvars\s*=").unwrap());

/// `SteamIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `SteamCommunityIE._match_valid_url`.
pub fn community_suitable(url: &str) -> bool {
    COMMUNITY_URL.is_match(url)
}

/// `SteamCommunityBroadcastIE._match_valid_url`.
pub fn broadcast_suitable(url: &str) -> bool {
    BROADCAST_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Steam {what} are not supported natively"))
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `{find_element(cls=CLASS, html=True)}, {extract_attributes}`.
fn element_attributes(class: &str, html: &str) -> Option<Map<String, Value>> {
    let (_, element) = crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()?;
    Some(crate::html_elements::extract_attributes(&element))
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.into()),
        _ => None,
    }
}

/// `url_basename`.
fn url_basename(text: &str) -> String {
    let path = match Url::parse(text) {
        Ok(url) => url.path().to_owned(),
        Err(_) => text.split(['?', '#']).next().unwrap_or_default().to_owned(),
    };
    path.trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// `_remove_duplicate_formats`: the first format of each URL, or of each
/// set of fragment URLs for fragmented formats.
pub(crate) fn remove_duplicate_formats(formats: &mut Vec<crate::VideoFormat>) {
    let mut seen_urls: Vec<Url> = Vec::new();
    let mut seen_fragments: Vec<Vec<String>> = Vec::new();
    formats.retain(|format| {
        if format.fragments.is_empty() {
            if seen_urls.contains(&format.url) {
                return false;
            }
            seen_urls.push(format.url.clone());
            return true;
        }
        let mut urls: Vec<String> = format
            .fragments
            .iter()
            .map(|fragment| fragment.url.to_string())
            .collect();
        urls.sort();
        urls.dedup();
        if seen_fragments.contains(&urls) {
            return false;
        }
        seen_fragments.push(urls);
        true
    });
}

/// `join_nonempty(*values, delim=delim)`.
fn join_nonempty(values: &[Option<&str>], delim: &str) -> String {
    values
        .iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(delim)
}

impl Extractor for Steam {
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
        let app_id = found["id"].to_owned();
        start(NAME, url);
        for (name, value) in [
            ("wants_mature_content", "1"),
            ("birthtime", "946652401"),
            ("lastagecheckage", "1-January-2000"),
        ] {
            crate::cookies::set_cookie("store.steampowered.com", name, value);
        }
        let webpage = download_webpage(client, url, NAME, &app_id)?;
        let data_props = element_attributes("gamehighlight_desktopcarousel", &webpage)
            .and_then(|attributes| attributes.get("data-props").cloned())
            .and_then(|props| props.as_str().map(str::to_owned))
            .and_then(|props| crate::json::from_str::<Value>(&props).ok())
            .filter(Value::is_object);
        let app_name = data_props
            .as_ref()
            .and_then(|props| props.get("appName"))
            .and_then(Value::as_str)
            .map(crate::html::clean_html);
        let trailers: Vec<Value> = data_props
            .as_ref()
            .and_then(|props| props.get("trailers"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Upstream's `_entries` generator: trailers are extracted as the
        // playlist is processed (see `fetch_trailers`).
        let state = json!({
            "kind": "steam",
            "url": url.as_str(),
            "app_id": app_id,
            "app_name": app_name,
            "trailers": trailers,
        });
        let title = app_name.filter(|name| !name.is_empty()).unwrap_or_default();
        let keys = if title.is_empty() { "id" } else { "id,title" };
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}{}", crate::info_order::GENERATOR).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: app_id,
            title,
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            nested: Vec::new(),
            pending: Some(crate::PendingEntries { state }),
            entries: Vec::new(),
            extra,
        })))
    }
}

/// Whether a playlist's pending state is a Steam app's trailers.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == "steam"
}

/// The pending trailers of a Steam app playlist, extracted in order until
/// `limit` entries exist (upstream's `_entries` generator).
pub(crate) fn fetch_trailers(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let Value::Object(mut state) = state else {
        return Ok(None);
    };
    let url = Url::parse(state["url"].as_str().unwrap_or_default())?;
    let app_id = state["app_id"].as_str().unwrap_or_default().to_owned();
    let app_name = state["app_name"].as_str().map(str::to_owned);
    let mut trailers: std::collections::VecDeque<Value> = match state.shift_remove("trailers") {
        Some(Value::Array(trailers)) => trailers.into(),
        _ => Default::default(),
    };
    while let Some(trailer) = trailers.pop_front() {
        if limit.is_some_and(|limit| entries.len() >= limit) {
            trailers.push_front(trailer);
            break;
        }
        if let Some(entry) = trailer_entry(client, &url, &app_id, app_name.as_deref(), &trailer)? {
            entries.push(entry);
        }
    }
    if trailers.is_empty() {
        return Ok(None);
    }
    state.insert("trailers".into(), Value::Array(trailers.into()));
    Ok(Some(Value::Object(state)))
}

/// One trailer of `_entries`: `None` without an `id`.
fn trailer_entry(
    client: &Client,
    url: &Url,
    app_id: &str,
    app_name: Option<&str>,
    trailer: &Value,
) -> Result<Option<VideoInfo>> {
    let app_id = app_id.to_owned();
    let app_name = app_name.map(str::to_owned);

    // `lambda _, v: str_or_none(v['id'])`.
    let Some(movie_id) = trailer
        .as_object()
        .and_then(|trailer| str_or_none(trailer.get("id")))
        .filter(|id| !id.is_empty())
    else {
        return Ok(None);
    };
    let mut thumbnails = Vec::new();
    for key in ["poster", "thumbnail"] {
        let Some(thumbnail) = trailer
            .get(key)
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let mut entry = Map::new();
        entry.insert("url".into(), thumbnail.clone().into());
        let (width, height) = super::html5::parse_resolution(&url_basename(&thumbnail));
        if let Some(width) = width {
            entry.insert("width".into(), width.into());
        }
        if let Some(height) = height {
            entry.insert("height".into(), height.into());
        }
        thumbnails.push(Value::Object(entry));
    }
    let mut formats = Vec::new();
    if let Some(manifest) = trailer
        .get("hlsManifest")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
    {
        let manifest = Url::parse(&manifest)?;
        if let Some(stream) = super::extract_m3u8_master_optional(
            client,
            &manifest,
            NAME,
            &app_id,
            Some("hls"),
            false,
            false,
        ) {
            formats.extend(stream.formats);
        }
    }
    for manifest in trailer
        .get("dashManifests")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(super::json_ld::url_or_none)
    {
        let manifest = Url::parse(&manifest)?;
        if let Some(stream) = super::extract_mpd_optional_note(
            client,
            &manifest,
            NAME,
            &app_id,
            Some("dash"),
            "Downloading MPD manifest",
        ) {
            formats.extend(stream.formats);
        }
    }
    remove_duplicate_formats(&mut formats);
    let mut info = dict_info(
        url,
        (NAME, NAME),
        vec![
            (
                "id",
                join_nonempty(&[Some(&app_id), Some(&movie_id)], "-").into(),
            ),
            (
                "title",
                join_nonempty(&[app_name.as_deref(), Some("video"), Some(&movie_id)], " ").into(),
            ),
            ("formats", Value::Null),
            ("series", app_name.clone().map_or(Value::Null, Value::from)),
            ("series_id", app_id.clone().into()),
            ("thumbnails", Value::Array(thumbnails)),
        ],
        formats,
    );
    if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
        shape.push_str(";thumbnail=url,width,height");
    }
    Ok(Some(info))
}

impl Extractor for SteamCommunity {
    fn key(&self) -> &'static str {
        COMMUNITY_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = COMMUNITY_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let file_id = found["id"].to_owned();
        start(COMMUNITY_NAME, url);
        let webpage = download_webpage(client, url, COMMUNITY_NAME, &file_id)?;
        // `_search_json(..., default={}, transform_source=js_to_json)`.
        let flashvars = super::search_json(&FLASHVARS, &webpage, true)
            .flatten()
            .unwrap_or(json!({}));
        // `(..., 'YOUTUBE_VIDEO_ID', {str}, any)`.
        let from_flashvars = match &flashvars {
            Value::Object(map) => map.values().find_map(|value| {
                value
                    .get("YOUTUBE_VIDEO_ID")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }),
            Value::Array(items) => items.iter().find_map(|value| {
                value
                    .get("YOUTUBE_VIDEO_ID")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }),
            _ => None,
        };
        let youtube_id = from_flashvars
            .filter(|id| !id.is_empty())
            .or_else(|| {
                element_attributes("movieFrame modal", &webpage)
                    .and_then(|attributes| attributes.get("id").cloned())
                    .and_then(|id| id.as_str().map(str::to_owned))
            })
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                Error::Reported(format!("[{COMMUNITY_NAME}] {file_id}: No video found"))
            })?;
        let target = super::target_url(&youtube_id)?;
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow
            .extra
            .insert(crate::FOLLOW_URL.into(), target.as_str().into());
        Ok(Some(follow))
    }
}

impl Extractor for SteamCommunityBroadcast {
    fn key(&self) -> &'static str {
        BROADCAST_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = BROADCAST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(BROADCAST_NAME, url);
        let webpage = download_webpage(client, url, BROADCAST_NAME, &video_id)?;
        let api = super::update_url_query(
            &Url::parse("https://steamcommunity.com/broadcast/getbroadcastmpd/")?,
            &[("steamid", &video_id)],
        );
        let json_data = download_json(client, &api, BROADCAST_NAME, &video_id)?;
        let hls_url = json_data
            .get("hls_url")
            .ok_or_else(|| super::clyp::key_error(&video_id, "hls_url"))?
            .as_str()
            .ok_or_else(|| unsupported("HLS URLs that are not strings"))?;
        let manifest = Url::parse(hls_url)?;
        let stream = super::extract_m3u8_fatal_master(
            client,
            &manifest,
            BROADCAST_NAME,
            &video_id,
            None,
            true,
        )?;
        let users = super::update_url_query(
            &Url::parse("https://steamcommunity.com/actions/ajaxresolveusers")?,
            &[("steamids", &video_id)],
        );
        let users = download_json(client, &users, BROADCAST_NAME, &video_id)?;
        let uploader = users
            .as_array()
            .ok_or_else(|| unsupported("user lists that are not arrays"))?
            .first()
            // An `IndexError` is not wrapped in an `ExtractorError`.
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        // `_generic_title('', webpage)`: the OpenGraph title, else `<title>`.
        let title = crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
            .or_else(|| crate::html::html_extract_title(&webpage))
            .unwrap_or_default();
        let mut info = dict_info(
            url,
            (BROADCAST_NAME, BROADCAST_NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", title.into()),
                ("formats", Value::Null),
                ("live_status", "is_live".into()),
                (
                    "view_count",
                    json_data.get("num_view").cloned().unwrap_or(Value::Null),
                ),
                (
                    "uploader",
                    uploader.get("persona_name").cloned().unwrap_or(Value::Null),
                ),
                (
                    "uploader_id",
                    str_or_none(uploader.get("accountid")).map_or(Value::Null, Value::from),
                ),
                ("subtitles", Value::Null),
            ],
            stream.formats,
        );
        info.subtitles = stream.subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://store.steampowered.com/app/105600"));
        assert!(suitable(
            "https://store.steampowered.com/app/271590/Grand_Theft_Auto_V/"
        ));
        assert!(suitable(
            "https://store.steampowered.com/agecheck/app/271590/?x=1"
        ));
        assert!(!suitable("https://store.steampowered.com/app/271590/a/b"));
        assert!(community_suitable(
            "https://steamcommunity.com/sharedfiles/filedetails/?id=242472205"
        ));
        assert!(community_suitable(
            "https://steamcommunity.com/sharedfiles/filedetails/242472205"
        ));
        assert!(broadcast_suitable(
            "https://steamcommunity.com/broadcast/watch/76561199073851486"
        ));
    }
}
