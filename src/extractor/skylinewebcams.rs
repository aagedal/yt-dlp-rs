//! Upstream `SkylineWebcamsIE` (`yt_dlp/extractor/skylinewebcams.py`): a
//! webcam page's live `m3u8` stream URL as the result's own format, with
//! its OpenGraph title and description.
use super::{Extractor, broken_warning, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct SkylineWebcams;

const NAME: &str = "SkylineWebcams";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?skylinewebcams\.com/[^/]+/webcam/(?:[^/]+/)+(?P<id>[^/]+)\.html",
    )
    .unwrap()
});
/// `(?:url|source)\s*:\s*(["\'])(?P<url>(?:https?:)?//.+?\.m3u8.*?)\1` as
/// one alternative per quote.
static STREAM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:url|source)\s*:\s*(?:"(?P<a>(?:https?:)?//.+?\.m3u8.*?)"|'(?P<b>(?:https?:)?//.+?\.m3u8.*?)')"#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for SkylineWebcams {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        broken_warning();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        // `_search_regex(..., 'stream url', group='url')` (fatal).
        let stream_url = STREAM_URL
            .captures(&webpage)
            .and_then(|found| found.name("a").or_else(|| found.name("b")))
            .map(|found| found.as_str().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract stream url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let search = |property: &str, name: &str| {
            let value = crate::html::og_search_property(&webpage, property);
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let title = search("title", "OpenGraph title");
        let description = search("description", "OpenGraph description");
        // Processing's `sanitize_url` gives a protocol-relative URL `http:`.
        let media = Url::parse(&super::proto_relative_http(&stream_url))?;
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp4".into()),
            "",
        );
        for format in &mut info.formats {
            format.ext = "mp4".into();
            format.format_id = String::new();
            // `determine_protocol`: a live `.m3u8` URL is `m3u8`.
            format.protocol = Some("m3u8".into());
        }
        info.format_id = None;
        info.id = id;
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(
            &mut info,
            "description",
            description.map_or(Value::Null, Value::String),
        );
        crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(true));
        Ok(Some(crate::info_order::mark_info(
            info,
            "untyped-dict;id,url,ext,title,description,is_live",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.skylinewebcams.com/it/webcam/italia/lazio/roma/scalinata-piazza-di-spagna-barcaccia.html"
        ));
        assert!(!suitable("https://www.skylinewebcams.com/it/webcam/x.html"));
    }

    #[test]
    fn stream_urls_follow_upstream_pattern() {
        let found = STREAM_URL
            .captures(r#"source:'//a.b/live.m3u8?x=1',parentId:"#)
            .unwrap();
        assert_eq!(&found["b"], "//a.b/live.m3u8?x=1");
    }
}
