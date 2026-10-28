//! Upstream `BusinessInsiderIE` (`yt_dlp/extractor/businessinsider.py`):
//! Business Insider pages as `url` results of their JW Platform media.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct BusinessInsider;

const NAME: &str = "BusinessInsider";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:[^/]+\.)?businessinsider\.(?:com|nl)/(?:[^/]+/)*(?P<id>[^/?#&]+)")
        .unwrap()
});
static JWPLATFORM_IDS: LazyLock<[Regex; 4]> = LazyLock::new(|| {
    [
        Regex::new(r#"data-media-id=["']([a-zA-Z0-9]{8})"#).unwrap(),
        Regex::new(r#"id=["']jwplayer_([a-zA-Z0-9]{8})"#).unwrap(),
        Regex::new(r#"id["']?\s*:\s*["']?([a-zA-Z0-9]{8})"#).unwrap(),
        Regex::new(r"(?:jwplatform\.com/players/|jwplayer_)([a-zA-Z0-9]{8})").unwrap(),
    ]
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_regex` with several patterns: the first pattern that matches.
fn first_match(webpage: &str) -> Option<String> {
    JWPLATFORM_IDS
        .iter()
        .find_map(|pattern| pattern.captures(webpage))
        .map(|found| found[1].to_owned())
}

impl Extractor for BusinessInsider {
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
        let jwplatform_id = first_match(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract jwplatform id; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        // `url_result(f'jwplatform:{id}', JWPlatformIE.ie_key(), video_id)`.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_URL.into(),
            format!("jwplatform:{jwplatform_id}").into(),
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
            "http://uk.businessinsider.com/how-much-radiation-youre-exposed-to-in-everyday-life-2016-6"
        ));
        assert!(suitable(
            "https://www.businessinsider.nl/5-scientifically-proven-things-make-you-less-attractive-2017-7/"
        ));
        assert!(!suitable("https://www.businessinsider.de/x"));
    }
}
