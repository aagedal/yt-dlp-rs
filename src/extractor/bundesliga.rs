//! Upstream `BundesligaIE` (`yt_dlp/extractor/bundesliga.py`): the `vid`
//! query's JW Player media as a `url` result of `jwplatform:ID`.
use super::Extractor;
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct Bundesliga;

const NAME: &str = "Bundesliga";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?bundesliga\.com/[a-z]{2}/bundesliga/videos(?:/[^?]+)?\?vid=(?P<id>[a-zA-Z0-9]{8})",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Bundesliga {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = &found["id"];
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `url_result(f'jwplatform:{video_id}', JWPlatformIE, video_id)`.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow
            .extra
            .insert(crate::FOLLOW_URL.into(), format!("jwplatform:{id}").into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.bundesliga.com/en/bundesliga/videos?vid=bhhHkKyN"
        ));
        assert!(suitable(
            "https://www.bundesliga.com/en/bundesliga/videos/latest-features/T8IKc8TX?vid=ROHjs06G"
        ));
        assert!(!suitable("https://www.bundesliga.com/en/bundesliga/videos"));
    }
}
