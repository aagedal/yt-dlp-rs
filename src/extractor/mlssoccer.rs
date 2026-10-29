//! Upstream `MLSSoccerIE` (`yt_dlp/extractor/mlssoccer.py`): MLS and club
//! video pages whose `data-options` attribute names the first Brightcove
//! video of the page, as a `url` result of its player.
use super::{Extractor, download_webpage, follow_url};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MlsSoccer;

const NAME: &str = "MLSSoccer";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:(?:cfmontreal|intermiamicf|lagalaxy|lafc|houstondynamofc|dcunited|atlutd|mlssoccer|fcdallas|columbuscrew|coloradorapids|fccincinnati|chicagofirefc|austinfc|nashvillesc|whitecapsfc|sportingkc|soundersfc|sjearthquakes|rsl|timbers|philadelphiaunion|orlandocitysc|newyorkredbulls|nycfc)\.com|(?:torontofc)\.ca|(?:revolutionsoccer)\.net)/video/#?(?P<id>[^/&$#?]+)",
    )
    .unwrap()
});
static OPTIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-options="([^"]+)""#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// A subscript value as Python's `str()` would show it.
fn py_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

impl Extractor for MlsSoccer {
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
        // `_html_search_regex(...)` (fatal, unescaped), then `_parse_json`.
        let Some(found) = OPTIONS.captures(&webpage) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract json; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let options: Value =
            serde_json::from_str(&crate::html::unescape_html(&found[1])).map_err(|_| {
                Error::UnsupportedHtml5("MLSSoccer data-options that do not parse as JSON".into())
            })?;
        let first = options
            .get("videoList")
            .and_then(Value::as_array)
            .and_then(|videos| videos.first())
            .ok_or_else(|| {
                Error::UnsupportedHtml5("MLSSoccer data-options without a videoList entry".into())
            })?;
        let account_id = py_text(first.get("accountId").ok_or_else(|| {
            Error::UnsupportedHtml5("MLSSoccer videoList entries without accountId".into())
        })?);
        let brightcove_id = py_text(first.get("videoId").ok_or_else(|| {
            Error::UnsupportedHtml5("MLSSoccer videoList entries without videoId".into())
        })?);
        let target = format!(
            "https://players.brightcove.net/{account_id}/default_default/index.html?videoId={brightcove_id}"
        );
        Ok(Some(follow_url(url, target)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.mlssoccer.com/video/the-octagon-can-alphonso-davies-lead-canada-to-first-world-cup-since-1986#the-octagon-can-alphonso-davies-lead-canada-to-first-world-cup-since-1986"
        ));
        assert!(suitable(
            "https://www.torontofc.ca/video/highlights-toronto-fc-vs-cf-montreal-october-23-2021-x6733"
        ));
        assert!(suitable(
            "https://www.sjearthquakes.com/video/#highlights-austin-fc-vs-san-jose-earthquakes-june-19-2021"
        ));
        assert!(!suitable("https://www.mlssoccer.com/"));
    }
}
