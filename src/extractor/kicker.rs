//! Upstream `KickerIE` (`yt_dlp/extractor/kicker.py`): the page's private
//! Dailymotion ID as a `url` result for the Dailymotion extractor.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct Kicker;

const NAME: &str = "Kicker";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)kicker\.(?:de)/(?P<id>[\w-]+)/video").unwrap()
});
static PRIVATE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-dmprivateid\s*=\s*['"](?P<video_id>\w+)"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Kicker {
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
        let slug = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &slug)?;
        let id = PRIVATE_ID
            .captures(&webpage)
            .map(|found| found["video_id"].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {slug}: Unable to extract video id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // `video_title=self._html_extract_title(webpage)` (non-fatal); a
        // `url` result's title is not carried over to the Dailymotion result.
        if crate::html::html_extract_title(&webpage).is_none() {
            super::search_warning(NAME, "title");
        }
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_URL.into(),
            format!("https://www.dailymotion.com/video/{id}").into(),
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
            "https://www.kicker.de/pogba-dembel-co-die-top-11-der-abloesefreien-spieler-905049/video"
        ));
        assert!(suitable(
            "https://www.kicker.de/ex-unioner-in-der-bezirksliga-felix-kroos-vereinschallenge-in-pankow-902825/video"
        ));
        assert!(!suitable("https://kicker.de/x/video"));
        assert!(!suitable("https://www.kicker.de/x/news"));
    }
}
