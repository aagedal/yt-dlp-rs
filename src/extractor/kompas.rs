//! Upstream `KompasVideoIE` (`yt_dlp/extractor/kompas.py`): Kompas.com
//! videos through their Jixie ID (`JixieBaseIE`).
use super::{Extractor, download_webpage};
use crate::{Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct KompasVideo;

const NAME: &str = "KompasVideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://video\.kompas\.com/\w+/(?P<id>\d+)/(?P<slug>[\w-]+)")
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for KompasVideo {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found["slug"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        super::jixie::extract_from_jixie_id(
            client,
            url,
            (NAME, NAME),
            &display_id,
            &video_id,
            &webpage,
        )
        .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://video.kompas.com/watch/164474/kim-jong-un-siap-kirim-nuklir-lawan-as-dan-korsel"
        ));
        assert!(!suitable("https://video.kompas.com/watch/x/y"));
    }
}
