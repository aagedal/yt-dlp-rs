//! Upstream `DctpTvIE` (`yt_dlp/extractor/dctp.py`): the REST API's
//! version, slug and media JSON, and the medium's HLS, S3 and CDN format
//! URLs built from its UUID.
use super::clyp::key_error;
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_json_note};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct DctpTv;

const NAME: &str = "DctpTv";
const BASE_URL: &str = "http://dctp-ivms2-restapi.s3.amazonaws.com";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?dctp\.tv/(?:#/)?filme/(?P<id>[^/?#&]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("dctp.tv {what} are not supported natively"))
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// Python's `str(value)` for a JSON string or number.
fn python_str(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `url_or_none`.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

fn download(client: &Client, url: &str, id: &str, note: &str) -> Result<Value> {
    download_json_note(client, &Url::parse(url)?, NAME, id, note)
}

/// `add_formats(suffix)`: the HLS, S3 and CDN URLs of one rendition.
fn add_formats(
    formats: &mut Vec<crate::VideoFormat>,
    uuid: &str,
    suffix: &str,
    display_id: &str,
) -> Result<()> {
    let templ = |host: &str| format!("https://{host}/{uuid}_dctp_{suffix}.m4v");
    formats.push(dict_format(
        vec![
            ("format_id", format!("hls-{suffix}").into()),
            (
                "url",
                format!("{}/playlist.m3u8", templ("cdn-segments.dctp.tv")).into(),
            ),
            ("protocol", "m3u8_native".into()),
        ],
        NAME,
        display_id,
    )?);
    for (prefix, host) in [
        ("s3", "completed-media.s3.amazonaws.com"),
        ("http", "cdn-media.dctp.tv"),
    ] {
        formats.push(dict_format(
            vec![
                ("format_id", format!("{prefix}-{suffix}").into()),
                ("url", templ(host).into()),
            ],
            NAME,
            display_id,
        )?);
    }
    Ok(())
}

impl Extractor for DctpTv {
    fn key(&self) -> &'static str {
        NAME
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

        let version = download(
            client,
            &format!("{BASE_URL}/version.json"),
            &display_id,
            "Downloading version JSON",
        )?;
        let version_name = version
            .get("version_name")
            .ok_or_else(|| key_error(&display_id, "version_name"))?;
        let version_name = python_str(version_name).ok_or_else(|| unsupported("version names"))?;
        let restapi_base = format!("{BASE_URL}/{version_name}/restapi");

        let info = download(
            client,
            &format!("{restapi_base}/slugs/{display_id}.json"),
            &display_id,
            "Downloading video info JSON",
        )?;
        let object_id = info
            .get("object_id")
            .ok_or_else(|| key_error(&display_id, "object_id"))?;
        let object_id = python_str(object_id).ok_or_else(|| unsupported("object IDs"))?;

        let media = download(
            client,
            &format!("{restapi_base}/media/{object_id}.json"),
            &display_id,
            "Downloading media JSON",
        )?;
        let uuid = media
            .get("uuid")
            .ok_or_else(|| key_error(&display_id, "uuid"))?;
        let Value::String(uuid) = uuid else {
            return Err(unsupported("media UUIDs that are not strings"));
        };
        let title = media
            .get("title")
            .ok_or_else(|| key_error(&display_id, "title"))?
            .clone();
        let is_wide = media.get("is_wide").is_some_and(truthy);

        let mut formats = Vec::new();
        add_formats(
            &mut formats,
            uuid,
            &format!("0500_{}", if is_wide { "16x9" } else { "4x3" }),
            &display_id,
        )?;
        if is_wide {
            add_formats(&mut formats, uuid, "720p", &display_id)?;
        }

        let mut thumbnails = Vec::new();
        if let Some(Value::Array(images)) = media.get("images") {
            for image in images.iter().filter_map(Value::as_object) {
                let Some(image_url) = url_or_none(image.get("url")) else {
                    continue;
                };
                let mut thumbnail = Map::new();
                thumbnail.insert("url".into(), image_url.into());
                thumbnail.insert("width".into(), int_or_none(image.get("width")));
                thumbnail.insert("height".into(), int_or_none(image.get("height")));
                thumbnails.push(Value::Object(thumbnail));
            }
        }

        let description = media
            .get("description")
            .filter(|value| truthy(value))
            .or_else(|| media.get("teaser"))
            .cloned()
            .unwrap_or(Value::Null);
        let timestamp = match media.get("created") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(created)) => {
                crate::dates::unified_timestamp(created, false).map_or(Value::Null, Value::from)
            }
            Some(_) => return Err(unsupported("creation dates that are not strings")),
        };
        let duration = match media.get("duration_in_ms") {
            None | Some(Value::Null) => Value::Null,
            Some(value) => super::clyp::float_or_none(Some(value))
                .as_f64()
                .map_or(Value::Null, |ms| Value::from(ms / 1000.0)),
        };

        let info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", uuid.clone().into()),
                ("display_id", display_id.into()),
                ("title", title),
                (
                    "alt_title",
                    media.get("subtitle").cloned().unwrap_or(Value::Null),
                ),
                ("description", description),
                ("timestamp", timestamp),
                ("duration", duration),
                ("thumbnails", Value::Array(thumbnails)),
                ("formats", Value::Null),
            ],
            formats,
        );
        // The thumbnails' keys, `None` values kept.
        let shape = info
            .extra
            .get(crate::info_order::INFO_SHAPE)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("{shape};thumbnail!=url,width,height"),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.dctp.tv/filme/videoinstallation-fuer-eine-kaufhausfassade/"
        ));
        assert!(suitable("https://dctp.tv/#/filme/sind-youtuber"));
        assert!(!suitable("https://www.dctp.tv/serien/x"));
    }
}
