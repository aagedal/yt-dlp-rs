//! Upstream `NascarClassicsIE` (`yt_dlp/extractor/nascar.py`): NASCAR
//! Classics videos from the page's Next.js `contentData`: its input's HLS
//! source with the name, description, thumbnail, tags and start time, and
//! the overlay's first timeline events as chapters.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct NascarClassics;

const NAME: &str = "NascarClassics";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?classics\.nascar\.com/video/(?P<id>[\w~-]+)").unwrap()
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

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("NASCAR Classics {what} are not supported natively"))
}

/// Python `float(value)`: `None` when it raises; non-finite values (which
/// upstream would print as `NaN`/`Infinity`) are unsupported.
fn python_float(value: &Value) -> Result<Option<f64>> {
    let number = match value {
        Value::Number(number) => number.as_f64(),
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        Value::String(text) => {
            let text = text.trim();
            let lowered = text.to_ascii_lowercase();
            let unsigned = lowered.trim_start_matches(['+', '-']);
            if matches!(unsigned, "inf" | "infinity" | "nan") {
                return Err(unsupported("non-finite chapter timestamps"));
            }
            if text.contains('_') {
                return Err(unsupported("chapter timestamps with underscores"));
            }
            if text.is_empty()
                || !text
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
            {
                None
            } else {
                text.parse().ok()
            }
        }
        _ => None,
    };
    match number {
        Some(number) if !number.is_finite() => Err(unsupported("non-finite chapter timestamps")),
        number => Ok(number),
    }
}

/// `('overlay', 'data', 'timelines', 0, 'events', lambda _, v:
/// float(v['timestamp']) is not None, {'start_time': ..., 'title': ...})`.
fn chapters(content: &Value) -> Result<Option<Value>> {
    let events = match content
        .pointer("/overlay/data/timelines")
        .and_then(|timelines| match timelines {
            Value::Array(timelines) => timelines.first(),
            _ => None,
        })
        .and_then(|timeline| timeline.get("events"))
    {
        Some(Value::Array(events)) => events.iter().collect::<Vec<_>>(),
        Some(Value::Object(events)) => events.values().collect(),
        _ => return Ok(None),
    };
    let mut chapters = Vec::new();
    for event in events {
        let Some(timestamp) = event.as_object().and_then(|event| event.get("timestamp")) else {
            continue;
        };
        let Some(start_time) = python_float(timestamp)? else {
            continue;
        };
        let mut chapter = serde_json::Map::new();
        chapter.insert("start_time".into(), start_time.into());
        if let Some(Value::String(title)) = event.get("name") {
            chapter.insert("title".into(), title.clone().into());
        }
        chapters.push(Value::Object(chapter));
    }
    Ok((!chapters.is_empty()).then_some(Value::Array(chapters)))
}

impl Extractor for NascarClassics {
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
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        let mut content = &next_data;
        for key in ["props", "pageProps", "contentData"] {
            content = content.get(key).ok_or_else(|| key_error(&video_id, key))?;
        }
        let input = content
            .get("input")
            .ok_or_else(|| key_error(&video_id, "input"))?;
        let source = match input.get("src") {
            Some(Value::String(source)) => source,
            Some(_) => return Err(unsupported("sources that are not strings")),
            None => return Err(key_error(&video_id, "src")),
        };
        let manifest = Url::parse(source).map_err(|_| unsupported("sources that are not URLs"))?;
        let stream = extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, false)?;
        let mut fields: Vec<(&str, Value)> =
            vec![("id", video_id.clone().into()), ("formats", Value::Null)];
        if let Some(Value::String(title)) = input.get("name") {
            fields.push(("title", title.clone().into()));
        }
        if let Some(Value::String(description)) = input.get("description")
            && !description.is_empty()
        {
            fields.push(("description", description.clone().into()));
        }
        if let Some(thumbnail) = input
            .get("thumbnail")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            fields.push(("thumbnail", thumbnail.into()));
        }
        let tags: Vec<Value> = match input.pointer("/settings/tags") {
            Some(Value::Array(tags)) => {
                tags.iter().filter(|tag| tag.is_string()).cloned().collect()
            }
            Some(Value::Object(tags)) => tags
                .values()
                .filter(|tag| tag.is_string())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        if !tags.is_empty() {
            fields.push(("tags", Value::Array(tags)));
        }
        if let Some(timestamp) = input
            .get("start_time")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.push(("timestamp", timestamp.into()));
        }
        if let Some(chapters) = chapters(content)? {
            fields.push(("chapters", chapters));
        }
        let mut info = dict_info(url, (NAME, NAME), fields, stream.formats);
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";chapter=start_time,title");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://classics.nascar.com/video/Ka5qGuxzZ~SIvJii7uAC~wszPshklHN"
        ));
        assert!(!suitable("https://www.nascar.com/video/abc"));
    }

    #[test]
    fn floats_follow_python() {
        assert_eq!(python_float(&Value::from(" 12.5 ")).unwrap(), Some(12.5));
        assert_eq!(python_float(&Value::from("x")).unwrap(), None);
        assert_eq!(python_float(&Value::Null).unwrap(), None);
        assert!(python_float(&Value::from("nan")).is_err());
    }
}
