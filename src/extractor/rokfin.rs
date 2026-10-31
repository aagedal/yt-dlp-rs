//! Upstream `RokfinIE` and `RokfinStackIE` (`yt_dlp/extractor/rokfin.py`):
//! a post's or stream's `prod-api-v2` JSON with its media URL (or HLS
//! playlist), and a stack's posts as `url` entries. Login (premium content,
//! token refresh) and the channel and search classes are not supported.
use super::{Extractor, dict_format, dict_info, download_webpage_note, extract_m3u8_optional};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Rokfin;
pub struct RokfinStack;

const NAME: &str = "Rokfin";
const STACK_NAME: &str = "rokfin:stack";
const API_BASE: &str = "https://prod-api-v2.production.rokfin.com/api/v2/public/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"https?://(?:www\.)?rokfin\.com/(?P<id>(?P<type>post|stream)/\d+)")
        .unwrap()
});
static STACK_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"https?://(?:www\.)?rokfin\.com/stack/(?P<id>[^/]+)").unwrap()
});
static STORYBOARD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://[^/]+/([^/]+)/storyboard\.vtt").unwrap());

/// `_match_id` of [`Rokfin`].
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id` of [`RokfinStack`].
pub fn stack_suitable(url: &str) -> bool {
    STACK_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Rokfin {what} are not supported natively"))
}

/// `traverse_obj(value, path...)` for a string-valued `url_or_none`: the
/// first candidate that is an absolute URL.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| Url::parse(text).is_ok_and(|url| matches!(url.scheme(), "http" | "https")))
        .map(str::to_owned)
}

/// `str_or_none` of a JSON value (numbers are stringified).
fn str_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => text.clone().into(),
        Some(Value::Number(number)) => number.to_string().into(),
        _ => Value::Null,
    }
}

/// `float_or_none`.
fn float_or_none(value: Option<&Value>) -> Value {
    crate::extractor::clyp::float_or_none(value)
}

/// `int_or_none`.
fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number.as_i64().map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

/// `unified_timestamp(value)`.
fn unified_timestamp(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_str)
        .and_then(|date| crate::dates::unified_timestamp(date, false))
}

/// `value.get(key)` for an object.
fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key)
}

/// Upstream `url_result(url, video_id=..., video_title=...)` without an
/// `ie`: a `url` entry resolved when processed.
fn url_entry(target: &str, id: &str, title: Option<String>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = id.to_owned();
    let mut entry = Map::new();
    entry.insert("id".into(), id.into());
    if let Some(title) = &title {
        entry.insert("title".into(), title.clone().into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// Upstream `_download_json_using_access_token` without a token: a plain
/// `prod-api-v2` request.
fn api_json(client: &Client, path: &str, id: &str) -> Result<Value> {
    let url = Url::parse(&format!("{API_BASE}{path}"))?;
    let body = download_webpage_note(client, &url, NAME, id, "Downloading webpage")?;
    crate::json::from_str(&body).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {id}: Failed to parse JSON (caused by {error})"
        ))
    })
}

impl Extractor for Rokfin {
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
        let video_id = found["id"].to_owned();
        let video_type = found["type"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let metadata = api_json(client, &video_id, &video_id)?;

        let scheduled = unified_timestamp(field(&metadata, "scheduledAt"));
        let live_status = if field(&metadata, "stoppedAt").is_some_and(|value| !value.is_null()) {
            "was_live"
        } else if scheduled.is_some() {
            "is_upcoming"
        } else if video_type == "stream" {
            "is_live"
        } else {
            "not_live"
        };
        let content = field(&metadata, "content").cloned().unwrap_or(Value::Null);

        let mut video_url =
            url_or_none(field(&metadata, "url")).or_else(|| url_or_none(content.get("contentUrl")));
        if video_url.is_none() || video_url.as_deref() == Some("fake.m3u8") {
            let timeline = url_or_none(field(&metadata, "timelineUrl"))
                .or_else(|| url_or_none(content.get("timelineUrl")));
            video_url = timeline.and_then(|timeline| {
                STORYBOARD
                    .captures(&timeline)
                    .map(|found| format!("https://stream.v.rokfin.com/{}.m3u8", &found[1]))
            });
        }
        let mut formats = Vec::new();
        if let Some(video_url) = &video_url {
            if video_url.ends_with(".m3u8") {
                if live_status == "is_live" {
                    return Err(unsupported("live HLS streams"));
                }
                let manifest = Url::parse(video_url)?;
                if let Some(stream) =
                    extract_m3u8_optional(client, &manifest, NAME, &video_id, None, false)
                {
                    formats.extend(stream.formats);
                }
            } else {
                formats.push(dict_format(
                    vec![("url", video_url.clone().into())],
                    NAME,
                    &video_id,
                )?);
            }
        }
        if formats.is_empty() {
            let premium = field(&metadata, "premiumPlan").is_some_and(truthy)
                || field(&metadata, "premium").is_some_and(truthy);
            if premium {
                return Err(unsupported("premium-only videos (login)"));
            }
            if let Some(timestamp) = scheduled {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Stream is offline; scheduled for {timestamp}"
                )));
            }
        }

        let uploader = field(&metadata, "createdBy")
            .and_then(|creator| creator.get("username"))
            .or_else(|| field(&metadata, "creator").and_then(|creator| creator.get("username")))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let channel = field(&metadata, "createdBy")
            .and_then(|creator| creator.get("name"))
            .or_else(|| field(&metadata, "creator").and_then(|creator| creator.get("name")));
        let channel_id = field(&metadata, "createdBy")
            .and_then(|creator| creator.get("id"))
            .or_else(|| field(&metadata, "creator").and_then(|creator| creator.get("id")));
        let timestamp = scheduled
            .or_else(|| {
                field(&metadata, "postedAtMilli")
                    .and_then(Value::as_f64)
                    .map(|milli| (milli / 1000.0) as i64)
            })
            .or_else(|| unified_timestamp(field(&metadata, "creationDateTime")));
        let premium = field(&metadata, "premiumPlan").is_some_and(truthy);

        let title = field(&metadata, "title")
            .filter(|value| value.is_string())
            .or_else(|| content.get("contentTitle"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let thumbnail = url_or_none(field(&metadata, "thumbnail"))
            .or_else(|| url_or_none(content.get("thumbnailUrl1")));
        let description = field(&metadata, "description")
            .filter(|value| value.is_string())
            .or_else(|| content.get("contentDescription"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let tags: Vec<Value> = field(&metadata, "tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| tag.get("title").and_then(Value::as_str).map(str::to_owned))
            .map(Value::from)
            .collect();
        let fields: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("title", title.map_or(Value::Null, Value::from)),
            ("duration", float_or_none(content.get("duration"))),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            ("description", description.map_or(Value::Null, Value::from)),
            ("like_count", int_or_none(field(&metadata, "likeCount"))),
            (
                "dislike_count",
                int_or_none(field(&metadata, "dislikeCount")),
            ),
            ("channel", str_or_none(channel)),
            ("channel_id", str_or_none(channel_id)),
            (
                "channel_url",
                uploader
                    .map(|name| format!("https://rokfin.com/{name}"))
                    .map_or(Value::Null, Value::from),
            ),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
            (
                "release_timestamp",
                if live_status != "not_live" {
                    timestamp.map_or(Value::Null, Value::from)
                } else {
                    Value::Null
                },
            ),
            ("tags", Value::Array(tags)),
            ("live_status", live_status.into()),
            (
                "availability",
                if premium { "premium_only" } else { "public" }.into(),
            ),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.webpage_url = url.clone();
        Ok(Some(ExtractResult::Video(info)))
    }
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

impl Extractor for RokfinStack {
    fn key(&self) -> &'static str {
        STACK_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = STACK_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let list_id = found["id"].to_owned();
        crate::screen::set_extractor(STACK_NAME);
        crate::screen::screen(&format!(
            "[{STACK_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let metadata = api_json(client, &format!("stack/{list_id}"), &list_id)?;
        let mut entries = Vec::new();
        for content in metadata
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let media_type = match content.get("mediaType").and_then(Value::as_str) {
                Some("video" | "audio") => Some("post"),
                Some("stream" | "dead_stream") => Some("stream"),
                Some("stack") => Some("stack"),
                _ => None,
            };
            let Some(media_type) = media_type else {
                continue;
            };
            let video_id = if media_type == "post" {
                content.get("id")
            } else {
                content.get("mediaId")
            };
            let video_id = match video_id {
                Some(Value::Number(number)) => number.to_string(),
                Some(Value::String(text)) if !text.is_empty() => text.clone(),
                _ => continue,
            };
            let title = content
                .get("content")
                .and_then(|inner| inner.get("contentTitle"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            entries.push(url_entry(
                &format!("https://rokfin.com/{media_type}/{video_id}"),
                &format!("{media_type}/{video_id}"),
                title,
            )?);
        }
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: list_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: STACK_NAME.into(),
            extractor_key: "RokfinStack".into(),
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
            "https://rokfin.com/post/223/Julian-Assange-Arrested-Streaming-In-Real-Time"
        ));
        assert!(suitable(
            "https://www.rokfin.com/stream/10543/Its-A-Crazy-Mess"
        ));
        assert!(!suitable("https://rokfin.com/stack/271/Tulsi"));
        assert!(stack_suitable(
            "https://www.rokfin.com/stack/271/Tulsi-Gabbard-Portsmouth-Townhall-FULL--Feb-9-2020"
        ));
        assert!(!stack_suitable("https://rokfin.com/post/223"));
    }
}
