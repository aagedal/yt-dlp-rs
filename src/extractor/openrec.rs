//! Upstream `OpenRecCaptureIE`, `OpenRecPlaylistIE` and
//! `OpenRecChannelSearchIE` (`yt_dlp/extractor/openrec.py`, mellow-fan.com):
//! a capture page's `window.pageStore` HLS master, a playlist's captures
//! (the webpage's links; the account API's playlists are not ported) and a
//! channel's capture search through the public search API.
use super::{
    Extractor, dict_info, download_json, download_json_note, download_webpage, update_url_query,
};
use crate::html::clean_html;
use crate::html_elements::elements_text_and_html_by_class;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct OpenRecCapture;
pub struct OpenRecPlaylist;
pub struct OpenRecChannelSearch;

const CAPTURE_NAME: &str = "mellowfan:capture";
const CAPTURE_KEY: &str = "OpenRecCapture";
const PLAYLIST_NAME: &str = "mellowfan:playlist";
const PLAYLIST_KEY: &str = "OpenRecPlaylist";
const SEARCH_NAME: &str = "mellowfan:channel:search";
const SEARCH_KEY: &str = "OpenRecChannelSearch";
const BASE_URL: &str = "https://www.mellow-fan.com";
const API_BASE: &str = "https://apiv5.mellow-fan.com/api/v5";
const PUBLIC_API_BASE: &str = "https://public.mellow-fan.com/external/api/v5";

static CAPTURE_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?(?:mellow-fan\.com|openrec\.tv)/(?:m/)?capture/(?P<id>[^/?#]+)",
    )
    .unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?(?:mellow-fan\.com|openrec\.tv)/(?:m/)?user/[^/?#]+/playlist/(?P<id>[^/?#]+)",
    )
    .unwrap()
});
static SEARCH_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?(?:mellow-fan\.com|openrec\.tv)/(?:m/)?user/(?P<id>[^/?#]+)/search(?:/(?P<type>capture|movie))?(?:[/?#]|$)",
    )
    .unwrap()
});
static PAGE_STORE_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.pageStore\s*=").unwrap());
static HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bhref\s*=\s*"([^"]*)""#).unwrap());

/// `_extract_m3u8_formats(headers=self._HEADERS)`'s `Referer`.
const REFERER: &str = "https://www.mellow-fan.com/";

/// `OpenRecCaptureIE._match_valid_url`.
pub fn capture_suitable(url: &str) -> bool {
    CAPTURE_URL.is_match(url)
}

/// `OpenRecPlaylistIE._match_valid_url`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `OpenRecChannelSearchIE._match_valid_url`.
pub fn channel_search_suitable(url: &str) -> bool {
    SEARCH_URL.is_match(url)
}

/// `KeyError` on a missing key, as the extractor's error text.
fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `str_or_none(value)` for an ID.
fn id_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `clean_html` of a string field, `filter`ed to non-empty text.
fn clean_text(value: Option<&Value>) -> Option<String> {
    let text = clean_html(value?.as_str()?);
    (!text.is_empty()).then_some(text)
}

/// `url_or_none` of a field.
fn url_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    (text.starts_with("https://") || text.starts_with("http://")).then(|| text.to_owned())
}

/// `url_result(url)` as a playlist entry, with or without an `ie_key`.
fn url_entry(target: &str, ie_key: Option<&str>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    if let Some(ie_key) = ie_key {
        entry.insert("ie_key".into(), ie_key.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// A `playlist_result` for the given entries.
fn playlist(
    id: String,
    title: Option<String>,
    url: &Url,
    (name, key): (&str, &str),
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut extra = Map::new();
    if title.is_none() {
        extra.insert("title".into(), Value::Null);
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "playlist-dict;id,title".into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

/// `_extract_pagestore`'s plain `window.pageStore = {...}` form.
fn page_store(webpage: &str, id: &str) -> Result<Value> {
    super::search_json(&PAGE_STORE_START, webpage, false)
        .flatten()
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{CAPTURE_NAME}] {id}: Unable to extract window pagestore; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

impl Extractor for OpenRecCapture {
    fn key(&self) -> &'static str {
        CAPTURE_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = CAPTURE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(CAPTURE_NAME);
        crate::screen::screen(&format!(
            "[{CAPTURE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, CAPTURE_NAME, &video_id)?;
        let store = page_store(&webpage, &video_id)?;
        let capture = store
            .get("capture")
            .ok_or_else(|| key_error(&video_id, "capture"))?;
        let manifest = url_text(capture.get("source")).ok_or_else(|| {
            Error::Reported(format!("[{CAPTURE_NAME}] {video_id}: No HLS source"))
        })?;
        let manifest = Url::parse(&manifest)?;
        let referer = [("Referer", REFERER)];
        let mut formats = Vec::new();
        if let Some(mut stream) = super::extract_m3u8_master_optional_headers(
            client,
            &manifest,
            CAPTURE_NAME,
            &video_id,
            &referer,
        ) {
            formats = std::mem::take(&mut stream.formats);
        }
        for format in &mut formats {
            let mut headers = Map::new();
            headers.insert("Referer".into(), REFERER.into());
            format.http_headers = Some(headers);
        }

        let channel = store.get("movie").and_then(|movie| movie.get("channel"));
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("formats", Value::Null),
            ("http_headers", json!({"Referer": REFERER})),
        ];
        if let Some(name) = clean_text(channel.and_then(|c| c.get("name"))) {
            fields.push(("channel", name.into()));
        }
        if let Some(id) = id_text(channel.and_then(|c| c.get("id"))) {
            fields.push(("channel_id", id.into()));
        }
        if let Some(title) = clean_text(capture.get("title")) {
            fields.push(("title", title.into()));
        }
        if let (Some(start), Some(end)) = (
            capture.get("startTime").and_then(Value::as_i64),
            capture.get("endTime").and_then(Value::as_i64),
        ) {
            fields.push(("duration", (end - start).into()));
        }
        if let Some(thumbnail) = url_text(capture.get("thumbnailUrl")) {
            fields.push(("thumbnail", thumbnail.into()));
        }
        if let Some(timestamp) = capture
            .get("publishedAt")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.push(("timestamp", timestamp.into()));
        }
        Ok(Some(dict_info(
            url,
            (CAPTURE_NAME, CAPTURE_KEY),
            fields,
            formats,
        )))
    }
}

impl Extractor for OpenRecPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(PLAYLIST_NAME);
        crate::screen::screen(&format!(
            "[{PLAYLIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // The account API answers with its status for anonymous requests;
        // an answer that lists items is upstream's unported API path.
        let me = download_json(
            client,
            &Url::parse(&format!("{API_BASE}/users/me/playlists/{playlist_id}"))?,
            PLAYLIST_NAME,
            &playlist_id,
        )?;
        let has_items = me
            .get("data")
            .and_then(|data| data.get("items"))
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(Value::is_object));
        if has_items {
            return Err(Error::Reported(format!(
                "[{PLAYLIST_NAME}] {playlist_id}: Playlists from the account API are not supported natively"
            )));
        }

        let webpage = download_webpage(client, url, PLAYLIST_NAME, &playlist_id)?;
        let title = elements_text_and_html_by_class("sc-1ak77bz-10", &webpage)
            .ok()
            .and_then(|found| found.into_iter().next())
            .map(|(text, _)| clean_html(&text))
            .filter(|text| !text.is_empty());
        let mut entries = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for (_, outer) in
            elements_text_and_html_by_class("sc-vc0xhn-0", &webpage).unwrap_or_default()
        {
            let Some((_, inner)) = elements_text_and_html_by_class("sc-1ddd11y-0", &outer)
                .unwrap_or_default()
                .into_iter()
                .next()
            else {
                continue;
            };
            let Some(href) = HREF.captures(&inner).map(|found| found[1].to_owned()) else {
                continue;
            };
            // `urljoin('https://www.mellow-fan.com/', href)`, de-duplicated by
            // `playlist_from_matches`.
            let target = Url::parse(&format!("{BASE_URL}/"))?.join(&href)?;
            if !seen.iter().any(|known| known == target.as_str()) {
                seen.push(target.as_str().to_owned());
                entries.push(url_entry(target.as_str(), None)?);
            }
        }
        Ok(Some(playlist(
            playlist_id,
            title,
            url,
            (PLAYLIST_NAME, PLAYLIST_KEY),
            entries,
        )))
    }
}

impl Extractor for OpenRecChannelSearch {
    fn key(&self) -> &'static str {
        SEARCH_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SEARCH_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let channel_id = found["id"].to_owned();
        let search_type = found.name("type").map(|kind| kind.as_str().to_owned());
        crate::screen::set_extractor(SEARCH_NAME);
        crate::screen::screen(&format!(
            "[{SEARCH_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `parse_qs(url)['search_query'][-1]`.
        let search_query = url
            .query_pairs()
            .filter(|(key, _)| key == "search_query")
            .map(|(_, value)| value.into_owned())
            .filter(|value| !value.is_empty())
            .last()
            .ok_or_else(|| Error::Reported("Unable to extract search query".into()))?;

        let Some(search_type) = search_type else {
            let mut entries = Vec::new();
            for kind in ["capture", "movie"] {
                let search_url = update_url_query(
                    &Url::parse(&format!("{BASE_URL}/user/{channel_id}/search/{kind}"))?,
                    &[("search_query", &search_query)],
                );
                entries.push(url_entry(search_url.as_str(), Some(SEARCH_KEY))?);
            }
            return Ok(Some(playlist(
                channel_id.clone(),
                Some(format!("{channel_id}:{search_query}")),
                url,
                (SEARCH_NAME, SEARCH_KEY),
                entries,
            )));
        };

        let mut entries = Vec::new();
        for page in 1.. {
            let api = update_url_query(
                &Url::parse(&format!("{PUBLIC_API_BASE}/search-{search_type}s"))?,
                &[
                    ("channel_ids", &channel_id),
                    ("page", &page.to_string()),
                    ("search_query", &search_query),
                ],
            );
            let items = download_json_note(
                client,
                &api,
                SEARCH_NAME,
                &channel_id,
                &format!("Downloading page {page}"),
            )?;
            let Some(items) = items.as_array().filter(|items| !items.is_empty()) else {
                break;
            };
            for item in items {
                let item_type = if search_type == "movie"
                    && item.get("is_live").and_then(Value::as_bool) == Some(true)
                {
                    "live"
                } else {
                    search_type.as_str()
                };
                // `(None, 'capture')` then `'id'`: the item's own ID first.
                let item_id = id_text(item.get("id"))
                    .or_else(|| id_text(item.get("capture").and_then(|c| c.get("id"))));
                let (ie_key, target) = match item_type {
                    "capture" => (
                        CAPTURE_KEY,
                        format!("{BASE_URL}/capture/{}", item_id.unwrap_or_default()),
                    ),
                    "live" => (
                        "OpenRec",
                        format!("{BASE_URL}/live/{}", item_id.unwrap_or_default()),
                    ),
                    _ => (
                        "OpenRecMovie",
                        format!("{BASE_URL}/movie/{}", item_id.unwrap_or_default()),
                    ),
                };
                entries.push(url_entry(&target, Some(ie_key))?);
            }
        }

        Ok(Some(playlist(
            channel_id.clone(),
            Some(format!("{channel_id}:{search_query}:{search_type}")),
            url,
            (SEARCH_NAME, SEARCH_KEY),
            entries,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(capture_suitable(
            "https://www.mellow-fan.com/capture/l2q00vxl8q8"
        ));
        assert!(capture_suitable("https://openrec.tv/m/capture/9pdz9334vng"));
        assert!(!capture_suitable("https://www.mellow-fan.com/live/abc"));
        assert!(playlist_suitable(
            "https://www.mellow-fan.com/user/sagara_mayu/playlist/xngNMzv71yLjGdW"
        ));
        assert!(channel_search_suitable(
            "https://www.mellow-fan.com/user/DbD_BPF/search/capture?search_query=x"
        ));
        assert!(channel_search_suitable(
            "https://www.mellow-fan.com/user/DbD_BPF/search?search_query=x"
        ));
        assert!(!channel_search_suitable(
            "https://www.mellow-fan.com/user/DbD_BPF"
        ));
    }
}
