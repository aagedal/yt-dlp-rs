//! Upstream `GMANetworkVideoIE` (`yt_dlp/extractor/gmanetwork.py`): a GMA
//! full-episode page's `YOUTUBE_VIDEO` variable, else its network content API
//! `video_file` (YouTube) or `dailymotion_file`, followed as `url` results.
use super::{Extractor, download_json, download_webpage, follow_url};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct GmaNetworkVideo;

const NAME: &str = "GMANetworkVideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www)\.gmanetwork\.com/(?:\w+/){3}(?P<id>\d+)/(?P<display_id>[\w-]+)/video",
    )
    .unwrap()
});
static YOUTUBE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s*YOUTUBE_VIDEO\s*=\s*['"]+(?P<yt_id>[\w-]+)"#).unwrap());
static NETWORK_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"NETWORK_URL\s*=\s*['"](?P<url>[^'"]+)"#).unwrap());

/// `_match_valid_url`'s ID (the first one, as upstream's `group('id')`).
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for GmaNetworkVideo {
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
        let content_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;

        // Webpage route: `_search_regex(..., fatal=False)`.
        if let Some(found) = YOUTUBE_ID.captures(&webpage) {
            return Ok(Some(follow_url(url, found["yt_id"].to_owned())));
        }
        super::search_warning(NAME, "youtube_id");

        // API call route (`fullepisodes_video.js`'s `NETWORK_URL`).
        let network = NETWORK_URL.captures(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract network_url; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let api = Url::parse(&format!(
            "{}api/data/content/video/{content_id}",
            &network["url"]
        ))?;
        let json = download_json(client, &api, NAME, &display_id)?;
        let truthy = |key: &str| {
            json.get(key)
                .filter(|value| super::nuxt::truthy(value))
                .and_then(|value| value.as_str().map(str::to_owned))
        };
        if let Some(video_file) = truthy("video_file") {
            return Ok(Some(follow_url(url, video_file)));
        }
        match json.get("dailymotion_file") {
            Some(serde_json::Value::String(target)) => Ok(Some(follow_url(url, target.to_owned()))),
            _ => Err(super::clyp::key_error(&display_id, "dailymotion_file")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.gmanetwork.com/fullepisodes/home/running_man_philippines/168677/running-man-philippines-catch-the-thief-full-chapter-2/video?section=home"
        ));
        assert!(!suitable("https://www.gmanetwork.com/news/"));
    }
}
