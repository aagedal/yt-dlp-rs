//! Upstream `GrouponIE` (`yt_dlp/extractor/groupon.py`): Groupon deals as
//! playlists of their `payload.carousel.dealVideos` YouTube `url` results,
//! with the OpenGraph title and description.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Groupon;

const NAME: &str = "Groupon";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?groupon\.com/deals/(?P<id>[^/?#&]+)").unwrap()
});
static PAYLOAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:var\s+|window\.)payload\s*=\s*(.*?);\n").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Groupon {what} are not supported natively"))
}

fn og(webpage: &str, property: &str, name: &str) -> Value {
    match crate::html::og_search_property(webpage, property) {
        Some(value) => value.into(),
        None => {
            super::search_warning(NAME, name);
            Value::Null
        }
    }
}

impl Extractor for Groupon {
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
        let raw = PAYLOAD
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {playlist_id}: Unable to extract payload; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let payload: Value = crate::json::from_str(&raw).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {playlist_id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let carousel = payload.get("carousel").ok_or_else(|| {
            Error::Reported(format!(
                "{playlist_id}: An extractor error has occurred. (caused by KeyError('carousel')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let videos = match carousel.get("dealVideos") {
            None => Vec::new(),
            Some(Value::Array(videos)) => videos.clone(),
            Some(_) => return Err(unsupported("deal video lists that are not lists")),
        };
        let mut entries = Vec::new();
        for video in &videos {
            let truthy = |key: &str| video.get(key).filter(|value| super::nuxt::truthy(value));
            let provider = match truthy("provider") {
                None => continue,
                Some(Value::String(provider)) => provider.to_lowercase(),
                Some(_) => return Err(unsupported("providers that are not strings")),
            };
            let Some(video_id) = truthy("media")
                .or_else(|| truthy("id"))
                .or_else(|| truthy("baseURL"))
            else {
                continue;
            };
            // Upstream fails unpacking `None` for other providers.
            if provider != "youtube" {
                return Err(unsupported("video providers other than YouTube"));
            }
            let video_id = match video_id {
                Value::String(id) => id.clone(),
                Value::Number(id) => id.to_string(),
                _ => return Err(unsupported("video IDs that are not strings")),
            };
            entries.push(super::nrk::url_entry(
                &video_id,
                "Youtube",
                None,
                super::youtube::temp_id(&video_id),
                None,
            )?);
        }
        let title = og(&webpage, "title", "OpenGraph title");
        let description = og(&webpage, "description", "OpenGraph description");
        let mut extra = Map::new();
        extra.insert("description".into(), description);
        if title.is_null() {
            extra.insert("title".into(), Value::Null);
        }
        // `{'_type': 'playlist', 'id', 'entries', 'title', 'description'}`.
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "typed-playlist".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.as_str().unwrap_or_default().to_owned(),
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
            "https://www.groupon.com/deals/bikram-yoga-huntington-beach-2#ooid=tubGNycTo@9Uxg82uESj4i61EYX8nyuf"
        ));
        assert!(!suitable("https://www.groupon.com/local"));
    }
}
