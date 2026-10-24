//! Upstream `BigoIE` (`yt_dlp/extractor/bigo.py`): a room's
//! `getInternalStudioInfo` (POSTed `siteId`) live HLS source and metadata.
use super::{Extractor, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Bigo;

const NAME: &str = "Bigo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?bigo\.tv/(?:[a-z]{2,}/)?(?P<id>[^/]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_download_json(API, user_id, data=urlencode_postdata({'siteId': ...}),
/// headers={'Accept': 'application/json'})`.
fn studio_info(client: &Client, id: &str) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
    let api = Url::parse("https://ta.bigo.tv/official_website/studio/getInternalStudioInfo")?;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("siteId", id)
        .finish();
    let mut response = super::direct::post(
        client,
        &api,
        &[
            ("Accept", "application/json"),
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

/// A truthy Python value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

/// `str(value)` of a JSON scalar, as Python formats it.
fn python_str(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `value or ...`: a non-empty string.
fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

impl Extractor for Bigo {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let user_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let info_raw = studio_info(client, &user_id)?;
        let Value::Object(info_raw) = info_raw else {
            return Err(Error::Reported(format!(
                "[{NAME}] {user_id}: Received invalid JSON data; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        if truthy(info_raw.get("code")) {
            return Err(Error::Reported(format!(
                "[{NAME}] {user_id}: Bigo says: {} (code {})",
                python_str(info_raw.get("msg").unwrap_or(&Value::Null)),
                python_str(&info_raw["code"])
            )));
        }
        let data = info_raw
            .get("data")
            .filter(|data| truthy(Some(data)))
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        if !truthy(data.get("alive")) {
            return Err(Error::Reported(format!(
                "[{NAME}] {user_id}: The channel is not currently live"
            )));
        }
        let manifest = data
            .get("hls_src")
            .and_then(Value::as_str)
            .and_then(|text| Url::parse(text).ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(format!(
                    "Bigo rooms without an absolute hls_src are not supported natively: {user_id}"
                ))
            })?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &user_id, None, true)?;
        // `ext='mp4', entry_protocol='m3u8'`.
        for format in &mut info.formats {
            format.ext = "mp4".into();
            format.protocol = Some("m3u8".into());
        }
        info.ext = "mp4".into();
        let nick_name = text(data.get("nick_name"));
        info.id = text(data.get("roomId")).unwrap_or_else(|| user_id.clone());
        info.title = text(data.get("roomTopic"))
            .or_else(|| nick_name.clone())
            .unwrap_or_else(|| user_id.clone());
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.thumbnail = data
            .get("snapshot")
            .filter(|value| !value.is_null())
            .map(python_str);
        let uploader = data.get("nick_name").cloned().unwrap_or(Value::Null);
        crate::metadata_parser::set_field(&mut info, "uploader", uploader);
        crate::metadata_parser::set_field(&mut info, "uploader_id", Value::String(user_id));
        crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(true));
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,formats,subtitles,thumbnail,uploader,uploader_id,is_live",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.bigo.tv/ja/221338632"));
        assert!(suitable("https://www.bigo.tv/th/Tarlerm1304"));
        assert!(suitable("https://bigo.tv/115976881"));
        assert!(!suitable("https://www.bigo.tv/"));
    }
}
