//! Upstream `NZHeraldIE` (`yt_dlp/extractor/nzherald.py`): articles whose
//! Brightcove player is embedded in the page, or whose Fusion metadata names
//! the video's Brightcove ID, as `url` results of the player.
use super::{Extractor, brightcove, download_webpage, follow_url};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct NzHerald;

const NAME: &str = "nzherald";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?nzherald\.co\.nz/[\w/-]+/(?P<id>[A-Z0-9]+)")
        .unwrap()
});
/// `BrightcoveNewIE._VALID_URL` as an `embedUrl` JSON-LD value.
static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"embedUrl"\s*:\s*"(https?://players\.brightcove\.net/\d+/[^/]+_[^/]+/index\.html\?.*(?:video|playlist)Id=(?:\d+|ref:[^&]+))"#,
    )
    .unwrap()
});
static FUSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Fusion\.globalContent\s*=\s*(\{.+?\})\s*;").unwrap());

/// `BRIGHTCOVE_URL_TEMPLATE`.
const BRIGHTCOVE_URL_TEMPLATE: &str =
    "http://players.brightcove.net/1308227299001/S1BXZn8t_default/index.html?videoId=";

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python's truthiness for the JSON values used here.
fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// The first `brightcoveId` (a string) or `content_elements[...].referent.id`
/// of a video or article object (`traverse_obj(..., get_all=False)`).
fn brightcove_video_id(base: &Value) -> Option<String> {
    if let Some(id) = base.get("brightcoveId").and_then(Value::as_str) {
        return Some(id.to_owned());
    }
    base.get("content_elements")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|element| {
            element
                .get("referent")?
                .get("id")?
                .as_str()
                .map(str::to_owned)
        })
}

impl Extractor for NzHerald {
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
        let article_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &article_id)?;
        // `_extract_bc_embed_url`: the page's player, else its `embedUrl`.
        let embedded = brightcove::brightcove_urls(&webpage)
            .into_iter()
            .next()
            .or_else(|| {
                EMBED_URL
                    .captures(&webpage)
                    .map(|found| found[1].to_owned())
            });
        if let Some(target) = embedded {
            return Ok(Some(follow_url(url, target)));
        }
        let Some(found) = FUSION.captures(&webpage) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {article_id}: Unable to extract fusion metadata; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let fusion: Value = serde_json::from_str(&found[1]).map_err(|_| {
            Error::UnsupportedHtml5("nzherald Fusion metadata that does not parse as JSON".into())
        })?;
        let video = fusion.get("video").cloned().unwrap_or(Value::Null);
        // `customVideo` embeds need a second API request, not supported here.
        if !truthy(&video)
            && fusion
                .pointer("/customVideo/embed/id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
        {
            return Err(Error::UnsupportedHtml5(
                "nzherald customVideo embeds (an extra content API request) are not supported natively".into(),
            ));
        }
        let base = if truthy(&video) { &video } else { &fusion };
        let Some(bc_video_id) = brightcove_video_id(base).filter(|id| !id.is_empty()) else {
            // An empty `video` object is an article without a video.
            if video.as_object().is_some_and(serde_json::Map::is_empty) {
                return Err(Error::Reported(format!(
                    "[{NAME}] {article_id}: This article does not have a video."
                )));
            }
            return Err(Error::Reported(format!(
                "[{NAME}] {article_id}: Failed to extract brightcove video id; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        Ok(Some(follow_url(
            url,
            format!("{BRIGHTCOVE_URL_TEMPLATE}{bc_video_id}"),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.nzherald.co.nz/nz/queen-elizabeth-death-nz-public-holiday-announced-for-september-26/CEOPBSXO2JDCLNK3H7E3BIE2FA/"
        ));
        assert!(suitable(
            "https://nzherald.co.nz/the-country/video/focus-nzs-first-mass-covid-19-vaccination-event/N5I7IL3BRFLZSD33TLDLYJDGK4/"
        ));
        assert!(suitable(
            "https://www.nzherald.co.nz/the-vision-is-clear/news/tvic-damian-roper-planting-trees-an-addiction/AN2AAEPNRK5VLISDWQAJZB6ATQ"
        ));
        assert!(!suitable("https://www.nzherald.co.nz/"));
    }
}
