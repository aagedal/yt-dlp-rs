//! Upstream `ViouslyIE` (`yt_dlp/extractor/viously.py`): no URLs of its own
//! (`_VALID_URL = False`); the generic extractor's embed detection gives
//! each `viously-player`/`vsly-player` div ID's non-fatal HLS formats with
//! the custom-alphabet base64 export JSON's metadata.
use super::html5::WebpageEntry;
use crate::{Error, Result};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub(crate) const NAME: &str = "Viously";

static PLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div[^>]*class="(?:[^"]*\s)?v(?:iou)?sly-player(?:\s[^"]*)?"[^>]*>"#).unwrap()
});

const STANDARD_ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
const CUSTOM_ALPHABET: &str = "VIOUSLYABCDEFGHJKMNPQRTWXZviouslyabcdefghjkmnpqrtwxz9876543210+/=";

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Viously {what} are not supported natively"))
}

/// `custom_decode`: the custom alphabet translated to standard base64,
/// decoded as UTF-8 with NULs stripped. Malformed exports (upstream's
/// Python-worded `Failed to parse JSON` warnings) remain explicit errors.
fn custom_decode(text: &str) -> Result<String> {
    let translated: String = text
        .chars()
        .map(|c| {
            CUSTOM_ALPHABET
                .find(c)
                .and_then(|index| STANDARD_ALPHABET[index..].chars().next())
                .unwrap_or(c)
        })
        // `b64decode` (non-strict) discards characters outside the alphabet.
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        .collect();
    let data = base64::engine::general_purpose::STANDARD
        .decode(translated)
        .map_err(|_| unsupported("exports that are not valid base64"))?;
    let text = String::from_utf8(data).map_err(|_| unsupported("exports that are not UTF-8"))?;
    Ok(text.trim_matches('\0').to_owned())
}

/// `_download_json(url, video_id, transform_source=custom_decode,
/// fatal=False)`: HTTP errors are warnings.
fn export_json(client: &Client, url: &Url, id: &str) -> Result<Option<Value>> {
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
    let mut response = match super::direct::request(client, url, None) {
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
    let text = custom_decode(&super::decode_webpage(&body, response.headers()))?;
    crate::json::from_str(&text)
        .map(Some)
        .map_err(|_| unsupported("exports that are not JSON"))
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

/// `traverse_obj(data, ('video', {...}))`.
fn metadata(data: Option<&Value>) -> Vec<(&'static str, Value)> {
    let mut fields = Vec::new();
    let Some(video) = data.and_then(|data| data.get("video")) else {
        return fields;
    };
    for key in ["title", "description"] {
        if let Some(Value::String(text)) = video.get(key) {
            fields.push((key, text.clone().into()));
        }
    }
    if let Some(duration) = int_or_none(video.get("duration")) {
        fields.push(("duration", duration.into()));
    }
    if let Some(timestamp) = video
        .get("iso_date")
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
    {
        fields.push(("timestamp", timestamp.into()));
    }
    if let Some(Value::String(category)) = video.pointer("/category/name")
        && !category.is_empty()
    {
        fields.push(("categories", Value::Array(vec![category.clone().into()])));
    }
    fields
}

/// `_extract_from_webpage`: every player div with an `id`, skipping those
/// whose HLS playlist gives no formats.
pub(crate) fn webpage_entries(
    client: &Client,
    _url: &Url,
    webpage: &str,
) -> Result<Vec<WebpageEntry>> {
    let mut entries = Vec::new();
    for player in PLAYER.find_iter(webpage) {
        // `traverse_obj(players, (..., {extract_attributes}, 'id'))`.
        let video_id = match crate::html_elements::extract_attributes(player.as_str()).get("id") {
            None | Some(Value::Null) => continue,
            Some(Value::String(id)) => id.clone(),
            Some(_) => return Err(unsupported("player IDs that are not strings")),
        };
        let manifest = Url::parse(&format!(
            "https://www.viously.com/video/hls/{video_id}/index.m3u8"
        ))
        .map_err(|_| unsupported("player IDs that do not form URLs"))?;
        let formats = match super::extract_m3u8_master_optional(
            client, &manifest, NAME, &video_id, None, false, false,
        ) {
            Some(mut stream) => {
                super::hls::without_ext(&mut stream);
                stream.formats
            }
            None => Vec::new(),
        };
        if formats.is_empty() {
            continue;
        }
        let export = Url::parse(&format!("https://www.viously.com/export/json/{video_id}"))
            .map_err(|_| unsupported("player IDs that do not form URLs"))?;
        let data = export_json(client, &export, &video_id)?;
        let mut fields: Vec<(&'static str, Value)> =
            vec![("id", video_id.into()), ("formats", Value::Null)];
        fields.extend(metadata(data.as_ref()));
        // `add_default_extra_info(info, ie, None)`.
        fields.push(("extractor", NAME.into()));
        fields.push(("extractor_key", NAME.into()));
        entries.push(WebpageEntry {
            extractor: (NAME, NAME),
            fields,
            formats,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_alphabet_round_trips() {
        // "{}" is standard base64 "e30=".
        let encoded: String = "e30="
            .chars()
            .map(|c| {
                let index = STANDARD_ALPHABET.find(c).unwrap();
                CUSTOM_ALPHABET[index..].chars().next().unwrap()
            })
            .collect();
        assert_eq!(custom_decode(&encoded).unwrap(), "{}");
    }
}
