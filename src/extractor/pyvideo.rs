//! Upstream `PyvideoIE` (`yt_dlp/extractor/pyvideo.py`): PyVideo talks as
//! playlists of their `pyvideo/data` JSON videos (YouTube `url` results or
//! direct media), or of the page's "Media URL" links when that JSON fails.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Pyvideo;

const NAME: &str = "Pyvideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?pyvideo\.org/(?P<category>[^/]+)/(?P<id>[^/?#&.]+)",
    )
    .unwrap()
});
static MEDIA_URLS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)Media URL:(.+?)</li>").unwrap());
static MEDIA_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+href=(?:"(http.+?)"|'(http.+?)')"#).unwrap());
static YOUTUBE_WATCH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://www\.youtube\.com/watch\?v=").unwrap());

/// `_match_valid_url`.
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
    Error::UnsupportedHtml5(format!("PyVideo {what} are not supported natively"))
}

/// Python truthiness of a parsed JSON value (`if data:`).
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

/// `_download_json(..., fatal=False)`: `None` (after a warning) when the
/// request or the parse fails.
fn download_data(client: &Client, api: &Url, video_id: &str) -> Result<Option<Value>> {
    match super::download_json(client, api, NAME, video_id) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: Unable to download JSON metadata: {}",
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

/// `url_result(url, 'Youtube')`.
fn youtube_entry(target: &str) -> Result<VideoInfo> {
    let temp_id = super::youtube::temp_id(target);
    let missing = temp_id.is_none();
    let mut entry = super::nrk::url_entry(target, "Youtube", None, temp_id, None)?;
    // `get_temp_id(url)` is `None` for a URL YouTube does not match.
    if missing {
        entry.id = String::new();
    }
    Ok(entry)
}

/// A `{'id': ..., 'url': ..., ...}` entry that is its own format.
fn media_entry(page: &Url, fields: Vec<(&str, Value)>, media: &str) -> Result<VideoInfo> {
    let ext = super::direct::determine_ext(media)
        .unwrap_or_else(|| "unknown_video".into())
        .to_lowercase();
    let mut format: crate::VideoFormat =
        serde_json::from_value(json!({"format_id": "", "url": media, "ext": ext}))
            .map_err(|error| Error::Reported(format!("[{NAME}] {error}")))?;
    crate::info_order::mark_format(&mut format, "url-only");
    let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
    let shape = format!("untyped-dict;{}", keys.join(","));
    Ok(crate::info_order::mark_info(
        dict_info(page, (NAME, NAME), fields, vec![format]),
        &shape,
    ))
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        Some(Value::Bool(value)) => i64::from(*value).into(),
        _ => Value::Null,
    }
}

/// Python's `str(value)` of a JSON id.
fn str_id(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) if number.is_i64() || number.is_u64() => Ok(number.to_string()),
        _ => Err(unsupported("video ids that are not strings or integers")),
    }
}

fn json_entries(page: &Url, data: &Value, video_id: &str) -> Result<Vec<VideoInfo>> {
    let videos = match data.get("videos") {
        None => return Err(key_error(video_id, "videos")),
        Some(Value::Array(videos)) => videos,
        Some(_) => return Err(unsupported("video lists that are not arrays")),
    };
    let mut entries = Vec::new();
    for video in videos {
        let Value::Object(video) = video else {
            return Err(unsupported("videos that are not objects"));
        };
        let video_url = match video.get("url") {
            Some(Value::String(text)) if !text.is_empty() => text.as_str(),
            None | Some(Value::Null) => continue,
            Some(Value::String(_)) => continue,
            Some(_) => return Err(unsupported("video URLs that are not strings")),
        };
        if video.get("type").and_then(Value::as_str) == Some("youtube") {
            entries.push(youtube_entry(video_url)?);
            continue;
        }
        let id = match data.get("id").filter(|id| truthy(id)) {
            Some(id) => str_id(id)?,
            None => video_id.to_owned(),
        };
        let title = match data.get("title") {
            None => return Err(key_error(video_id, "title")),
            Some(Value::String(title)) => Value::from(title.as_str()),
            Some(Value::Null) => Value::Null,
            Some(_) => return Err(unsupported("titles that are not strings")),
        };
        let description = ["description", "summary"]
            .iter()
            .find_map(|key| data.get(*key).filter(|value| truthy(value)))
            .cloned()
            .unwrap_or(Value::Null);
        let fields = vec![
            ("id", Value::from(id)),
            ("url", Value::from(video_url)),
            ("title", title),
            ("description", description),
            (
                "thumbnail",
                data.get("thumbnail_url").cloned().unwrap_or(Value::Null),
            ),
            ("duration", int_or_none(data.get("duration"))),
        ];
        entries.push(media_entry(page, fields, video_url)?);
    }
    Ok(entries)
}

fn page_entries(client: &Client, url: &Url, video_id: &str) -> Result<Vec<VideoInfo>> {
    let webpage = download_webpage(client, url, NAME, video_id)?;
    // `_og_search_title(webpage)` (non-fatal).
    let title = crate::html::og_search_property(&webpage, "title");
    if title.is_none() {
        crate::screen::warning(&format!(
            "[{NAME}] unable to extract OpenGraph title; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    let Some(media_urls) = MEDIA_URLS.captures(&webpage) else {
        return Err(Error::Reported(format!(
            "[{NAME}] {video_id}: Unable to extract media urls; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let mut entries = Vec::new();
    for found in MEDIA_LINK.captures_iter(&media_urls[1]) {
        let media = found
            .get(1)
            .or_else(|| found.get(2))
            .map_or("", |media| media.as_str());
        if YOUTUBE_WATCH.is_match(media) {
            entries.push(youtube_entry(media)?);
            continue;
        }
        let fields = vec![
            ("id", Value::from(video_id)),
            ("url", Value::from(media)),
            ("title", title.clone().map_or(Value::Null, Value::from)),
        ];
        entries.push(media_entry(url, fields, media)?);
    }
    Ok(entries)
}

impl Extractor for Pyvideo {
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
        let category = found["category"].to_owned();
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://raw.githubusercontent.com/pyvideo/data/master/{category}/videos/{video_id}.json"
        ))?;
        let entries = match download_data(client, &api, &video_id)? {
            Some(data) if truthy(&data) => json_entries(url, &data, &video_id)?,
            _ => page_entries(client, url, &video_id)?,
        };
        Ok(Some(super::nrk::playlist_result(
            url, NAME, video_id, None, None, entries, false,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://pyvideo.org/pycon-us-2013/become-a-logging-expert-in-30-minutes.html"
        ));
        assert!(!suitable("http://pyvideo.org/pycon-us-2013"));
    }
}
