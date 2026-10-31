//! Upstream `AMCNetworksIE` (`yt_dlp/extractor/amcnetworks.py`): AMC, BBC
//! America, IFC, WE tv and Sundance TV pages whose `window.initialData`
//! names a Brightcove video, as a `url` result of its player.
use super::{Extractor, download_webpage, follow_url};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct AmcNetworks;

const NAME: &str = "AMCNetworks";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?(?:amc|bbcamerica|ifc|(?:we|sundance)tv)\.com/(?P<id>(?:movies|shows(?:/[^/?#]+)+)/[^/?#&]+)",
    )
    .unwrap()
});
static INITIAL_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.initialData\s*=\s*JSON\.parse\(String\.raw`").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_json(...)` (fatal) error text.
fn search_error(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// A `KeyError` raised by upstream's subscripts.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// A subscript value as Python's `str()` would show it.
fn py_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

impl Extractor for AmcNetworks {
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
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let initial_data = match super::search_json(&INITIAL_DATA, &webpage, false) {
            Some(Some(data)) => data,
            Some(None) => {
                return Err(Error::UnsupportedHtml5(
                    "AMCNetworks initial data that does not parse as JSON".into(),
                ));
            }
            None => return Err(search_error(&display_id, "initial data")),
        };
        let video_id = initial_data
            .pointer("/initialData/properties/videoId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        let Some(video_id) = video_id else {
            // `report_drm(display_id)`: every locked video is DRM-protected.
            crate::no_formats::raise_expected(
                NAME,
                Some(&display_id),
                None,
                "This video is DRM protected",
            )?;
            return Err(Error::UnsupportedHtml5(
                "AMCNetworks DRM videos with --ignore-no-formats-error".into(),
            ));
        };
        let config = initial_data
            .get("config")
            .ok_or_else(|| key_error(&display_id, "config"))?;
        let brightcove = config
            .get("brightcove")
            .ok_or_else(|| key_error(&display_id, "brightcove"))?;
        let account_id = py_text(
            brightcove
                .get("accountId")
                .ok_or_else(|| key_error(&display_id, "accountId"))?,
        );
        let player_id = py_text(
            brightcove
                .get("playerId")
                .ok_or_else(|| key_error(&display_id, "playerId"))?,
        );
        // `url_result(player URL, BrightcoveNewIE, video_id)`.
        let target = format!(
            "https://players.brightcove.net/{account_id}/{player_id}_default/index.html?videoId={video_id}"
        );
        Ok(Some(follow_url(url, target)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.amc.com/shows/dark-winds/videos/dark-winds-a-look-at-season-3--1072027"
        ));
        assert!(suitable(
            "http://www.bbcamerica.com/shows/the-hunt/full-episodes/season-1/episode-01-the-hardest-challenge"
        ));
        assert!(suitable("http://www.ifc.com/movies/chaos"));
        assert!(suitable(
            "https://www.sundancetv.com/shows/riviera/full-episodes/season-1/episode-01-episode-1"
        ));
        assert!(!suitable("https://www.amc.com/"));
    }
}
