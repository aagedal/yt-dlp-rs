//! Upstream `CCCIE` (`yt_dlp/extractor/ccc.py`): a media.ccc.de event page's
//! `data-id`, the `public/events/ID` JSON and its recordings as formats; and
//! `CCCPlaylistIE`: a conference's `public/conferences/ID` JSON events as
//! `CCC` url entries.
use super::clyp::key_error;
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Ccc;
pub struct CccPlaylist;

const PLAYLIST_NAME: &str = "media.ccc.de:lists";
const PLAYLIST_KEY: &str = "CCCPlaylist";

const NAME: &str = "media.ccc.de";
const KEY: &str = "CCC";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?media\.ccc\.de/v/(?P<id>[^/?#&]+)").unwrap()
});
static EVENT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-id=(?:'(?P<a>\d+)'|"(?P<b>\d+)")"#).unwrap());

static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"https?://(?:www\.)?media\.ccc\.de/c/(?P<id>[^/?#&]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `CCCPlaylistIE._VALID_URL`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `url_result(event_url, ie=CCCIE.ie_key())` as a playlist entry.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), KEY.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
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

impl Extractor for CccPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(PLAYLIST_NAME);
        crate::screen::screen(&format!(
            "[{PLAYLIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://media.ccc.de/public/conferences/{playlist_id}"
        ))?;
        let conf = download_json(client, &api, PLAYLIST_NAME, &playlist_id)?;
        let mut entries = Vec::new();
        if let Some(events) = conf.get("events").and_then(Value::as_array) {
            for event in events {
                let link = event
                    .get("frontend_link")
                    .and_then(Value::as_str)
                    .filter(|link| link.starts_with("http://") || link.starts_with("https://"));
                if let Some(link) = link {
                    entries.push(url_entry(link)?);
                }
            }
        }
        let title = conf
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_default();
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title,
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
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
        assert!(playlist_suitable("https://media.ccc.de/c/30c3"));
        assert!(!playlist_suitable("https://media.ccc.de/v/30c3"));
    }
}
