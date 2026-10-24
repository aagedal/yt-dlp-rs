//! Upstream `SpiegelIE` (`yt_dlp/extractor/spiegel.py`): the page's
//! `mediaId` as a `url_transparent` result of its `jwplatform:` video,
//! overlaid with the article ID and `og:title`.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Spiegel;

const NAME: &str = "Spiegel";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?(?:spiegel|manager-magazin)\.de(?:/[^/]+)+/[^/]*-(?P<id>[0-9]+|[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12})(?:-embed|-iframe)?(?:\.html)?(?:$|[#?])").unwrap()
});

/// `(&#34;|["\'])mediaId\1\s*:\s*(&#34;|["\'])(?P<id>(?:(?!\2).)+)\2`, with
/// the backreferences spelled out per quote pair.
static MEDIA_ID: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    let quotes = ["&#34;", "\"", "'"];
    let mut patterns = Vec::new();
    for first in quotes {
        for second in quotes {
            let body = if second.len() == 1 {
                format!("[^{}\\n]+", regex::escape(second))
            } else {
                // `(?:(?!&#34;).)+`: no `&#34;` inside (and no newline).
                "(?:[^&\\n]|&(?:[^#\\n]|#(?:[^3\\n]|3(?:[^4\\n]|4[^;\\n]))))+".to_owned()
            };
            patterns.push(
                Regex::new(&format!(
                    r"{f}mediaId{f}\s*:\s*{s}(?P<id>{body}){s}",
                    f = regex::escape(first),
                    s = regex::escape(second),
                ))
                .unwrap(),
            );
        }
    }
    patterns
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// The leftmost (then first-alternative) `mediaId` match.
fn media_id(webpage: &str) -> Option<String> {
    MEDIA_ID
        .iter()
        .filter_map(|pattern| pattern.captures(webpage))
        .min_by_key(|found| found.get(0).map_or(usize::MAX, |whole| whole.start()))
        .map(|found| found["id"].to_owned())
}

impl Extractor for Spiegel {
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
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `_html_search_regex`.
        let media = media_id(&webpage)
            .map(|found| crate::html::clean_html(&found).trim().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract media id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let title = crate::html::og_search_property(&webpage, "title");
        let target = Url::parse(&format!("jwplatform:{media}"))?;
        let Some(ExtractResult::Video(mut info)) =
            super::jwplatform::JwPlatform.extract_result(client, &target)?
        else {
            return Ok(None);
        };
        // `url_transparent`: the non-`None` fields other than `_type`, `url`,
        // `id` and `ie_key`.
        let mut keys = vec!["display_id"];
        crate::metadata_parser::set_field(&mut info, "display_id", Value::from(video_id));
        if let Some(title) = title {
            keys.push("title");
            info.title = title;
        }
        info.webpage_url = url.clone();
        let inner_shape = match info.extra.get(crate::info_order::INFO_SHAPE) {
            Some(Value::String(shape)) => shape.clone(),
            _ => String::new(),
        };
        let shape = format!("{inner_shape}|transparent:{}", keys.join(","));
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info, &shape,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.spiegel.de/video/vulkan-tungurahua-in-ecuador-ist-wieder-aktiv-video-1259285.html"
        ));
        assert!(suitable(
            "https://www.spiegel.de/panorama/urteile-im-goldmuenzenprozess-haftstrafen-fuer-clanmitglieder-a-aae8df48-43c1-4c61-867d-23f0a2d254b7"
        ));
        assert!(suitable(
            "http://www.spiegel.de/sport/sonst/badminton-wm-die-randsportart-soll-populaerer-werden-a-987092.html"
        ));
        assert!(!suitable("https://www.spiegel.de/video/"));
    }

    #[test]
    fn media_id_follows_upstream_regex() {
        assert_eq!(
            media_id(r#"x &#34;mediaId&#34;:&#34;abc&#34; "mediaId": "zzz""#).as_deref(),
            Some("abc")
        );
        assert_eq!(
            media_id(r#"{'mediaId': "II0BUyxY"}"#).as_deref(),
            Some("II0BUyxY")
        );
        assert_eq!(media_id("mediaId: 1"), None);
    }
}
