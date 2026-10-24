//! Upstream `EuropeanTourIE` (`yt_dlp/extractor/europeantour.py`): the
//! page's `brightcove-player` video ID and `ACCOUNT_ID` as a `url` result of
//! that account's Brightcove player.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct EuropeanTour;

const NAME: &str = "EuropeanTour";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?europeantour\.com/dpworld-tour/news/video/(?P<id>[^/&?#$]+)")
        .unwrap()
});
static PLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)brightcove-player\s?video-id="([^"]+)".*"ACCOUNT_ID":"([^"]+)""#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for EuropeanTour {
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
        // `re.search(...).groups()` on a missing match.
        let player = PLAYER
            .captures(&webpage)
            .ok_or_else(|| Error::Reported("'NoneType' object has no attribute 'groups'".into()))?;
        // The account group cannot be empty, so upstream's fallback account
        // is never used.
        let target = format!(
            "http://players.brightcove.net/{}/default_default/index.html?videoId={}",
            &player[2], &player[1]
        );
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.europeantour.com/dpworld-tour/news/video/the-best-shots-of-the-2021-seasons/"
        ));
        assert!(!suitable("https://www.europeantour.com/dpworld-tour/news/"));
    }
}
