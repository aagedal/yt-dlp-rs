//! Upstream `NineNewsIE` (`yt_dlp/extractor/ninenews.py`): the page's
//! `__INITIAL_STATE__` Brightcove video ID and account as a `url` result of
//! that account's default Brightcove player.
use super::{Extractor, download_webpage, search_json};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct NineNews;

const NAME: &str = "9News";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?9news\.com\.au/(?:[\w-]+/){2,3}(?P<id>[\w-]+)/?(?:$|[?#])")
        .unwrap()
});
static INITIAL_STATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+__INITIAL_STATE__\s*=\s*").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python `...` over a dict's values or a list's items.
fn branch(value: &Value) -> Vec<&Value> {
    match value {
        Value::Object(map) => map.values().collect(),
        Value::Array(items) => items.iter().collect(),
        _ => Vec::new(),
    }
}

/// `traverse_obj(state, ('videoIndex', 'currentVideo', 'brightcoveId',
/// {str}), ('article', ..., 'media', lambda _, v: v['type'] == 'video',
/// 'urn', {str}), get_all=False)`.
fn video_id(state: &Value) -> Option<String> {
    if let Some(id) = state
        .pointer("/videoIndex/currentVideo/brightcoveId")
        .and_then(Value::as_str)
    {
        return Some(id.to_owned());
    }
    let article = state.get("article")?;
    for item in branch(article) {
        let Some(media) = item.get("media") else {
            continue;
        };
        for entry in branch(media) {
            if entry.get("type").and_then(Value::as_str) != Some("video") {
                continue;
            }
            if let Some(urn) = entry.get("urn").and_then(Value::as_str) {
                return Some(urn.to_owned());
            }
        }
    }
    None
}

/// `traverse_obj(state, ('videoIndex', 'config', (None, 'video'), 'account',
/// {str}), get_all=False)`.
fn account(state: &Value) -> Option<String> {
    [
        "/videoIndex/config/account",
        "/videoIndex/config/video/account",
    ]
    .iter()
    .find_map(|pointer| state.pointer(pointer).and_then(Value::as_str))
    .map(str::to_owned)
}

impl Extractor for NineNews {
    fn key(&self) -> &'static str {
        "NineNews"
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
        let state = match search_json(&INITIAL_STATE, &webpage, false) {
            Some(Some(state)) => state,
            Some(None) => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {article_id}: Unable to extract initial state - Failed to parse JSON"
                )));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {article_id}: Unable to extract initial state; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let (Some(video_id), Some(account)) = (video_id(&state), account(&state)) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {article_id}: Unable to get the required video data; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let target = format!(
            "https://players.brightcove.net/{account}/default_default/index.html?videoId={video_id}"
        );
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.9news.com.au/videos/national/fair-trading-pulls-dozens-of-toys-from-shelves/clqgc7dvj000y0jnvfism0w5m"
        ));
        assert!(suitable(
            "https://www.9news.com.au/national/outrage-as-parents-banned-from-giving-gifts-to-kindergarten-teachers/e19b49d4-a1a4-4533-9089-6e10e2d9386a"
        ));
        assert!(!suitable("https://www.9news.com.au/national/"));
    }

    #[test]
    fn state_names_the_video_and_account() {
        let state = serde_json::json!({
            "article": [{"media": [{"type": "image", "urn": "x"}, {"type": "video", "urn": "1"}]}],
            "videoIndex": {"config": {"video": {"account": "2"}}}
        });
        assert_eq!(video_id(&state).as_deref(), Some("1"));
        assert_eq!(account(&state).as_deref(), Some("2"));
    }
}
