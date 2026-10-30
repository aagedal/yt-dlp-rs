//! Upstream `NTSLiveIE` (`yt_dlp/extractor/nts.py`): the episode page's
//! `window._REACT_STATE_` episode, whose first audio source URL (usually a
//! SoundCloud or Mixcloud page) is followed as a `url_transparent` result
//! overlaid with the episode's name, description, genres and dates.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NtsLive;

const NAME: &str = "nts.live";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?nts\.live/shows/[^/?#]+/episodes/(?P<id>[^/?#]+)",
    )
    .unwrap()
});
static REACT_STATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\._REACT_STATE_\s*=").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for NtsLive {
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
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `_search_json(r'window\._REACT_STATE_\s*=', webpage, 'react state', video_id)`.
        let data = match super::search_json(&REACT_STATE, &webpage, false) {
            Some(Some(data)) => data,
            Some(None) => {
                return Err(Error::UnsupportedHtml5(
                    "NTS react states that are not valid JSON are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract react state; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let episode = data.get("episode").and_then(Value::as_object);
        let text = |key: &str| {
            episode
                .and_then(|episode| episode.get(key))
                .and_then(Value::as_str)
        };
        // `('audio_sources', ..., 'url', {url_or_none}, any)`.
        let sources = match episode.and_then(|episode| episode.get("audio_sources")) {
            Some(Value::Array(items)) => items.iter().collect::<Vec<_>>(),
            Some(Value::Object(map)) => map.values().collect(),
            _ => Vec::new(),
        };
        let media = sources.into_iter().find_map(|source| {
            source
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
        });
        let mut fields = Map::new();
        if let Some(title) = text("name") {
            fields.insert("title".into(), title.into());
        }
        if let Some(description) = text("description") {
            fields.insert("description".into(), description.into());
        }
        let genres: Vec<Value> = match episode.and_then(|episode| episode.get("genres")) {
            Some(Value::Array(items)) => items.iter().collect::<Vec<_>>(),
            Some(Value::Object(map)) => map.values().collect(),
            _ => Vec::new(),
        }
        .into_iter()
        .filter_map(|genre| genre.get("value").and_then(Value::as_str))
        .map(Value::from)
        .collect();
        if !genres.is_empty() {
            fields.insert("genres".into(), Value::Array(genres));
        }
        for (key, source) in [
            ("timestamp", "broadcast"),
            ("modified_timestamp", "updated"),
        ] {
            if let Some(timestamp) = text(source).and_then(crate::dates::parse_iso8601) {
                fields.insert(key.into(), timestamp.into());
            }
        }
        // A `url_transparent` result without a `url` fails in processing.
        let Some(media) = media else {
            return Err(Error::Reported("'url'".into()));
        };
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": media, "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.nts.live/shows/yu-su/episodes/yu-su-2nd-april-2024"
        ));
        assert!(suitable(
            "https://nts.live/shows/absolute-fiction/episodes/absolute-fiction-23rd-july-2022"
        ));
        assert!(!suitable("https://www.nts.live/shows/yu-su"));
    }
}
