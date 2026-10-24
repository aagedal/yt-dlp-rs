//! Upstream `DLiveVODIE` and `DLiveStreamIE` (`yt_dlp/extractor/dlive.py`):
//! a past broadcast's or a channel's live stream GraphQL JSON and its fatal
//! HLS playlist.
use super::cloudycdn::post_json;
use super::clyp::key_error;
use super::{Extractor, dict_info, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct DLiveVod;
pub struct DLiveStream;

const VOD: &str = "dlive:vod";
const VOD_KEY: &str = "DLiveVOD";
const STREAM: &str = "dlive:stream";
const STREAM_KEY: &str = "DLiveStream";
const API: &str = "https://graphigo.prd.dlive.tv/";

static VOD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?dlive\.tv/p/(?P<uploader_id>.+?)\+(?P<id>[^/?#&]+)").unwrap()
});
static STREAM_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?dlive\.tv/(?P<id>[\w.-]+)").unwrap());

/// `DLiveVODIE._match_valid_url`.
pub fn vod_suitable(url: &str) -> bool {
    VOD_URL.is_match(url)
}

/// `DLiveStreamIE._match_valid_url`: `dlive\.tv/(?!p/)ID`.
fn stream_id(url: &str) -> Option<String> {
    let found = STREAM_URL.captures(url)?;
    let id = found.name("id")?;
    (!url[id.start()..].starts_with("p/")).then(|| id.as_str().to_owned())
}

/// `DLiveStreamIE._match_valid_url`.
pub fn stream_suitable(url: &str) -> bool {
    stream_id(url).is_some()
}

/// `int_or_none(value, 1000)`.
fn int_scaled(value: Option<&Value>) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|number| number.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    };
    number.map_or(Value::Null, |number| number.div_euclid(1000).into())
}

/// `_download_json(API, id, data=json.dumps({'query': QUERY}).encode())`.
fn graphql(client: &Client, (extractor, id): (&'static str, &str), query: &str) -> Result<Value> {
    let body = format!(
        "{{\"query\": {}}}",
        serde_json::to_string(query).map_err(|error| Error::Reported(error.to_string()))?
    );
    post_json(client, &Url::parse(API)?, (extractor, id), body)
}

/// `value[key]`, a `KeyError`, or the `TypeError` of a `None` value.
fn index<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    match value {
        Value::Object(object) => object.get(key).ok_or_else(|| key_error(id, key)),
        Value::Null => Err(Error::Reported(
            "'NoneType' object is not subscriptable".into(),
        )),
        _ => Err(Error::UnsupportedHtml5(
            "DLive responses that are not objects are not supported natively".into(),
        )),
    }
}

fn text(value: &Value, what: &str) -> Result<String> {
    value.as_str().map(str::to_owned).ok_or_else(|| {
        Error::UnsupportedHtml5(format!(
            "DLive {what} that are not strings are not supported natively"
        ))
    })
}

fn get(value: &Value, key: &str) -> Value {
    value.get(key).cloned().unwrap_or(Value::Null)
}

impl Extractor for DLiveVod {
    fn key(&self) -> &'static str {
        VOD_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VOD_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let uploader_id = found["uploader_id"].to_owned();
        let vod_id = found["id"].to_owned();
        crate::screen::set_extractor(VOD);
        crate::screen::screen(&format!(
            "[{VOD}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let query = format!(
            "query {{\n  pastBroadcast(permlink:\"{uploader_id}+{vod_id}\") {{\n    content\n    createdAt\n    length\n    playbackUrl\n    title\n    thumbnailUrl\n    viewCount\n  }}\n}}"
        );
        let response = graphql(client, (VOD, &vod_id), &query)?;
        let broadcast = index(index(&response, "data", &vod_id)?, "pastBroadcast", &vod_id)?;
        let title = index(broadcast, "title", &vod_id)?.clone();
        let manifest = text(index(broadcast, "playbackUrl", &vod_id)?, "playback URLs")?;
        let mut stream =
            extract_m3u8_fatal(client, &Url::parse(&manifest)?, VOD, &vod_id, None, false)?;
        for format in &mut stream.formats {
            format.ext = "mp4".into();
        }
        Ok(Some(dict_info(
            url,
            (VOD, VOD_KEY),
            vec![
                ("id", vod_id.clone().into()),
                ("title", title),
                ("uploader_id", uploader_id.into()),
                ("formats", Value::Null),
                ("description", get(broadcast, "content")),
                ("thumbnail", get(broadcast, "thumbnailUrl")),
                ("timestamp", int_scaled(broadcast.get("createdAt"))),
                (
                    "view_count",
                    super::streamable::int_or_none(broadcast.get("viewCount")),
                ),
            ],
            stream.formats,
        )))
    }
}

impl Extractor for DLiveStream {
    fn key(&self) -> &'static str {
        STREAM_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(display_name) = stream_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(STREAM);
        crate::screen::screen(&format!(
            "[{STREAM}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let query = format!(
            "query {{\n  userByDisplayName(displayname:\"{display_name}\") {{\n    livestream {{\n      content\n      createdAt\n      title\n      thumbnailUrl\n      watchingCount\n    }}\n    username\n  }}\n}}"
        );
        let response = graphql(client, (STREAM, &display_name), &query)?;
        let user = index(
            index(&response, "data", &display_name)?,
            "userByDisplayName",
            &display_name,
        )?;
        let livestream = index(user, "livestream", &display_name)?;
        let title = index(livestream, "title", &display_name)?.clone();
        let username = text(index(user, "username", &display_name)?, "usernames")?;
        let manifest = Url::parse(&format!(
            "https://live.prd.dlive.tv/hls/live/{username}.m3u8"
        ))?;
        let mut stream = extract_m3u8_fatal(client, &manifest, STREAM, &display_name, None, false)?;
        for format in &mut stream.formats {
            format.ext = "mp4".into();
        }
        Ok(Some(dict_info(
            url,
            (STREAM, STREAM_KEY),
            vec![
                ("id", display_name.clone().into()),
                ("title", title),
                ("uploader", display_name.into()),
                ("uploader_id", username.into()),
                ("formats", Value::Null),
                ("description", get(livestream, "content")),
                ("thumbnail", get(livestream, "thumbnailUrl")),
                ("is_live", Value::Bool(true)),
                ("timestamp", int_scaled(livestream.get("createdAt"))),
                (
                    "view_count",
                    super::streamable::int_or_none(livestream.get("watchingCount")),
                ),
            ],
            stream.formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(vod_suitable("https://dlive.tv/p/pdp+3mTzOl4WR"));
        assert!(vod_suitable("https://dlive.tv/p/pdpreplay+D-RD-xSZg"));
        assert!(!vod_suitable("https://dlive.tv/pdp"));
        assert_eq!(stream_id("https://dlive.tv/pdp").as_deref(), Some("pdp"));
        assert_eq!(
            stream_id("https://www.dlive.tv/p.q-r").as_deref(),
            Some("p.q-r")
        );
        assert!(!stream_suitable("https://dlive.tv/p/pdp+3mTzOl4WR"));
    }
}
