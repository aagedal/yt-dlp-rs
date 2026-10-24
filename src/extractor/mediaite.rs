//! Upstream `MediaiteIE` (`yt_dlp/extractor/mediaite.py`): the article's JW
//! Player media ID and its `cdn.jwplayer.com/v2/media/ID` data, parsed like
//! `_parse_jwplayer_data` (the item's `mediaid` as the video ID).
use super::{Extractor, download_json, download_webpage_quiet};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Mediaite;

const NAME: &str = "Mediaite";

// `(?!/category)` as a check on the first path segment.
static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?mediaite\.com(?:/[\w-]+){2}").unwrap());
static CATEGORY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?mediaite\.com/category").unwrap());
static VIDEO_ID: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#""https://cdn\.jwplayer\.com/players/(\w+)"#).unwrap(),
        Regex::new(r#"data-video-id\s*=\s*"([^"]+)""#).unwrap(),
    ]
});

/// `_VALID_URL`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url) && !CATEGORY.is_match(url)
}

/// `video_data['mediaid']` of the (first) playlist item.
fn media_id(data: &Value) -> Option<String> {
    let item = match data.get("playlist") {
        Some(Value::Array(items)) => items.iter().find(|item| item.is_object())?,
        Some(item) if item.is_object() => item,
        _ => data,
    };
    item.get("mediaid")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

impl Extractor for Mediaite {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_download_webpage(url, None)`.
        crate::screen::screen(&format!("[{NAME}] Downloading webpage"));
        let webpage = download_webpage_quiet(client, url, NAME, "None")?;
        let page_id = VIDEO_ID
            .iter()
            .find_map(|regex| regex.captures(&webpage))
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] Unable to extract id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let api = Url::parse(&format!("https://cdn.jwplayer.com/v2/media/{page_id}"))?;
        let data = download_json(client, &api, NAME, &page_id)?;
        let id = media_id(&data).ok_or_else(|| {
            Error::UnsupportedHtml5("Mediaite JW Player data without a string mediaid".into())
        })?;
        let no_formats = || {
            Error::Reported(format!(
                "[{NAME}] {id}: No video formats found!; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let Some(entry) = super::jwplayer::parse_required(&data, &id, &api)? else {
            return Err(no_formats());
        };
        let title_missing = entry.get("title").is_none_or(Value::is_null);
        let Some((mut info, keys)) =
            super::html5::jwplayer_entry_info(client, entry, url, &id, NAME)?
        else {
            return Err(no_formats());
        };
        if title_missing {
            info.title = String::new();
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let shape = format!("jwplatform;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.mediaite.com/sports/bill-burr-roasts-nfl-for-promoting-black-lives-matter/"
        ));
        assert!(suitable(
            "https://www.mediaite.com/politics/i-cant-read-it-fast-enough-while-defending-trump/"
        ));
        assert!(!suitable("https://www.mediaite.com/category/tv/"));
        assert!(!suitable("https://www.mediaite.com/tv"));
    }
}
