//! Upstream `MaoriTVIE` (`yt_dlp/extractor/maoritv.py`): the page's
//! `data-main-video-id` as a `url` result of its Brightcove player.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct MaoriTV;

const NAME: &str = "MaoriTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?maoritelevision\.com/shows/(?:[^/]+/)+(?P<id>[^/?&#]+)",
    )
    .unwrap()
});
static BRIGHTCOVE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-main-video-id=["'](\d+)"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for MaoriTV {
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
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let brightcove_id = BRIGHTCOVE_ID
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract brightcove id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // `url_result(BRIGHTCOVE_URL_TEMPLATE % brightcove_id, 'BrightcoveNew', brightcove_id)`.
        let target = format!(
            "http://players.brightcove.net/1614493167001/HJlhIQhQf_default/index.html?videoId={brightcove_id}"
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
            "https://www.maoritelevision.com/shows/korero-mai/S01E054/korero-mai-series-1-episode-54"
        ));
        assert!(!suitable("https://www.maoritelevision.com/shows/"));
    }
}
