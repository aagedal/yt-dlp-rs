//! Upstream `TeleBruxellesIE` (`yt_dlp/extractor/telebruxelles.py`): BX1
//! (formerly Télé Bruxelles) articles and live streams from the page's
//! player `file` RTMP URL, as Wowza formats, with the article ID, the first
//! `<h1>` (else Open Graph) title and the Open Graph description.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct TeleBruxelles;

const NAME: &str = "TeleBruxelles";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?(?:telebruxelles|bx1)\.be/(?:[^/]+/)*(?P<id>[^/#?]+)").unwrap()
});
static ARTICLE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<article[^>]+\bid=["']post-(\d+)"#).unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<h1[^>]*>(.+?)</h1>").unwrap());
static RTMP_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"file["']?\s*:\s*"(r(?:tm|mt)ps?://[^/]+/(?:vod/mp4:"\s*\+\s*"[^"]+"\s*\+\s*"\.mp4|stream/live))""#,
    )
    .unwrap()
});
static RMTP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^rmtp").unwrap());
static CONCAT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""\s*\+\s*""#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_html_search_regex`: the cleaned first group.
fn html_search(regex: &Regex, webpage: &str) -> Option<String> {
    regex
        .captures(webpage)
        .map(|found| crate::html::clean_html(&found[1]))
}

impl Extractor for TeleBruxelles {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
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
        let article_id = html_search(&ARTICLE_ID, &webpage).filter(|id| !id.is_empty());
        let title = html_search(&TITLE, &webpage)
            .filter(|title| !title.is_empty())
            .or_else(|| {
                let title = crate::html::og_search_property(&webpage, "title");
                if title.is_none() {
                    super::search_warning(NAME, "OpenGraph title");
                }
                title
            });
        let description = crate::html::og_search_property(&webpage, "description");
        let rtmp_url = html_search(&RTMP_URL, &webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract RTMP url; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        // The live stream URLs' scheme is misspelled `rmtp`.
        let rtmp_url = RMTP.replace(&rtmp_url, "rtmp");
        let rtmp_url = CONCAT.replace_all(&rtmp_url, "").into_owned();
        let id = article_id.unwrap_or_else(|| display_id.clone());
        let formats = super::extract_wowza_formats(client, &rtmp_url, NAME, &id, &[])?;
        let is_live = rtmp_url.contains("stream/live");
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", id.into()),
                ("display_id", display_id.into()),
                ("title", optional(title)),
                ("description", optional(description)),
                ("formats", Value::Null),
                ("is_live", is_live.into()),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://bx1.be/news/que-risque-lauteur-dune-fausse-alerte-a-la-bombe/"
        ));
        assert!(suitable("https://bx1.be/lives/direct-tv/"));
        assert!(suitable("http://www.telebruxelles.be/dernier-jt"));
        assert!(!suitable("https://bx1.be/"));
        assert!(!suitable("https://example.be/news/x"));
    }

    #[test]
    fn rtmp_urls_join_their_concatenation() {
        let page = r#"file: "rtmp://bx1.test/vod/mp4:" + "2016/a" + ".mp4""#;
        let found = html_search(&RTMP_URL, page).unwrap();
        assert_eq!(
            CONCAT.replace_all(&found, ""),
            "rtmp://bx1.test/vod/mp4:2016/a.mp4"
        );
    }
}
