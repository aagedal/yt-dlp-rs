//! Upstream `AliExpressLiveIE` (`yt_dlp/extractor/aliexpress.py`):
//! AliExpress live replays from the page's `runParams`: the replay HLS
//! playlist with the title, cover, host name and start time.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct AliExpressLive;

const NAME: &str = "AliExpressLive";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://live\.aliexpress\.com/live/(?P<id>\d+)").unwrap()
});
static RUN_PARAMS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)runParams\s*=\s*(\{.+?\})\s*;?\s*var").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for AliExpressLive {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
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
        let raw = RUN_PARAMS
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract runParams; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let data: Value = crate::json::from_str(&raw).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let title = data
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "title"))?;
        let stream_url = match data.get("replyStreamUrl") {
            Some(Value::String(stream_url)) => stream_url.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "AliExpress stream URLs that are not strings are not supported natively".into(),
                ));
            }
            None => return Err(key_error(&video_id, "replyStreamUrl")),
        };
        let manifest = Url::parse(&stream_url).map_err(|_| {
            Error::UnsupportedHtml5(
                "AliExpress stream URLs that are not URLs are not supported natively".into(),
            )
        })?;
        let stream =
            extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, Some("hls"), false)?;
        let uploader = data
            .pointer("/followBar/name")
            .filter(|name| name.is_string())
            .cloned()
            .unwrap_or(Value::Null);
        // `float_or_none(startTimeLong, scale=1000)`.
        let timestamp = match data.get("startTimeLong") {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
            _ => None,
        }
        .map_or(Value::Null, |value| json!(value / 1000.0));
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title),
                (
                    "thumbnail",
                    data.get("coverUrl").cloned().unwrap_or(Value::Null),
                ),
                ("uploader", uploader),
                ("timestamp", timestamp),
                ("formats", Value::Null),
            ],
            stream.formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://live.aliexpress.com/live/2800002704436634"
        ));
        assert!(!suitable("https://www.aliexpress.com/item/1.html"));
    }
}
