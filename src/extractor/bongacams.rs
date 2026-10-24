//! Upstream `BongaCamsIE` (`yt_dlp/extractor/bongacams.py`): the room's
//! `tools/amf.php` `getRoomData` POST and its live HLS playlist.
use super::{Extractor, extract_m3u8_fatal_live};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct BongaCams;

const NAME: &str = "BongaCams";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?P<host>(?:[^/]+\.)?bongacams\d*\.(?:com|net))/(?P<id>[^/?&#]+)")
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("BongaCams {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_download_json(url, id, data=urlencode_postdata(...), headers=...)`.
fn post_json(client: &Client, url: &Url, id: &str, body: String) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
    let mut response = super::direct::post(
        client,
        url,
        &[
            ("X-Requested-With", "XMLHttpRequest"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        body,
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: NAME,
            id: id.to_owned(),
            what: "JSON metadata",
            source: Box::new(error),
        },
        error => error,
    })?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

impl Extractor for BongaCams {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let host = found["host"].to_owned();
        let channel_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!("https://{host}/tools/amf.php"))?;
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("method", "getRoomData"),
                ("args[]", channel_id.as_str()),
                ("args[]", "false"),
            ])
            .finish();
        let amf = post_json(client, &api, &channel_id, body)?;
        let local = match &amf {
            Value::Object(map) => map
                .get("localData")
                .ok_or_else(|| key_error(&channel_id, "localData"))?,
            _ => return Err(unsupported("room data that are not objects")),
        };
        let server_url = match local {
            Value::Object(map) => map
                .get("videoServerUrl")
                .ok_or_else(|| key_error(&channel_id, "videoServerUrl"))?,
            _ => return Err(unsupported("local data that are not objects")),
        };
        let Value::String(server_url) = server_url else {
            return Err(unsupported("video server URLs that are not strings"));
        };
        let performer = amf.get("performerData");
        let uploader_id = performer
            .and_then(|data| data.get("username"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map_or_else(|| channel_id.clone(), str::to_owned);
        let uploader = performer
            .and_then(|data| data.get("displayName"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let like_count =
            super::streamable::int_or_none(performer.and_then(|data| data.get("loversCount")));
        let manifest = Url::parse(&format!(
            "{server_url}/hls/stream_{uploader_id}/playlist.m3u8"
        ))?;
        let mut info =
            extract_m3u8_fatal_live(client, &manifest, NAME, &channel_id, Some("hls"), false)?;
        // `ext='mp4'`.
        for format in &mut info.formats {
            format.ext = "mp4".into();
        }
        info.ext = "mp4".into();
        info.id = channel_id;
        info.title = uploader
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| uploader_id.clone());
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        match uploader {
            Some(uploader) => {
                crate::metadata_parser::set_field(&mut info, "uploader", uploader.into())
            }
            None => {
                info.extra.insert("uploader".into(), Value::Null);
            }
        }
        crate::metadata_parser::set_field(&mut info, "uploader_id", uploader_id.into());
        if like_count.is_null() {
            info.extra.insert("like_count".into(), Value::Null);
        } else {
            crate::metadata_parser::set_field(&mut info, "like_count", like_count);
        }
        info.metadata.age_limit = Some(18);
        crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(true));
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,uploader,uploader_id,like_count,age_limit,is_live,formats",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://de.bongacams.com/azumi-8"));
        assert!(suitable("https://cn.bongacams.com/azumi-8"));
        assert!(suitable("https://de.bongacams.net/claireashton"));
        assert!(suitable("https://bongacams2.com/x"));
        assert!(!suitable("https://bongacams.org/x"));
    }

    #[test]
    fn room_data_is_form_encoded() {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("method", "getRoomData"),
                ("args[]", "a b"),
                ("args[]", "false"),
            ])
            .finish();
        assert_eq!(body, "method=getRoomData&args%5B%5D=a+b&args%5B%5D=false");
    }
}
