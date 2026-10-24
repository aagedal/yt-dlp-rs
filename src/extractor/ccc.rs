//! Upstream `CCCIE` (`yt_dlp/extractor/ccc.py`): a media.ccc.de event page's
//! `data-id`, the `public/events/ID` JSON and its recordings as formats.
use super::clyp::key_error;
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Ccc;

const NAME: &str = "media.ccc.de";
const KEY: &str = "CCC";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?media\.ccc\.de/v/(?P<id>[^/?#&]+)").unwrap());
static EVENT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-id=(?:'(?P<a>\d+)'|"(?P<b>\d+)")"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("media.ccc.de {what} are not supported natively"))
}

/// `int_or_none(value, invscale=INVSCALE)`.
fn int_or_none(value: Option<&Value>, invscale: i64) -> Value {
    let number = match value {
        Some(Value::Bool(flag)) => Some(i64::from(*flag)),
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    };
    number.map_or(Value::Null, |number| Value::from(number * invscale))
}

/// `event_data.get(key)` of a string (or missing) value.
fn optional_str(map: &Value, key: &str) -> Result<Option<String>> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(unsupported(&format!("{key} values that are not strings"))),
    }
}

impl Extractor for Ccc {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let event_id = EVENT_ID
            .captures(&webpage)
            .and_then(|found| found.name("a").or_else(|| found.name("b")))
            .map(|found| found.as_str().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract event id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let api = Url::parse(&format!("https://media.ccc.de/public/events/{event_id}"))?;
        let event_data = download_json(client, &api, NAME, &event_id)?;
        if !event_data.is_object() {
            return Err(unsupported("event data that are not objects"));
        }
        let recordings = match event_data.get("recordings") {
            None => Vec::new(),
            Some(Value::Array(items)) => items.clone(),
            Some(_) => return Err(unsupported("recording lists that are not arrays")),
        };
        let mut formats = Vec::new();
        for recording in &recordings {
            if !recording.is_object() {
                return Err(unsupported("recordings that are not objects"));
            }
            let Some(recording_url) = recording
                .get("recording_url")
                .filter(|value| super::nuxt::truthy(value))
            else {
                continue;
            };
            let language = optional_str(recording, "language")?;
            let Some(folder) = optional_str(recording, "folder")? else {
                return Err(unsupported("recordings without a folder"));
            };
            let language = language.filter(|text| !text.is_empty());
            let format_id = match (&language, folder.is_empty()) {
                (Some(language), true) => Value::from(language.clone()),
                (Some(language), false) => Value::from(format!("{language}-{folder}")),
                (None, false) => Value::from(folder.clone()),
                (None, true) => Value::Null,
            };
            let vcodec = if folder.contains("h264") {
                Value::from("h264")
            } else if folder == "mp3" || folder == "opus" {
                Value::from("none")
            } else {
                Value::Null
            };
            formats.push(dict_format(
                vec![
                    ("format_id", format_id),
                    ("url", recording_url.clone()),
                    ("width", int_or_none(recording.get("width"), 1)),
                    ("height", int_or_none(recording.get("height"), 1)),
                    ("filesize", int_or_none(recording.get("size"), 1024 * 1024)),
                    (
                        "language",
                        recording.get("language").cloned().unwrap_or(Value::Null),
                    ),
                    ("vcodec", vcodec),
                ],
                NAME,
                &event_id,
            )?);
        }
        let title = event_data
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&display_id, "title"))?;
        if !title.is_string() {
            return Err(unsupported("titles that are not strings"));
        }
        // `try_get(event_data, lambda x: ', '.join(x['persons']))`.
        let creator = event_data
            .get("persons")
            .and_then(Value::as_array)
            .and_then(|persons| {
                persons
                    .iter()
                    .map(|person| person.as_str())
                    .collect::<Option<Vec<_>>>()
            })
            .map_or(Value::Null, |persons| Value::from(persons.join(", ")));
        let timestamp = optional_str(&event_data, "date")?
            .and_then(|date| crate::dates::parse_iso8601(&date))
            .map_or(Value::Null, Value::from);
        let get = |key: &str| event_data.get(key).cloned().unwrap_or(Value::Null);
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", event_id.clone().into()),
                ("display_id", display_id.clone().into()),
                ("title", title),
                ("creator", creator),
                ("description", get("description")),
                ("thumbnail", get("thumb_url")),
                ("timestamp", timestamp),
                ("duration", int_or_none(event_data.get("length"), 1)),
                ("view_count", int_or_none(event_data.get("view_count"), 1)),
                ("tags", get("tags")),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://media.ccc.de/v/32c3-7368-shopshifting#download"
        ));
        assert!(!suitable("https://media.ccc.de/c/30c3"));
    }
}
