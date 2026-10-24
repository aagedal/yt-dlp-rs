//! Upstream `ErocastIE` (`yt_dlp/extractor/erocast.py`): a track page's
//! `song_data_ID` JSON, its HLS stream and metadata.
use super::{Extractor, download_webpage, extract_m3u8_fatal, search_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Erocast;

const NAME: &str = "Erocast";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?erocast\.me/track/(?P<id>[0-9]+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float.trunc() as i64))
            .map(Value::from),
        Value::String(text) => text.trim().parse::<i64>().ok().map(Value::from),
        Value::Bool(flag) => Some(Value::from(i64::from(*flag))),
        _ => None,
    }
}

/// `str_or_none(value)`.
fn str_or_none(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Null => None,
        Value::String(text) => Some(Value::from(text.clone())),
        Value::Bool(true) => Some("True".into()),
        Value::Bool(false) => Some("False".into()),
        Value::Number(number) => Some(Value::from(number.to_string())),
        _ => None,
    }
}

impl Extractor for Erocast {
    fn key(&self) -> &'static str {
        NAME
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
        let webpage = download_webpage(client, url, NAME, &id)?;
        let start = Regex::new(&format!(r"<script>\s*var song_data_{id}\s*="))
            .map_err(|error| Error::UnsupportedHtml5(error.to_string()))?;
        let data = match search_json(&start, &webpage, false) {
            Some(Some(data @ Value::Object(_))) => data,
            Some(_) => {
                return Err(Error::UnsupportedHtml5(format!(
                    "Erocast song data that is not a JSON object is not supported natively: {id}"
                )));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract data; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let stream = data
            .get("file_url")
            .filter(|value| match value {
                Value::Null | Value::Bool(false) => false,
                Value::String(text) => !text.is_empty(),
                _ => true,
            })
            .or_else(|| data.get("stream_url"))
            .and_then(Value::as_str)
            .and_then(|text| Url::parse(text).ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(format!(
                    "Erocast tracks without an absolute file_url or stream_url are not supported natively: {id}"
                ))
            })?;
        let mut info = extract_m3u8_fatal(client, &stream, NAME, &id, Some("hls"), false)?;
        // `ext='m4a'`.
        for format in &mut info.formats {
            format.ext = "m4a".into();
        }
        info.ext = "m4a".into();
        info.id = id;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.age_limit = Some(18);
        let text = |pointer: &str| {
            data.pointer(pointer)
                .and_then(Value::as_str)
                .map(|text| Value::from(text.to_owned()))
        };
        let url_value = |pointer: &str| {
            data.pointer(pointer)
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(Value::from)
        };
        let timestamp = |pointer: &str| {
            data.pointer(pointer)
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map(Value::from)
        };
        let title = text("/title");
        match title.as_ref().and_then(Value::as_str) {
            Some(title) => info.title = title.to_owned(),
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        let fields = [
            ("description", text("/description")),
            ("release_timestamp", timestamp("/created_at")),
            ("modified_timestamp", timestamp("/updated_at")),
            ("uploader", text("/user/name")),
            ("uploader_id", str_or_none(data.pointer("/user/id"))),
            ("uploader_url", url_value("/user/permalink_url")),
            ("thumbnail", url_value("/artwork_url")),
            ("duration", int_or_none(data.get("duration"))),
            ("view_count", int_or_none(data.get("plays"))),
            ("comment_count", int_or_none(data.get("comment_count"))),
        ];
        let mut order = vec!["id", "formats", "age_limit"];
        if title.is_some() {
            order.push("title");
        }
        for (key, value) in fields {
            if let Some(value) = value {
                crate::metadata_parser::set_field(&mut info, key, value);
                order.push(key);
            }
        }
        if let Some(permalink) = url_value("/permalink_url")
            .as_ref()
            .and_then(Value::as_str)
            .and_then(|text| Url::parse(text).ok())
        {
            info.webpage_url = permalink;
            order.push("webpage_url");
        }
        // A missing `title` key gets processing's generic title last.
        let order = format!(
            "dict;{}{}",
            order.join(","),
            if title.is_some() { "" } else { "-untitled" }
        );
        Ok(Some(crate::info_order::mark_info(info, &order)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://erocast.me/track/9787/f"));
        assert!(suitable("https://www.erocast.me/track/1"));
        assert!(!suitable("https://erocast.me/clarissaisshy"));
    }
}
