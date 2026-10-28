//! Upstream `HollywoodReporterIE` and `HollywoodReporterPlaylistIE`
//! (`yt_dlp/extractor/hollywoodreporter.py`): video pages as `url` results
//! for their JW Platform or YouTube showcase video, and video categories as
//! paged playlists of their video page links.
use super::{Extractor, download_webpage, download_webpage_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct HollywoodReporter;
pub struct HollywoodReporterPlaylist;

const NAME: &str = "HollywoodReporter";
const PLAYLIST_NAME: &str = "HollywoodReporterPlaylist";
const PAGE_SIZE: usize = 15;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?hollywoodreporter\.com/video/(?P<id>[\w-]+)").unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?hollywoodreporter\.com/vcategory/(?P<slug>[\w-]+)-(?P<id>\d+)")
        .unwrap()
});
static TITLE_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+href="([^"]+)"[^>]+class="c-title__link"#).unwrap());

/// `HollywoodReporterIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `HollywoodReporterPlaylistIE._match_valid_url`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `get_element_html_by_class` / `get_element_by_class`: the first
/// element's content and whole source.
fn element_by_class(class: &str, html: &str) -> Result<Option<(String, String)>> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .map(|elements| elements.into_iter().next())
        .map_err(Error::Reported)
}

/// `str` attribute values (`extract_attributes` keeps them as strings).
fn attribute(data: &Map<String, Value>, key: &str, id: &str) -> Result<String> {
    match data.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        // A valueless attribute is `None`.
        Some(_) => Err(Error::UnsupportedHtml5(format!(
            "Hollywood Reporter showcase attributes without a value ({key}) are not supported natively"
        ))),
        None => Err(key_error(id, key)),
    }
}

impl Extractor for HollywoodReporter {
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
        let display_id = found["id"].to_owned();
        announce(NAME, url);
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let card = element_by_class("vlanding-video-card__link", &webpage)?
            .map(|(_, whole)| whole)
            .unwrap_or_default();
        let data = crate::html_elements::extract_attributes(&card);
        let video_id = attribute(&data, "data-video-showcase-trigger", &display_id)?;
        let showcase_type = attribute(&data, "data-video-showcase-type", &display_id)?;
        match showcase_type.as_str() {
            "jwplayer" => Ok(Some(super::follow_url(
                url,
                format!("jwplatform:{video_id}"),
            ))),
            "youtube" => Ok(Some(super::follow_url(url, video_id))),
            _ => Err(Error::Reported(format!(
                "[{NAME}] {display_id}: Unsupported showcase type \"{showcase_type}\"; {}",
                crate::BUG_REPORTS_MESSAGE
            ))),
        }
    }
}

/// `url_result(url, HollywoodReporterIE)`.
fn video_entry(target: &str) -> Result<VideoInfo> {
    let temp_id = VALID_URL
        .captures(target)
        .map(|found| found["id"].to_owned());
    let missing = temp_id.is_none();
    let mut entry = super::nrk::url_entry(target, NAME, None, temp_id, None)?;
    // `get_temp_id(url)` is `None` for a URL this extractor does not match.
    if missing {
        entry.id = String::new();
    }
    Ok(entry)
}

/// `_fetch_page(slug, pl_id, page)`.
fn fetch_page(client: &Client, slug: &str, pl_id: &str, page: usize) -> Result<Vec<VideoInfo>> {
    let page = page + 1;
    let webpage = download_webpage_note(
        client,
        &Url::parse(&format!(
            "https://www.hollywoodreporter.com/vcategory/{slug}-{pl_id}/page/{page}/"
        ))?,
        PLAYLIST_NAME,
        pl_id,
        &format!("Downloading playlist page {page}"),
    )?;
    let section = element_by_class("video-playlist-river", &webpage)?
        .map(|(content, _)| content)
        .unwrap_or_default();
    TITLE_LINK
        .captures_iter(&section)
        .map(|found| video_entry(&found[1]))
        .collect()
}

/// Whether a playlist's pending state is a Hollywood Reporter category's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == PLAYLIST_NAME
}

/// `OnDemandPagedList(functools.partial(self._fetch_page, slug, pl_id), 15)`.
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let slug = state["slug"].as_str().unwrap_or_default().to_owned();
    let pl_id = state["id"].as_str().unwrap_or_default().to_owned();
    let mut fetch = |page: usize| fetch_page(client, &slug, &pl_id, page);
    let placeholder = || video_entry("https://www.hollywoodreporter.com/video/None");
    super::paged::fetch_requested(state, limit, entries, PAGE_SIZE, &mut fetch, &placeholder)
}

impl Extractor for HollywoodReporterPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let slug = found["slug"].to_owned();
        let pl_id = found["id"].to_owned();
        announce(PLAYLIST_NAME, url);
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;id,title{}", crate::info_order::GENERATOR).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: pl_id.clone(),
            title: slug.clone(),
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_NAME.into(),
            nested: Vec::new(),
            pending: Some(crate::PendingEntries {
                state: json!({
                    "kind": PLAYLIST_NAME,
                    "slug": slug,
                    "id": pl_id,
                    "random_access": true,
                }),
            }),
            entries: Vec::new(),
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
            "https://www.hollywoodreporter.com/video/chris-pine-michelle-rodriguez-sxsw-2023/"
        ));
        assert!(playlist_suitable(
            "https://www.hollywoodreporter.com/vcategory/heat-vision-breakdown-57822/"
        ));
        assert!(!suitable(
            "https://www.hollywoodreporter.com/vcategory/heat-vision-breakdown-57822/"
        ));
        assert!(!playlist_suitable(
            "https://www.hollywoodreporter.com/video/x"
        ));
    }
}
