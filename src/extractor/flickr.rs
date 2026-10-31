//! Upstream `FlickrIE` (`yt_dlp/extractor/flickr.py`): the page's API key
//! from `hermes_error_beacon.gne`, then the REST API's `photos.getInfo` and
//! (for videos) `video.getStreamInfo`, with streams ranked by `qualities`.
use super::clyp::key_error;
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_json_note};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Flickr;

const NAME: &str = "Flickr";
const API_BASE_URL: &str = "https://api.flickr.com/services/rest?";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.|secure\.)?flickr\.com/photos/[\w\-_@]+/(?P<id>\d+)",
    )
    .unwrap()
});

const QUALITIES: [&str; 10] = [
    "288p",
    "iphone_wifi",
    "100",
    "300",
    "700",
    "360p",
    "appletv",
    "720p",
    "1080p",
    "orig",
];

/// `_LICENSES` (https://help.yahoo.com/kb/flickr/SLN25525.html).
const LICENSES: [(&str, &str); 11] = [
    ("0", "All Rights Reserved"),
    ("1", "Attribution-NonCommercial-ShareAlike"),
    ("2", "Attribution-NonCommercial"),
    ("3", "Attribution-NonCommercial-NoDerivs"),
    ("4", "Attribution"),
    ("5", "Attribution-ShareAlike"),
    ("6", "Attribution-NoDerivs"),
    ("7", "No known copyright restrictions"),
    ("8", "United States government work"),
    ("9", "Public Domain Dedication (CC0)"),
    ("10", "Public Domain Work"),
];

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Flickr {what} are not supported natively"))
}

/// `dict.get(key, {})` on a value that must be a dict when present.
fn object<'a>(value: &'a Value, key: &str) -> Result<Option<&'a Value>> {
    match value.get(key) {
        None => Ok(None),
        Some(inner) if inner.is_object() => Ok(Some(inner)),
        Some(_) => Err(unsupported("info fields that are not objects")),
    }
}

/// `_call_api`.
fn call_api(
    client: &Client,
    method: &str,
    video_id: &str,
    api_key: &str,
    note: &str,
    secret: Option<&str>,
) -> Result<Value> {
    let method = format!("flickr.{method}");
    let mut query = vec![
        ("photo_id", video_id),
        ("method", method.as_str()),
        ("api_key", api_key),
        ("format", "json"),
        ("nojsoncallback", "1"),
    ];
    if let Some(secret) = secret.filter(|secret| !secret.is_empty()) {
        query.push(("secret", secret));
    }
    let url = Url::parse(&format!(
        "{API_BASE_URL}{}",
        super::login::urlencode(&query)
    ))?;
    let data = download_json_note(client, &url, NAME, video_id, note)?;
    let stat = data
        .get("stat")
        .ok_or_else(|| key_error(video_id, "stat"))?;
    if stat != "ok" {
        let message = data
            .get("message")
            .ok_or_else(|| key_error(video_id, "message"))?;
        let message = message
            .as_str()
            .ok_or_else(|| unsupported("API messages that are not strings"))?;
        return Err(Error::Reported(format!(
            "[{NAME}] {video_id}: {message}; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    Ok(data)
}

impl Extractor for Flickr {
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
        let beacon = Url::parse("https://www.flickr.com/hermes_error_beacon.gne")?;
        let beacon = download_json_note(client, &beacon, NAME, &video_id, "Downloading api key")?;
        let api_key = beacon
            .get("site_key")
            .ok_or_else(|| key_error(&video_id, "site_key"))?
            .as_str()
            .ok_or_else(|| unsupported("API keys that are not strings"))?
            .to_owned();
        let info = call_api(
            client,
            "photos.getInfo",
            &video_id,
            &api_key,
            "Downloading video info",
            None,
        )?;
        let video_info = info
            .get("photo")
            .ok_or_else(|| key_error(&video_id, "photo"))?;
        let media = video_info
            .get("media")
            .ok_or_else(|| key_error(&video_id, "media"))?;
        if media != "video" {
            return Err(Error::Reported(format!("[{NAME}] {video_id}: not a video")));
        }
        let secret = video_info
            .get("secret")
            .ok_or_else(|| key_error(&video_id, "secret"))?;
        let secret = match secret {
            Value::String(secret) => Some(secret.as_str()),
            Value::Null => None,
            _ => return Err(unsupported("photo secrets that are not strings")),
        };
        let streams = call_api(
            client,
            "video.getStreamInfo",
            &video_id,
            &api_key,
            "Downloading streams info",
            secret,
        )?;
        let streams = streams
            .get("streams")
            .ok_or_else(|| key_error(&video_id, "streams"))?
            .get("stream")
            .ok_or_else(|| key_error(&video_id, "stream"))?
            .as_array()
            .ok_or_else(|| unsupported("stream lists that are not arrays"))?;
        let mut formats = Vec::new();
        for stream in streams {
            // `str(stream.get('type'))`.
            let stream_type = match stream.get("type") {
                None | Some(Value::Null) => "None".to_owned(),
                Some(Value::String(text)) => text.clone(),
                Some(Value::Number(number)) => number.to_string(),
                Some(_) => return Err(unsupported("stream types that are not strings")),
            };
            let content = stream
                .get("_content")
                .ok_or_else(|| key_error(&video_id, "_content"))?;
            if !content.is_string() {
                return Err(unsupported("stream URLs that are not strings"));
            }
            let quality = QUALITIES
                .iter()
                .position(|known| *known == stream_type)
                .map_or(-1, |index| index as i64);
            formats.push(dict_format(
                vec![
                    ("format_id", stream_type.into()),
                    ("url", content.clone()),
                    ("quality", quality.into()),
                ],
                NAME,
                &video_id,
            )?);
        }
        let owner = object(video_info, "owner")?;
        let owner_field = |key: &str| {
            owner
                .and_then(|owner| owner.get(key))
                .cloned()
                .unwrap_or(Value::Null)
        };
        let uploader_id = owner_field("nsid");
        let uploader_path = Some(owner_field("path_alias"))
            .filter(super::nuxt::truthy)
            .unwrap_or_else(|| uploader_id.clone());
        let uploader_url = match &uploader_path {
            Value::String(path) if !path.is_empty() => {
                Value::String(format!("https://www.flickr.com/photos/{path}/"))
            }
            Value::Null => Value::Null,
            Value::String(_) => Value::Null,
            _ => return Err(unsupported("owner paths that are not strings")),
        };
        let title = video_info
            .get("title")
            .ok_or_else(|| key_error(&video_id, "title"))?
            .get("_content")
            .ok_or_else(|| key_error(&video_id, "_content"))?
            .clone();
        let content = |key: &str| -> Result<Option<Value>> {
            Ok(object(video_info, key)?.and_then(|inner| inner.get("_content").cloned()))
        };
        let tags = match object(video_info, "tags")?.and_then(|tags| tags.get("tag")) {
            None => Vec::new(),
            Some(Value::Array(tags)) => tags
                .iter()
                .map(|tag| tag.get("_content").cloned().unwrap_or(Value::Null))
                .collect(),
            Some(_) => return Err(unsupported("tag lists that are not arrays")),
        };
        let license = video_info
            .get("license")
            .and_then(Value::as_str)
            .and_then(|license| LICENSES.iter().find(|(key, _)| *key == license))
            .map_or(Value::Null, |(_, name)| Value::String((*name).into()));
        let duration =
            int_or_none(object(video_info, "video")?.and_then(|video| video.get("duration")));
        let info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", title),
                (
                    "description",
                    content("description")?.unwrap_or(Value::Null),
                ),
                ("formats", Value::Null),
                ("timestamp", int_or_none(video_info.get("dateuploaded"))),
                ("duration", duration),
                ("uploader_id", uploader_id),
                ("uploader", owner_field("realname")),
                ("uploader_url", uploader_url),
                ("comment_count", int_or_none(content("comments")?.as_ref())),
                ("view_count", int_or_none(video_info.get("views"))),
                ("tags", Value::Array(tags)),
                ("license", license),
            ],
            formats,
        );
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.flickr.com/photos/forestwander-nature-pictures/5645318632/in/photostream/"
        ));
        assert!(suitable(
            "https://secure.flickr.com/photos/10922353@N03/5645318632"
        ));
        assert!(!suitable(
            "https://www.flickr.com/photos/forestwander-nature-pictures/"
        ));
    }
}
