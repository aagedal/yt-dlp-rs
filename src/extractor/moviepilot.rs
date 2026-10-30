//! Upstream `MoviepilotIE` (`yt_dlp/extractor/moviepilot.py`): Moviepilot
//! movie trailers as transparent Dailymotion `url` results, with the trailer
//! page's Next.js title and summary.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Moviepilot;

const NAME: &str = "moviepilot";
const KEY: &str = "Moviepilot";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?moviepilot\.de/movies/(?P<id>[^/]+)").unwrap()
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

impl Extractor for Moviepilot {
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
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = Url::parse(&format!(
            "https://www.moviepilot.de/movies/{video_id}/trailer"
        ))?;
        let webpage = download_webpage(client, &page, NAME, &video_id)?;
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        let mut clip = &next_data;
        for key in ["props", "initialProps", "pageProps"] {
            clip = clip.get(key).ok_or_else(|| key_error(&video_id, key))?;
        }
        let remote_id = clip
            .get("video")
            .ok_or_else(|| key_error(&video_id, "video"))?
            .get("remoteId")
            .ok_or_else(|| key_error(&video_id, "remoteId"))?;
        let remote_id = match remote_id {
            Value::String(id) => id.clone(),
            Value::Number(id) => id.to_string(),
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "Moviepilot remote IDs that are not strings are not supported natively".into(),
                ));
            }
        };
        let mut fields = Map::new();
        fields.insert("display_id".into(), video_id.into());
        fields.insert(
            "title".into(),
            clip.get("title").cloned().unwrap_or(Value::Null),
        );
        fields.insert(
            "description".into(),
            clip.get("summary").cloned().unwrap_or(Value::Null),
        );
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({
                "url": format!("https://www.dailymotion.com/video/{remote_id}"),
                "fields": Value::Object(fields),
            }),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.moviepilot.de/movies/interstellar-2"));
        assert!(!suitable("https://www.moviepilot.de/serie/x"));
    }
}
