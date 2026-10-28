//! Upstream `TruNewsIE` (`yt_dlp/extractor/trunews.py`): TruNews streams as
//! `url` results of the Zype player for the API's first video with that
//! friendly title.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct TruNews;

const NAME: &str = "TruNews";
const KEY: &str = "TruNews";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?trunews\.com/stream/(?P<id>[^/?#&]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for TruNews {
    fn key(&self) -> &'static str {
        KEY
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
        let api = super::update_url_query(
            &Url::parse("https://api.zype.com/videos")?,
            &[
                (
                    "app_key",
                    "PUVKp9WgGUb3-JUw6EqafLx8tFVP6VKZTWbUOR-HOm__g4fNDt1bCsm_LgYf_k9H",
                ),
                ("per_page", "1"),
                ("active", "true"),
                ("friendly_title", id.as_str()),
            ],
        );
        let data = super::download_json(client, &api, NAME, &id)?;
        let key_error = |key: &str| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let response = data.get("response").ok_or_else(|| key_error("response"))?;
        let Value::Array(videos) = response else {
            return Err(Error::UnsupportedHtml5(
                "TruNews responses that are not lists are not supported natively".into(),
            ));
        };
        // An uncaught `IndexError` is reported as its message.
        let video = videos
            .first()
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let zype_id = match video.get("_id") {
            Some(Value::String(zype_id)) => zype_id.clone(),
            Some(Value::Number(zype_id)) => zype_id.to_string(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "TruNews Zype IDs that are not strings are not supported natively".into(),
                ));
            }
            None => return Err(key_error("_id")),
        };
        Ok(Some(super::follow_url(
            url,
            format!(
                "https://player.zype.com/embed/{zype_id}.js?api_key=X5XnahkjCwJrT_l5zUqypnaLEObotyvtUKJWWlONxDoHVjP8vqxlArLV8llxMbyt"
            ),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.trunews.com/stream/will-democrats-stage-a-circus-during-president-trump-s-state-of-the-union-speech"
        ));
        assert!(!suitable("https://www.trunews.com/"));
    }
}
