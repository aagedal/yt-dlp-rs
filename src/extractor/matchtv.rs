//! Upstream `MatchTVIE` (`yt_dlp/extractor/matchtv.py`): the Match TV live
//! channel's iframe `data-config` feed as a live HLS master.
use super::{Extractor, download_webpage, extract_m3u8_fatal_live};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MatchTv;

const NAME: &str = "MatchTV";
const ID: &str = "matchtv-live";
const IFRAME: &str = "https://video.matchtv.ru/iframe/channel/106";

static VALID_URL: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"^https?://matchtv\.ru/on-air/?(?:$|[?#])").unwrap(),
        Regex::new(r"^https?://video\.matchtv\.ru/iframe/channel/106/?(?:$|[?#])").unwrap(),
    ]
});
static CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-config="config=(https?://[^?"]+)[?"]"#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.iter().any(|regex| regex.is_match(url))
}

impl Extractor for MatchTv {
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
        let webpage = download_webpage(client, &Url::parse(IFRAME)?, NAME, ID)?;
        // `_html_search_regex(...)`, then `/feed/` -> `/media/` plus `.m3u8`.
        let video_url = CONFIG
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {ID}: Unable to extract video URL; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .replace("/feed/", "/media/")
            + ".m3u8";
        let manifest = Url::parse(&video_url)?;
        let mut info = extract_m3u8_fatal_live(client, &manifest, NAME, ID, None, false)?;
        info.id = ID.into();
        info.title = "Матч ТВ - Прямой эфир".into();
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(true));
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,is_live,formats",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://matchtv.ru/on-air/"));
        assert!(suitable("https://matchtv.ru/on-air?x=1"));
        assert!(suitable("https://video.matchtv.ru/iframe/channel/106"));
        assert!(!suitable("https://video.matchtv.ru/iframe/channel/107"));
        assert!(!suitable("https://matchtv.ru/on-air/foo"));
    }
}
