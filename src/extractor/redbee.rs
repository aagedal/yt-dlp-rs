//! Upstream `ParliamentLiveUKIE` (`yt_dlp/extractor/redbee.py`): UK
//! Parliament event pages through the RedBee exposure API (an anonymous
//! session, then the asset's play formats) with the non-fatal
//! `GetShareVideo` metadata. Only HLS play formats are ported: DASH and
//! Smooth Streaming formats, DRM-marked formats and the RTBF login flow
//! (`RedBeeBaseIE` subclasses other than this one) are not supported
//! natively and error instead of guessing.
use super::{
    Extractor, dict_info, download_json_headers, extract_m3u8_master_optional,
    post_json_note_headers,
};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub struct ParliamentLiveUk;

/// `IE_NAME` (the error prefix and `extractor`) and the class key.
const NAME: &str = "parliamentlive.tv";
const KEY: &str = "ParliamentLiveUK";
const CUSTOMER: &str = "UKParliament";
const BUSINESS_UNIT: &str = "ParliamentLive";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"(?i)https?://(?:www\.)?parliamentlive\.tv/Event/Index/(?P<id>[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12})",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("RedBee {what} are not supported natively"))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_API_URL`.
fn api_url() -> String {
    format!("https://exposure.api.redbee.live/v2/customer/{CUSTOMER}/businessunit/{BUSINESS_UNIT}")
}

/// `str(uuid.uuid4())`: a random UUID-shaped device identifier per run.
fn device_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bits = [0u64; 2];
    for (index, slot) in bits.iter_mut().enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_usize(index);
        *slot = hasher.finish();
    }
    let hex = format!("{:016x}{:016x}", bits[0], bits[1]);
    format!(
        "{}-{}-4{}-8{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

/// `_get_bearer_token(asset_id)`: the anonymous `sessionToken`.
fn bearer_token(client: &Client, id: &str) -> Result<String> {
    let device = device_id();
    let request = json!({
        "deviceId": device,
        "device": {
            "deviceId": device,
            "name": "Mozilla Firefox 102",
            "type": "WEB",
        },
    });
    let response = post_json_note_headers(
        client,
        &Url::parse(&format!("{}/auth/anonymous", api_url()))?,
        NAME,
        id,
        "Downloading JSON metadata",
        &[("Content-Type", "application/json;charset=utf-8")],
        request.to_string(),
    )?;
    response
        .get("sessionToken")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| key_error(id, "sessionToken"))
}

/// `_download_json(..., fatal=False)` for `GetShareVideo`: `None` with a
/// warning on HTTP errors.
fn download_share_info(client: &Client, id: &str) -> Result<Option<Value>> {
    let url = Url::parse(&format!(
        "https://www.parliamentlive.tv/Event/GetShareVideo/{id}"
    ))?;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
    let mut response = match super::direct::request(client, &url, None) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, response.headers());
    crate::json::from_str(&text)
        .map(Some)
        .map_err(|_| unsupported("metadata responses that are not JSON"))
}

impl Extractor for ParliamentLiveUk {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let token = bearer_token(client, &id)?;
        let play = download_json_headers(
            client,
            &Url::parse(&format!("{}/entitlement/{id}/play", api_url()))?,
            NAME,
            &id,
            &[
                ("Authorization", &format!("Bearer {token}")),
                ("Accept", "application/json, text/plain, */*"),
            ],
        )?;

        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        let entries = play
            .get("formats")
            .and_then(Value::as_array)
            .ok_or_else(|| key_error(&id, "formats"))?;
        for format in entries {
            let Some(locator) = format.get("mediaLocator").and_then(Value::as_str) else {
                continue;
            };
            if format.get("drm").is_some_and(super::nuxt::truthy) {
                return Err(unsupported("DRM-protected formats"));
            }
            match format.get("format").and_then(Value::as_str) {
                Some("HLS") => {
                    let manifest = Url::parse(locator)?;
                    // `_get_formats_and_subtitles` passes no `m3u8_id`.
                    if let Some(mut stream) = extract_m3u8_master_optional(
                        client, &manifest, NAME, &id, None, true, false,
                    ) {
                        // `_get_formats_and_subtitles` passes no `ext`.
                        super::hls::without_ext(&mut stream);
                        formats.extend(stream.formats);
                        super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                    }
                }
                Some("DASH" | "SMOOTHSTREAMING") => {
                    return Err(unsupported("DASH and Smooth Streaming play formats"));
                }
                _ => {}
            }
        }

        let share = download_share_info(client, &id)?.unwrap_or(Value::Null);
        let text = |value: Option<&Value>| value.filter(|value| value.is_string()).cloned();
        let title = text(share.pointer("/event/title")).unwrap_or(Value::Null);
        let thumbnail = text(share.get("thumbnailUrl")).unwrap_or(Value::Null);
        let timestamp = share
            .pointer("/event/publishedStartTime")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, true))
            .map_or(Value::Null, Value::from);

        let fields = vec![
            ("id", Value::from(id.as_str())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("title", title),
            ("thumbnail", thumbnail),
            ("timestamp", timestamp),
        ];
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        info.subtitles = subtitles;
        // `_format_sort_fields` (`res`, `proto`).
        info.format_sort_fields = vec!["res".into(), "proto".into()];
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://parliamentlive.tv/Event/Index/c1e9d44d-fd6c-4263-b50f-97ed26cc998b"
        ));
        assert!(suitable(
            "https://parliamentlive.tv/event/index/27cf25e4-e77b-42a3-93c5-c815cd6d7377"
        ));
        assert!(!suitable(
            "https://parliamentlive.tv/Event/Index/not-a-uuid"
        ));
    }

    #[test]
    fn device_ids_are_uuid_shaped() {
        let id = device_id();
        assert_eq!(id.len(), 36);
        assert_eq!(id.matches('-').count(), 4);
    }
}
