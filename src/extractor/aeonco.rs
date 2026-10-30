//! Upstream `AeonCoIE` (`yt_dlp/extractor/aeonco.py`): an Aeon video page's
//! JSON-LD `VideoObject` embed URL, followed (a Vimeo embed carries the page
//! as its `referer`, smuggled into the URL).
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct AeonCo;

const NAME: &str = "AeonCo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?aeon\.co/videos/(?P<id>[^/?]+)").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for AeonCo {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<crate::ExtractResult>> {
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
        // `traverse_obj(_yield_json_ld(...), (lambda: @type == VideoObject,
        // 'embedUrl', {url_or_none}), get_all=False)`.
        let embed_url = super::json_ld::yield_json_ld(&webpage)
            .iter()
            .filter(|value| value.get("@type").and_then(Value::as_str) == Some("VideoObject"))
            .find_map(|value| {
                value
                    .get("embedUrl")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
            })
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: No embed URL found in webpage; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let target = if embed_url.contains("player.vimeo.com") {
            // `VimeoIE._smuggle_referrer`.
            super::brightcove::smuggle(&embed_url, &json!({"referer": url.as_str()}))
        } else {
            embed_url
        };
        Ok(Some(super::follow_url(url, target)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://aeon.co/videos/raw-solar-storm-footage-is-the-punk-rock-antidote-to-sleek-james-webb-imagery"
        ));
        assert!(!suitable("https://aeon.co/essays/some-essay"));
    }
}
