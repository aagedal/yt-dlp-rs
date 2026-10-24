//! Upstream `EttuTvIE` (`yt_dlp/extractor/ettutv.py`): a player page's
//! `player-settings`, its POSTed `streamAccess` HLS stream and metadata.
use super::{Extractor, download_json, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct EttuTv;

const NAME: &str = "EttuTv";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?ettu\.tv/[^?#]+/playerpage/(?P<id>[0-9]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// An uncaught `KeyError(KEY)`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_download_json(url, video_id, data=b'')`.
fn post_json(client: &Client, url: &Url, id: &str) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
    let mut response = super::direct::post(
        client,
        url,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        String::new(),
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

impl Extractor for EttuTv {
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
        let api = Url::parse(&format!(
            "https://www.ettu.tv/api/v3/contents/{id}/player-settings?language=en&showTitle=true&device=desktop"
        ))?;
        let settings = download_json(client, &api, NAME, &id)?;
        let access = settings
            .get("streamAccess")
            .ok_or_else(|| key_error(&id, "streamAccess"))?;
        let access = access
            .as_str()
            .and_then(|text| Url::parse(text).ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(format!(
                    "EttuTv streamAccess values that are not absolute URLs are not supported natively: {id}"
                ))
            })?;
        let stream_response = post_json(client, &access, &id)?;
        let stream = stream_response
            .get("data")
            .ok_or_else(|| key_error(&id, "data"))?
            .get("stream")
            .ok_or_else(|| key_error(&id, "stream"))?;
        let stream = stream
            .as_str()
            .and_then(|text| Url::parse(text).ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(format!(
                    "EttuTv streams that are not absolute URLs are not supported natively: {id}"
                ))
            })?;
        let mut info = extract_m3u8_fatal(client, &stream, NAME, &id, None, true)?;
        // `ext='mp4'`.
        for format in &mut info.formats {
            format.ext = "mp4".into();
        }
        info.ext = "mp4".into();
        info.id = id;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let mut order = vec!["id", "formats", "subtitles"];
        match settings.get("title").filter(|value| !value.is_null()) {
            Some(Value::String(title)) => {
                info.title = title.clone();
                order.push("title");
            }
            Some(other) => {
                return Err(Error::UnsupportedHtml5(format!(
                    "EttuTv titles that are not strings are not supported natively: {other}"
                )));
            }
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        let description = settings
            .pointer("/metaInformation/competition")
            .filter(|value| !value.is_null())
            .cloned();
        let thumbnail = settings
            .get("image")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map(Value::from);
        let timestamp = settings
            .get("date")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, false))
            .map(Value::from);
        let is_live = settings
            .get("isLivestream")
            .filter(|value| value.is_boolean())
            .cloned();
        for (key, value) in [
            ("description", description),
            ("thumbnail", thumbnail),
            ("timestamp", timestamp),
            ("is_live", is_live),
        ] {
            if let Some(value) = value {
                crate::metadata_parser::set_field(&mut info, key, value);
                order.push(key);
            }
        }
        // A missing `title` key gets processing's generic title last.
        let untitled = !order.contains(&"title");
        let order = format!(
            "dict;{}{}",
            order.join(","),
            if untitled { "-untitled" } else { "" }
        );
        Ok(Some(crate::info_order::mark_info(info, &order)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.ettu.tv/en-int/playerpage/1573849"));
        assert!(!suitable("https://www.ettu.tv/en-int/playerpage/"));
    }
}
