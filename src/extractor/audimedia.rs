//! Upstream `AudiMediaIE` (`yt_dlp/extractor/audimedia.py`): Audi
//! MediaCenter videos from the page's `amtv-embed` payload and the
//! audimedia.tv video API: non-fatal HLS and HDS manifests, the video
//! versions' files, and the title, subtitle, thumbnail, date, duration and
//! view count.
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct AudiMedia;

const NAME: &str = "AudiMedia";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?audi-mediacenter\.com/(?:en|de)/audimediatv/(?:video/)?(?P<id>[^/?#]+)",
    )
    .unwrap()
});

/// The `raw payload` patterns, tried in order.
static PAYLOAD: LazyLock<[Regex; 5]> = LazyLock::new(|| {
    [
        r#"class="amtv-embed"[^>]+id="([0-9a-z-]+)""#,
        r#"id="([0-9a-z-]+)"[^>]+class="amtv-embed""#,
        r#"class=\\"amtv-embed\\"[^>]+id=\\"([0-9a-z-]+)\\""#,
        r#"id=\\"([0-9a-z-]+)\\"[^>]+class=\\"amtv-embed\\""#,
        r#"id=(?:\\)?"(amtve-[a-z]-\d+-[a-z]{2})"#,
    ]
    .map(|pattern| Regex::new(pattern).unwrap())
});

static BITRATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)k").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!(
        "Audi MediaCenter {what} are not supported natively"
    ))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `video_data.get(key)` as a non-empty string (Python truthiness).
fn text<'a>(data: &'a serde_json::Map<String, Value>, key: &str) -> Result<Option<&'a str>> {
    match data.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(unsupported(&format!("{key} values that are not strings"))),
    }
}

/// `_, stage_mode, video_id, _ = raw_payload.split('-')`.
fn split_payload(payload: &str) -> Result<(&str, &str)> {
    let parts: Vec<&str> = payload.split('-').collect();
    match parts.len() {
        4 => Ok((parts[1], parts[2])),
        got @ 0..4 => Err(Error::Reported(format!(
            "not enough values to unpack (expected 4, got {got})"
        ))),
        got => Err(Error::Reported(format!(
            "too many values to unpack (expected 4, got {got})"
        ))),
    }
}

impl Extractor for AudiMedia {
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
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let payload = PAYLOAD
            .iter()
            .find_map(|pattern| pattern.captures(&webpage))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract raw payload; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let (stage_mode, video_id) = split_payload(&payload[1])?;
        let video_id = video_id.to_owned();
        if matches!(stage_mode, "s" | "e") {
            // Upstream returns nothing for (ended) live streams.
            return Err(unsupported("live streams and ended live streams"));
        }
        let api = Url::parse(&format!(
            "https://www.audimedia.tv/api/video/v1/videos/{video_id}"
        ))
        .map_err(|_| unsupported("video IDs that do not form URLs"))?;
        let api = super::update_url_query_lists(
            &api,
            &[("embed[]", vec!["video_versions", "thumbnail_image"])],
        );
        let response = super::download_json(client, &api, NAME, &video_id)?;
        let video_data = match &response {
            Value::Object(response) => response
                .get("results")
                .ok_or_else(|| key_error(&display_id, "results"))?,
            _ => return Err(unsupported("API responses that are not objects")),
        };
        let Value::Object(video_data) = video_data else {
            return Err(unsupported("API results that are not objects"));
        };
        let mut formats = Vec::new();
        if let Some(hls) = text(video_data, "stream_url_hls")? {
            let manifest = Url::parse(hls).map_err(|_| unsupported("relative HLS URLs"))?;
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("hls"),
                false,
                false,
            ) {
                formats.extend(stream.formats);
            }
        }
        if let Some(hds) = text(video_data, "stream_url_hds")? {
            let manifest = Url::parse(&format!("{hds}?hdcore=3.4.0"))
                .map_err(|_| unsupported("relative HDS URLs"))?;
            formats.extend(super::extract_f4m_optional(
                client,
                NAME,
                &manifest,
                &video_id,
                Some("hds"),
            )?);
        }
        let versions = match video_data.get("video_versions") {
            None => &Vec::new(),
            Some(Value::Array(versions)) => versions,
            Some(_) => return Err(unsupported("video version lists that are not lists")),
        };
        for version in versions {
            let Value::Object(version) = version else {
                return Err(unsupported("video versions that are not objects"));
            };
            let version_url = match text(version, "download_url")? {
                Some(found) => found,
                None => match text(version, "stream_url")? {
                    Some(found) => found,
                    None => continue,
                },
            };
            let mut fields = vec![
                ("url", Value::from(version_url)),
                ("width", int_or_none(version.get("width"))),
                ("height", int_or_none(version.get("height"))),
                ("abr", int_or_none(version.get("audio_bitrate"))),
                ("vbr", int_or_none(version.get("video_bitrate"))),
            ];
            if let Some(bitrate) = BITRATE.captures(version_url) {
                fields.push(("format_id", format!("http-{}", &bitrate[1]).into()));
            }
            let mut format = dict_format(fields, NAME, &video_id)?;
            // `int_or_none` bitrates: processing's `tbr` sum stays an int.
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
        let title = match video_data.get("title") {
            Some(Value::String(title)) => title.clone(),
            Some(_) => return Err(unsupported("videos without a string title")),
            None => return Err(key_error(&display_id, "title")),
        };
        let thumbnail = match video_data.get("thumbnail_image") {
            None => Value::Null,
            Some(Value::Object(image)) => image.get("file").cloned().unwrap_or(Value::Null),
            Some(_) => return Err(unsupported("thumbnail images that are not objects")),
        };
        let timestamp = video_data
            .get("publication_date")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                (
                    "description",
                    video_data.get("subtitle").cloned().unwrap_or(Value::Null),
                ),
                ("thumbnail", thumbnail),
                ("timestamp", timestamp),
                ("duration", int_or_none(video_data.get("duration"))),
                ("view_count", int_or_none(video_data.get("view_count"))),
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
            "https://www.audi-mediacenter.com/en/audimediatv/video/60-seconds-of-audi-sport-104-2015-wec-bahrain-rookie-test-2991"
        ));
        assert!(!suitable(
            "https://www.audi-mediacenter.com/fr/audimediatv/x"
        ));
    }

    #[test]
    fn payloads_unpack_into_four_parts() {
        assert_eq!(split_payload("amtve-v-1565-en").unwrap(), ("v", "1565"));
        assert!(split_payload("a-b").is_err());
    }
}
