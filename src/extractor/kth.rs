//! Upstream `KTHIE` (`yt_dlp/extractor/kth.py`): a `play.kth.se` media page
//! as a `url` result of `kaltura:308:ID`, smuggling the KTH (NORDUnet)
//! Kaltura service URL.
use super::Extractor;
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::json;
use std::sync::LazyLock;
use url::Url;

pub struct Kth;

const NAME: &str = "KTH";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://play\.kth\.se/(?:[^/]+/)+(?P<id>[a-z0-9_]+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Kth {
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
        let target = super::brightcove::smuggle(
            &format!("kaltura:308:{id}"),
            &json!({"service_url": "https://api.kaltura.nordu.net"}),
        );
        Ok(Some(super::kaltura::follow(url, target)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://play.kth.se/media/Lunch+breakA+De+nya+aff%C3%A4rerna+inom+Fordonsdalen/0_uoop6oz9"
        ));
        assert!(!suitable("https://play.kth.se/"));
    }
}
