//! Upstream `OneFootballIE` (`yt_dlp/extractor/onefootball.py`): the page's
//! `cdn.jwplayer.com/manifests/ID.m3u8` URL as a `url_transparent` result of
//! `JWPlatformIE`, overlaid with the old archive ID and the page's JSON-LD.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct OneFootball;

const NAME: &str = "OneFootball";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?onefootball\.com/[a-z]{2}/video/[^/&?#]+-(?P<id>\d+)",
    )
    .unwrap()
});

static M3U8: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(https://cdn\.jwplayer\.com/manifests/\w+\.m3u8)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for OneFootball {
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
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        // `_search_json_ld(webpage, video_id, fatal=False)`.
        let mut json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract JSON-LD ; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        json_ld.shift_remove("url");
        let manifest = M3U8
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract m3u8_url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // `url_result(m3u8_url, JWPlatformIE, video_id, _old_archive_ids=[...],
        // **data_json, url_transparent=True)`: the fields other than `_type`,
        // `url`, `ie_key` and `id`.
        let mut fields = Map::new();
        fields.insert(
            "_old_archive_ids".into(),
            json!([format!("onefootball {id}")]),
        );
        for (key, value) in json_ld {
            if matches!(
                key.as_str(),
                "_type" | "url" | "id" | "ie_key" | "extractor" | "extractor_key"
            ) {
                continue;
            }
            fields.insert(key, value);
        }
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": manifest, "fields": Value::Object(fields)}),
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
            "https://onefootball.com/en/video/highlights-fc-zuerich-3-3-fc-basel-34012334"
        ));
        assert!(suitable(
            "https://www.onefootball.com/de/video/klopp-fumes-at-var-decisions-34041020"
        ));
        assert!(!suitable(
            "https://onefootball.com/en/news/some-story-34012334"
        ));
    }
}
