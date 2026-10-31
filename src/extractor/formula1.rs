//! Upstream `Formula1IE` (`yt_dlp/extractor/formula1.py`): the URL's
//! Brightcove ID as a `url` result of the Formula 1 Brightcove player.
use super::Extractor;
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct Formula1;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?formula1\.com/en/latest/video\.[^.]+\.(?P<id>\d+)\.html",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Formula1 {
    fn key(&self) -> &'static str {
        "Formula1"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let bc_id = &found["id"];
        crate::screen::set_extractor("Formula1");
        crate::screen::screen(&format!(
            "[Formula1] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `url_result(BRIGHTCOVE_URL_TEMPLATE % bc_id, 'BrightcoveNew', bc_id)`.
        let target = format!(
            "http://players.brightcove.net/6057949432001/S1WMrhjlh_default/index.html?videoId={bc_id}"
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
            "https://www.formula1.com/en/latest/video.race-highlights-spain-2016.6060988138001.html"
        ));
        assert!(!suitable("https://www.formula1.com/en/latest/video.html"));
    }
}
