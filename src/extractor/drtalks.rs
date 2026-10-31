//! Upstream `DrTalksIE` (`yt_dlp/extractor/drtalks.py`): the page's
//! Next.js video as a `url_transparent` result of its Brightcove player
//! link, overlaid with the page's title, summary and featured image.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct DrTalks;

const NAME: &str = "DrTalks";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?drtalks\.com/videos/(?P<id>[\w-]+)").unwrap()
});

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

impl Extractor for DrTalks {
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
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        // `['props']['pageProps']['data']['video']`.
        let mut video = &next_data;
        for key in ["props", "pageProps", "data", "video"] {
            video = video.get(key).ok_or_else(|| key_error(&video_id, key))?;
        }
        let link = video
            .get("videos")
            .ok_or_else(|| key_error(&video_id, "videos"))?
            .get("brightcoveVideoLink")
            .ok_or_else(|| key_error(&video_id, "brightcoveVideoLink"))?
            .as_str()
            .ok_or_else(|| {
                Error::UnsupportedHtml5("DrTalks video links that are not strings".into())
            })?;
        let target = Url::parse(link)
            .map_err(|_| Error::UnsupportedHtml5("DrTalks video links that are not URLs".into()))?;
        if !super::brightcove::suitable(target.as_str()) {
            return Err(Error::UnsupportedHtml5(
                "DrTalks video links other than Brightcove players are not supported natively"
                    .into(),
            ));
        }
        let text = |path: &[&str]| {
            let mut value = video;
            for key in path {
                value = value.get(*key)?;
            }
            value.as_str().map(str::to_owned)
        };
        let title = text(&["title"]);
        let description = text(&["videos", "summury"]);
        let thumbnail = text(&["featuredImage", "node", "sourceUrl"])
            .and_then(|thumbnail| super::json_ld::url_or_none(&thumbnail));
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
        if let Some(description) = description {
            keys.push("description");
            info.metadata.description = Some(description);
        }
        if let Some(thumbnail) = thumbnail {
            keys.push("thumbnail");
            crate::metadata_parser::set_field(&mut info, "thumbnail", Value::from(thumbnail));
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
            "https://drtalks.com/videos/six-pillars-of-resilience-tools-for-managing-stress-and-flourishing/"
        ));
        assert!(!suitable("https://drtalks.com/videos/"));
    }
}
