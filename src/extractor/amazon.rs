//! Upstream `AmazonStoreIE` (`yt_dlp/extractor/amazon.py`): a product page's
//! `var obj = jQuery.parseJSON('...')` videos as a playlist of direct HLS
//! entries (`AmazonReviewsIE` is not ported: its review pages no longer carry
//! the review body markup it reads).
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct AmazonStore;

const NAME: &str = "AmazonStore";
/// `RetryManager()`'s default `retries=3`: four attempts in total.
const RETRIES: u32 = 3;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?amazon\.(?:[a-z]{2,3})(?:\.[a-z]{2})?/(?:[^/]+/)?(?:dp|gp/product)/(?P<id>[^/&#$?]+)")
        .unwrap()
});
static DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s?obj\s?=\s?jQuery\.parseJSON\('").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `int_or_none` for the `videoHeight`/`videoWidth` strings.
fn int_value(value: Option<&Value>) -> Option<u32> {
    match value? {
        Value::Number(number) => number.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// One `videos` entry as a direct HLS result (`{'url': ..., ...}`).
fn video_entry(page: &Url, video: &Value, id: &str, url: &str) -> Result<VideoInfo> {
    let video_url = Url::parse(url)?;
    let mut entry = super::direct::build_at(video_url.clone(), &video_url, "m3u8".into());
    entry.id = id.to_owned();
    entry.title = video
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    entry.webpage_url = page.clone();
    entry.extractor = NAME.into();
    entry.extractor_key = NAME.into();
    entry.metadata.thumbnail = video
        .get("thumbUrl")
        .or_else(|| video.get("thumb"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    entry.metadata.duration = match video.get("durationSeconds") {
        Some(Value::Number(number)) => number.as_i64().map(crate::info::PyNumber::Int),
        _ => None,
    };
    entry.height = int_value(video.get("videoHeight"));
    entry.width = int_value(video.get("videoWidth"));
    entry.direct = None;
    Ok(entry)
}

impl Extractor for AmazonStore {
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
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let mut data = None;
        for attempt in 1..=RETRIES + 1 {
            let webpage = download_webpage(client, url, NAME, &playlist_id)?;
            if let Some(Some(value)) = super::search_json(&DATA, &webpage, true) {
                data = Some(value);
                break;
            }
            if attempt <= RETRIES {
                crate::screen::warning(&format!(
                    "[{NAME}] Unable to extract data. Retrying ({attempt}/{RETRIES})..."
                ));
            }
        }
        let Some(data) = data else {
            return Err(Error::Reported(format!(
                "[{NAME}] {playlist_id}: Unable to extract data; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };

        let mut entries = Vec::new();
        let videos = data
            .get("videos")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for video in videos {
            let is_video = video.get("isVideo").is_some_and(super::nuxt::truthy);
            let Some(video_url) = video
                .get("url")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            else {
                if is_video {
                    return Err(key_error(&playlist_id, "url"));
                }
                continue;
            };
            if !is_video {
                continue;
            }
            let market_id = video
                .get("marketPlaceID")
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(&playlist_id, "marketPlaceID"))?;
            entries.push(video_entry(url, &video, market_id, video_url)?);
        }

        let title = data.get("title").and_then(Value::as_str).map(str::to_owned);
        let keys = if title.is_some() { "id,title" } else { "id" };
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.amazon.co.uk/dp/B098XNCHLD/"));
        assert!(suitable(
            "https://www.amazon.es/Samsung-Smartphone-s-AMOLED-Quad-c%C3%A1mara-espa%C3%B1ola/dp/B08WX337PQ"
        ));
        assert!(!suitable("https://www.amazon.co.uk/"));
    }
}
