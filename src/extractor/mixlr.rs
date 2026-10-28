//! Upstream `MixlrIE` and `MixlrRecoringIE` (`yt_dlp/extractor/mixlr.py`):
//! a channel event's v3 API JSON (its progressive stream, checked with a
//! `HEAD` request, as the only format) and a recording's API JSON as an
//! untyped result.
use super::json_ld::url_or_none;
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Mixlr;
pub struct MixlrRecording;

const NAME: &str = "Mixlr";
const RECORDING_NAME: &str = "MixlrRecoring";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?(?P<username>[\w-]+)\.mixlr\.com/events/(?P<id>\d+)").unwrap()
});
static RECORDING_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?(?P<username>[\w-]+)\.mixlr\.com/recordings/(?P<id>\d+)")
        .unwrap()
});

/// `MixlrIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `MixlrRecoringIE._match_valid_url`.
pub fn recording_suitable(url: &str) -> bool {
    RECORDING_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `traverse_obj(obj, (*path))` through objects and list indices.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .try_fold(value, |value, key| match key.parse::<usize>() {
            Ok(index) => value.as_array()?.get(index),
            Err(_) => value.as_object()?.get(*key),
        })
}

/// `{str}`.
fn string(value: Option<&Value>) -> Value {
    value
        .filter(|value| value.is_string())
        .cloned()
        .unwrap_or(Value::Null)
}

/// `{url_or_none}`.
fn url_value(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(url_or_none)
        .map_or(Value::Null, Value::from)
}

/// `{parse_iso8601}` (non-strings fail inside `traverse_obj`).
fn timestamp(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map_or(Value::Null, Value::from)
}

/// `{bool}`.
fn boolean(value: Option<&Value>) -> Value {
    value
        .filter(|value| value.is_boolean())
        .cloned()
        .unwrap_or(Value::Null)
}

/// `{**traverse_obj(obj, {...})}`: the template's non-`None` values, a
/// repeated key keeping its first position.
fn merge(fields: &mut Vec<(&'static str, Value)>, template: Vec<(&'static str, Value)>) {
    for (key, value) in template {
        if value.is_null() {
            continue;
        }
        match fields.iter_mut().find(|(known, _)| *known == key) {
            Some(field) => field.1 = value,
            None => fields.push((key, value)),
        }
    }
}

impl Extractor for Mixlr {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let username = found["username"].to_owned();
        let event_id = found["id"].to_owned();
        announce(NAME, url);
        let api = Url::parse(&format!(
            "https://api.mixlr.com/v3/channels/{username}/events/{event_id}"
        ))?;
        let broadcast_info = download_json(client, &api, NAME, &event_id)?;

        let mut formats = Vec::new();
        let format_url = url_or_none_at(
            &broadcast_info,
            &["included", "0", "attributes", "progressive_stream_url"],
        );
        if let Some(format_url) = format_url {
            // `_request_webpage(HEADRequest(format_url), fatal=False,
            // note='Checking stream')`.
            crate::screen::screen(&format!("[{NAME}] {event_id}: Checking stream"));
            let target = Url::parse(&format_url)?;
            match super::direct::head_with_headers(client, &target, &[]) {
                Ok(response) if response.status().as_u16() == 200 => {
                    let mut ext = super::direct::urlhandle_detect_ext(&response, None);
                    drop(response);
                    if ext == "octet-stream" {
                        crate::screen::warning(&format!(
                            "[{NAME}] The server did not return a valid file extension for the stream URL. \
                             Assuming an mp3 stream; postprocessing may fail if this is incorrect"
                        ));
                        ext = "mp3".into();
                    }
                    if ext.is_empty() {
                        return Err(Error::UnsupportedHtml5(
                            "Mixlr streams without a detectable extension are not supported natively"
                                .into(),
                        ));
                    }
                    formats.push(dict_format(
                        vec![
                            ("url", format_url.as_str().into()),
                            ("ext", ext.into()),
                            ("vcodec", "none".into()),
                        ],
                        NAME,
                        &event_id,
                    )?);
                }
                Ok(_) => {}
                Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
                    crate::screen::warning(&format!(
                        "[{NAME}] {event_id}: Unable to download webpage: {}",
                        crate::upstream_error_text(&error)
                    ));
                }
                Err(error) => return Err(error),
            }
        }

        let release_timestamp = path(&broadcast_info, &["data", "attributes", "starts_at"])
            .and_then(Value::as_str)
            .map(str::to_owned);
        if formats.is_empty()
            && let Some(release) = release_timestamp.as_deref()
            && !release.is_empty()
        {
            crate::no_formats::raise_expected(
                NAME,
                None,
                Some(&event_id),
                &format!("This event will start at {release}"),
            )?;
        }

        let mut fields: Vec<(&'static str, Value)> = vec![
            ("id", event_id.as_str().into()),
            ("uploader", username.as_str().into()),
            ("formats", Value::Null),
            (
                "release_timestamp",
                release_timestamp
                    .as_deref()
                    .and_then(crate::dates::parse_iso8601)
                    .map_or(Value::Null, Value::from),
            ),
        ];
        let included = path(&broadcast_info, &["included", "0", "attributes"]);
        let get = |key: &str| included.and_then(|attributes| attributes.get(key));
        merge(
            &mut fields,
            vec![
                ("title", string(get("title"))),
                ("timestamp", timestamp(get("started_at"))),
                (
                    "concurrent_view_count",
                    int_or_none(get("concurrent_view_count")),
                ),
                ("like_count", int_or_none(get("heart_count"))),
                ("is_live", boolean(get("live"))),
            ],
        );
        let data = path(&broadcast_info, &["data", "attributes"]);
        let get = |key: &str| data.and_then(|attributes| attributes.get(key));
        merge(
            &mut fields,
            vec![
                ("title", string(get("title"))),
                ("description", string(get("description"))),
                ("timestamp", timestamp(get("started_at"))),
                (
                    "concurrent_view_count",
                    int_or_none(get("concurrent_view_count")),
                ),
                ("like_count", int_or_none(get("heart_count"))),
                ("thumbnail", url_value(get("artwork_url"))),
                ("uploader_id", string(get("broadcaster_id"))),
            ],
        );
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

/// `traverse_obj(obj, (*path, {url_or_none}))`.
fn url_or_none_at(value: &Value, keys: &[&str]) -> Option<String> {
    path(value, keys)
        .and_then(Value::as_str)
        .and_then(url_or_none)
}

impl Extractor for MixlrRecording {
    fn key(&self) -> &'static str {
        RECORDING_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = RECORDING_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let username = found["username"].to_owned();
        let recording_id = found["id"].to_owned();
        announce(RECORDING_NAME, url);
        let api = Url::parse(&format!(
            "https://api.mixlr.com/v3/channels/{username}/recordings/{recording_id}"
        ))?;
        let recording_info = download_json(client, &api, RECORDING_NAME, &recording_id)?;

        let attributes = path(&recording_info, &["data", "attributes"]);
        let get = |key: &str| attributes.and_then(|attributes| attributes.get(key));
        let ext = string(get("file_format"));
        let media = url_value(get("url"));
        let mut fields: Vec<(&'static str, Value)> = vec![("id", recording_id.as_str().into())];
        merge(
            &mut fields,
            vec![
                ("ext", ext.clone()),
                ("url", media.clone()),
                ("title", string(get("title"))),
                ("description", string(get("description"))),
                ("timestamp", timestamp(get("created_at"))),
                ("duration", int_or_none(get("duration"))),
                ("thumbnail", url_value(get("artwork_url"))),
                ("uploader_id", string(get("user_id"))),
            ],
        );
        let names = (RECORDING_NAME, RECORDING_NAME);
        let Some(media) = media.as_str() else {
            return Ok(Some(dict_info(url, names, fields, Vec::new())));
        };
        // A missing `ext` is processing's `determine_ext(url)`.
        let ext = match ext.as_str() {
            Some(ext) => ext.to_owned(),
            None => super::direct::determine_ext(media).unwrap_or_else(|| "unknown_video".into()),
        };
        let mut format: crate::VideoFormat =
            serde_json::from_value(json!({"format_id": "", "url": media, "ext": ext}))
                .map_err(|error| Error::Reported(format!("{recording_id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "url-only");
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("untyped-dict;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(
            dict_info(url, names, fields, vec![format]),
            &shape,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://suncity-104-9fm.mixlr.com/events/4387115"));
        assert!(suitable("https://www.brcountdown.mixlr.com/events/4395480"));
        assert!(!suitable(
            "https://brcountdown.mixlr.com/recordings/4395480"
        ));
        assert!(recording_suitable(
            "https://biblewayng.mixlr.com/recordings/2375193"
        ));
        assert!(!recording_suitable(
            "https://biblewayng.mixlr.com/events/2375193"
        ));
    }
}
