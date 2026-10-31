//! Upstream `WimbledonIE` (`yt_dlp/extractor/wimbledon.py`): the video as a
//! `url_transparent` result of the Wimbledon Brightcove player, overlaid
//! with the related-content API's title, description and duration.
use super::{Extractor, download_json};
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Wimbledon;

const NAME: &str = "Wimbledon";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?wimbledon\.com/\w+/video/media/(?P<id>\d+)\.html",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Wimbledon {
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
        let api = Url::parse(&format!(
            "https://www.wimbledon.com/relatedcontent/rest/v2/wim_v1/en/content/wim_v1_{video_id}_en"
        ))?;
        let metadata = download_json(client, &api, NAME, &video_id)?;
        // `traverse_obj(metadata, {...})`: the non-`None` values.
        let mut fields = Map::new();
        for key in ["title", "description"] {
            if let Some(value) = metadata.get(key).filter(|value| !value.is_null()) {
                fields.insert(key.into(), value.clone());
            }
        }
        // `('metadata', 'duration', {parse_duration})`: a string's duration.
        if let Some(duration) = metadata
            .get("metadata")
            .and_then(|metadata| metadata.get("duration"))
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
        {
            let duration = match duration {
                crate::info::PyNumber::Int(value) => Value::from(value),
                crate::info::PyNumber::Float(value) => Value::from(value),
            };
            fields.insert("duration".into(), duration);
        }
        if fields.get("title").is_some_and(|title| !title.is_string())
            || fields
                .get("description")
                .is_some_and(|description| !description.is_string())
        {
            return Err(crate::Error::UnsupportedHtml5(
                "Wimbledon metadata with a non-string title or description".into(),
            ));
        }
        let target = format!(
            "http://players.brightcove.net/3506358525001/default_default/index.html?videoId={video_id}"
        );
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.wimbledon.com/en_GB/video/media/6330247525112.html"
        ));
        assert!(suitable(
            "https://www.wimbledon.com/es_Es/video/media/6308377909112.html"
        ));
        assert!(!suitable("https://www.wimbledon.com/en_GB/video/media/"));
    }
}
