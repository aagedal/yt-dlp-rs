//! Upstream `MyVidsterIE` (`yt_dlp/extractor/myvidster.py`): the page's
//! `rel="videolink"` URL as a `url_result` followed by the registry.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct MyVidster;

const NAME: &str = "MyVidster";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?myvidster\.com/video/(?P<id>\d+)").unwrap());
static VIDEO_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"rel="videolink" href="(?P<real_url>.*)">"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for MyVidster {
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
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        // `_html_search_regex(..., 'real video url')` (fatal).
        let target = VIDEO_LINK
            .captures(&webpage)
            .map(|found| {
                crate::html::clean_html(&found["real_url"])
                    .trim()
                    .to_owned()
            })
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract real video url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let target = Url::parse(&target).map_err(|_| {
            Error::UnsupportedHtml5(format!(
                "MyVidster video links that are not absolute URLs are not supported natively: {target}"
            ))
        })?;
        // `url_result(url)`, followed by the registry.
        let mut info = super::direct::build_at(target.clone(), &target, String::new());
        info.formats.clear();
        info.direct = None;
        info.extra
            .insert(crate::FOLLOW_URL.into(), target.as_str().into());
        Ok(Some(ExtractResult::Video(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.myvidster.com/video/32059805/Hot_chemistry_with_raw_love_making"
        ));
        assert!(!suitable("http://www.myvidster.com/user/32059805"));
    }
}
