//! Upstream `FreespeechIE` (`yt_dlp/extractor/freespeech.py`): Freespeech
//! stories as `url` results of their `data-video-url` YouTube video.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct Freespeech;

const NAME: &str = "freespeech.org";
const KEY: &str = "Freespeech";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?freespeech\.org/stories/(?P<id>.+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Freespeech {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = super::download_webpage(client, url, NAME, &id)?;
        static YOUTUBE_URL: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"data-video-url="([^"]+)""#).unwrap());
        let youtube_url = YOUTUBE_URL
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract youtube url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        Ok(Some(super::follow_url(url, youtube_url)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.freespeech.org/stories/fcc-announces-net-neutrality-rollback-whats-stake/"
        ));
        assert!(!suitable("http://www.freespeech.org/"));
    }
}
