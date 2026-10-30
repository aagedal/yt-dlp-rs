//! Upstream `SkyNewsAUIE` (`yt_dlp/extractor/skynewsau.py`): the page's
//! `embedcode` (`ACCOUNT-VIDEO`) as a `url_transparent` result of that
//! account's Brightcove player, overlaid with the news API's caption and
//! creation date.
use super::{Extractor, download_json, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct SkyNewsAU;

const NAME: &str = "SkyNewsAU";
const API_KEY: &str = "6krsj3w249nk779d8fukqx9f";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?skynews\.com\.au/[^/]+/[^/]+/[^/]+/video/(?P<id>[a-z0-9]+)",
    )
    .unwrap()
});
static EMBEDCODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"embedcode\s?=\s?"([^"]+)""#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for SkyNewsAU {
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
        let embedcode = EMBEDCODE
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract embedcode; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let api = Url::parse(&format!(
            "https://content.api.news/v3/videos/brightcove/{embedcode}?api_key={API_KEY}"
        ))?;
        let response = download_json(client, &api, NAME, &video_id)?;
        let data = response.get("content").ok_or_else(|| {
            Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('content')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let parts: Vec<&str> = embedcode.split('-').collect();
        if parts.len() < 2 {
            return Err(Error::UnsupportedHtml5(
                "SkyNewsAU embed codes without an account and video ID".into(),
            ));
        }
        let target = Url::parse(&format!(
            "https://players.brightcove.net/{}/default_default/index.html?videoId={}",
            parts[0], parts[1]
        ))
        .map_err(|_| Error::UnsupportedHtml5("SkyNewsAU embed codes that are not URLs".into()))?;
        let title = data
            .get("caption")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let upload_date = data
            .get("date")
            .and_then(|date| date.get("created"))
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_strdate(date, true));
        let Some(mut info) = super::brightcove::BrightcoveNew.extract(client, &target)? else {
            return Ok(None);
        };
        // `url_transparent`: the non-`None` fields other than `_type`, `url`,
        // `id` and `ie_key`.
        let mut keys = Vec::new();
        if let Some(title) = title {
            keys.push("title");
            info.title = title;
            info.extra.shift_remove(crate::info::TITLE_MISSING);
        }
        if let Some(upload_date) = upload_date {
            keys.push("upload_date");
            crate::metadata_parser::set_field(&mut info, "upload_date", Value::from(upload_date));
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
            "https://www.skynews.com.au/world-news/united-states/incredible-vision-shows-lava-overflowing-from-spains-la-palma-volcano/video/0f4c6243d6903502c01251f228b91a71"
        ));
        assert!(!suitable("https://www.skynews.com.au/world-news/video/abc"));
    }
}
