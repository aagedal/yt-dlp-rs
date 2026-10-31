//! Upstream `RoyaLiveIE` (`yt_dlp/extractor/roya.py`): a Roya TV fast
//! channel's secured live HLS URL and its schedule title.
use super::{Extractor, download_json, extract_m3u8_fatal_live};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct RoyaLive;

const NAME: &str = "RoyaLive";
const SCHEDULE: &str = "https://backend.roya.tv/api/v01/channels/schedule-pagination";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:en\.)?roya\.tv/live-stream/(?P<id>\d+)").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Upstream's wrapped `KeyError` (reported with the media ID).
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_download_json(url, video_id, fatal=False)`: a warning and `None` on
/// failure.
fn optional_json(client: &Client, url: &Url, id: &str) -> Result<Option<Value>> {
    match download_json(client, url, NAME, id) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(Error::Reported(message)) if message.contains(": Failed to parse JSON ") => {
            crate::screen::warning(&message);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Python's `str(value)` for the channel `id` comparison; other types never
/// match a decimal media ID.
fn python_str(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(number.to_string()),
        _ => None,
    }
}

/// `('data', 0, 'channel', lambda _, v: str(v['id']) == media_id, 'title',
/// {str}, any)`.
fn schedule_title(schedule: &Value, id: &str) -> Option<String> {
    // An integer index never matches a JSON object's string keys.
    let first = schedule.get("data")?.as_array()?.first()?;
    let channels: Vec<&Value> = match first.get("channel")? {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map.values().collect(),
        _ => return None,
    };
    channels
        .into_iter()
        .filter(|channel| {
            channel
                .get("id")
                .and_then(python_str)
                .is_some_and(|text| text == id)
        })
        .find_map(|channel| channel.get("title")?.as_str().map(str::to_owned))
}

impl Extractor for RoyaLive {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let ticket = Url::parse(&format!(
            "https://ticket.roya-tv.com/api/v5/fastchannel/{id}"
        ))?;
        let ticket = download_json(client, &ticket, NAME, &id)?;
        let data = match &ticket {
            Value::Object(map) => map.get("data").ok_or_else(|| key_error(&id, "data"))?,
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "non-object Roya TV ticket responses are not supported natively".into(),
                ));
            }
        };
        let stream_url = match data {
            Value::Object(map) => map
                .get("secured_url")
                .ok_or_else(|| key_error(&id, "secured_url"))?,
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "non-object Roya TV ticket data is not supported natively".into(),
                ));
            }
        };
        let stream_url = stream_url.as_str().ok_or_else(|| {
            Error::UnsupportedHtml5(
                "non-string Roya TV stream URLs are not supported natively".into(),
            )
        })?;
        let stream_url = Url::parse(stream_url)?;
        let title = optional_json(client, &Url::parse(SCHEDULE)?, &id)?
            .and_then(|schedule| schedule_title(&schedule, &id));
        let mut info = extract_m3u8_fatal_live(client, &stream_url, NAME, &id, Some("hls"), false)?;
        info.id = id;
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(true));
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,formats,title,is_live",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://en.roya.tv/live-stream/1"));
        assert!(suitable("https://roya.tv/live-stream/10000"));
        assert!(!suitable("https://www.roya.tv/live-stream/1"));
        assert!(!suitable("https://roya.tv/live-stream/abc"));
    }

    #[test]
    fn schedule_titles_match_the_channel_id() {
        let schedule = serde_json::json!({"data": [{"channel": [
            {"id": 2, "title": "Other"},
            {"title": "No id"},
            {"id": "21", "title": 5},
            {"id": 21, "title": "Roya News"},
        ]}]});
        assert_eq!(
            schedule_title(&schedule, "21").as_deref(),
            Some("Roya News")
        );
        assert_eq!(schedule_title(&schedule, "3"), None);
    }
}
