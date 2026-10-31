//! Upstream `Liputan6IE` (`yt_dlp/extractor/liputan6.py`): an article's
//! `window.kmklabs.gtm` JSON names its Vidio video, which is followed as a
//! `vidio.com/watch/ID-DISPLAY_ID` URL (so `VidioIE` produces the result).
use super::clyp::key_error;
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Liputan6;

const NAME: &str = "Liputan6";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"https?://www\.liputan6\.com/\w+/read/\d+/(?P<id>[\w-]+)").unwrap()
});
static GTM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"window\.kmklabs\.gtm\s*=").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `str_or_none` of an ID (a number or a string).
fn id_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

impl Extractor for Liputan6 {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let display_id = found["id"].to_owned();
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let json_data = match super::search_json(&GTM, &webpage, false) {
            Some(Some(json_data)) => json_data,
            Some(None) => {
                return Err(Error::UnsupportedHtml5(
                    "Liputan6 gtm JSON that does not parse is not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract json_data; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let video_id = json_data
            .get("videos")
            .and_then(|videos| videos.get("video_1"))
            .and_then(|video| id_text(video.get("video_id")))
            .ok_or_else(|| key_error(&display_id, "video_id"))?;
        let target = format!("https://www.vidio.com/watch/{video_id}-{display_id}");
        Ok(Some(super::follow_url(url, target)))
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.liputan6.com/news/read/5007510/video-duh-perawat-rs-di-medan-diduga-salah-berikan-obat-ke-pasien"
        ));
        assert!(suitable(
            "https://www.liputan6.com/tv/read/5007719/video-program-minyakita"
        ));
        assert!(!suitable("https://www.liputan6.com/news/"));
    }
}
