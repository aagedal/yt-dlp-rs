//! Upstream `AtScaleConfEventIE` (`yt_dlp/extractor/atscaleconf.py`): an
//! @Scale conference event page's `data-url` video links as a playlist of
//! generic `url` results, titled from the page's OpenGraph title and
//! description.
use super::{Extractor, download_webpage};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct AtScaleConfEvent;

const NAME: &str = "AtScaleConfEvent";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?atscaleconference\.com/events/(?P<id>[^/&$?]+)",
    )
    .unwrap()
});
static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"data-url\s*=\s*"(https?://(?:www\.)?atscaleconference\.com/videos/[^"]+)""#)
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_og_search_property(prop, webpage)` (non-fatal: a warning when absent).
fn og_search(webpage: &str, prop: &str) -> Option<String> {
    let value = crate::html::og_search_property(webpage, prop);
    if value.is_none() {
        super::search_warning(NAME, &format!("OpenGraph {prop}"));
    }
    value
}

impl Extractor for AtScaleConfEvent {
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
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &playlist_id)?;
        // `playlist_from_matches(re.findall(...), ie='Generic', ...)`:
        // ordered, deduplicated `url` results.
        let mut seen: Vec<&str> = Vec::new();
        for found in VIDEO_URL.captures_iter(&webpage) {
            let target = found.get(1).unwrap().as_str();
            if !seen.contains(&target) {
                seen.push(target);
            }
        }
        let title = og_search(&webpage, "title");
        let description = og_search(&webpage, "description");
        let entries = seen
            .iter()
            .map(|target| super::nrk::url_entry(target, "Generic", None, None, None))
            .collect::<Result<Vec<_>>>()?;
        // `playlist_result(entries, playlist_id, title=..., description=...)`:
        // the keyword fields first (`None` values kept), then the ID.
        let mut extra = Map::new();
        if title.is_none() {
            extra.insert("title".into(), Value::Null);
        }
        extra.insert(
            "description".into(),
            description.map_or(Value::Null, Value::from),
        );
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;title,description,id{}",
                crate::info_order::GENERATOR
            )
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
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
            "https://atscaleconference.com/events/data-scale-spring-2022/"
        ));
        assert!(suitable(
            "https://www.atscaleconference.com/events/video-scale-2021"
        ));
        assert!(!suitable("https://atscaleconference.com/videos/x/"));
    }
}
