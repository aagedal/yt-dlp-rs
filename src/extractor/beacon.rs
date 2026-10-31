//! Upstream `BeaconTvIE` (`yt_dlp/extractor/beacon.py`): the next.js Apollo
//! state's `Content:` entry for the slug, its JSON-encoded JW Player video
//! (or podcast audio) data and the content's title, description and
//! publication time.
use super::{Extractor, download_webpage, search_nextjs_data};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct BeaconTv;

const NAME: &str = "BeaconTv";
const MEMBER_TIER: &str = "MemberTier:65b258d178f89be87b4dc0a4";
const LOGIN_HINT: &str = "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?beacon\.tv/content/(?P<id>[\w-]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn bug(id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `traverse_obj(value, (*path))`.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().try_fold(value, |value, key| value.get(key))
}

impl Extractor for BeaconTv {
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
        let next = search_nextjs_data(&webpage, NAME, &video_id)?;
        let content_data = path(&next, &["props", "pageProps", "__APOLLO_STATE__"])
            .and_then(Value::as_object)
            .and_then(|state| {
                state
                    .iter()
                    .find(|(key, value)| {
                        key.starts_with("Content:")
                            && value.get("slug").and_then(Value::as_str) == Some(&video_id)
                    })
                    .map(|(_, value)| value)
            })
            .filter(|value| super::nuxt::truthy(value))
            .ok_or_else(|| bug(&video_id, "Failed to extract content data"))?;
        let jwplayer_data = [
            ["contentVideo", "video", "videoData"],
            ["contentPodcast", "podcast", "audioData"],
        ]
        .iter()
        .filter_map(|keys| path(content_data, keys)?.as_str())
        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
        .find(Value::is_object);
        let Some(jwplayer_data) = jwplayer_data else {
            let content_type = content_data.get("contentType").and_then(Value::as_str);
            if !matches!(content_type, Some("videoPodcast" | "video" | "podcast")) {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Content is not a video/podcast"
                )));
            }
            if path(content_data, &["contentTier", "__ref"]).and_then(Value::as_str)
                != Some(MEMBER_TIER)
            {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: This video/podcast is for members only. {LOGIN_HINT}"
                )));
            }
            return Err(bug(&video_id, "Failed to extract content"));
        };
        let no_formats = || bug(&video_id, "No video formats found!");
        let Some(mut entry) = super::jwplayer::parse_required(&jwplayer_data, &video_id, url)?
        else {
            return Err(no_formats());
        };
        if let Some(title) = content_data.get("title").filter(|value| value.is_string()) {
            entry.insert("title".into(), title.clone());
        }
        if let Some(description) = content_data
            .get("description")
            .filter(|value| value.is_string())
        {
            entry.insert("description".into(), description.clone());
        }
        if let Some(timestamp) = content_data
            .get("publishedAt")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            entry.insert("timestamp".into(), timestamp.into());
        }
        let title_missing = entry.get("title").is_none_or(Value::is_null);
        let Some((mut info, keys)) =
            super::html5::jwplayer_entry_info(client, entry, url, &video_id, NAME)?
        else {
            return Err(no_formats());
        };
        if title_missing {
            info.title = String::new();
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let shape = format!("jwplatform;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://beacon.tv/content/welcome-to-beacon"));
        assert!(!suitable("https://beacon.tv/series/x"));
    }
}
