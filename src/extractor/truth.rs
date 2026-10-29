//! Upstream `TruthIE` (`yt_dlp/extractor/truth.py`): a status's API JSON,
//! fetched with impersonation, and its first media attachment as the
//! untitled URL result.
use super::{Extractor, direct, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Truth;

const NAME: &str = "Truth";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://truthsocial\.com/@[^/?#]+(?:/posts)?/(?P<id>\d+)").unwrap()
});

/// `_match_id`'s pattern is a match, so no ID is extracted from other URLs.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `with impersonation`: the API request upstream sends with
/// `impersonate=True`.
fn impersonated<T>(request: impl FnOnce() -> Result<T>) -> Result<T> {
    let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
        crate::screen::warning_once(&format!("[{NAME}] {message}"));
    });
    request()
}

/// `strip_or_none` over a string.
fn stripped(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Sets a field, keeping an upstream `None` as a null value at its place.
fn put(info: &mut VideoInfo, key: &str, value: Value) {
    if value.is_null() {
        info.extra.insert(key.into(), Value::Null);
    } else {
        crate::metadata_parser::set_field(info, key, value);
    }
}

impl Extractor for Truth {
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
        let api = Url::parse(&format!(
            "https://truthsocial.com/api/v1/statuses/{video_id}"
        ))?;
        let status = impersonated(|| download_json(client, &api, NAME, &video_id))?;
        let uploader_id = stripped(
            status
                .get("account")
                .and_then(|account| account.get("username")),
        );
        let media = status
            .get("media_attachments")
            .ok_or_else(|| super::clyp::key_error(&video_id, "media_attachments"))?;
        let attachment = match media {
            // `['media_attachments'][0]`: an empty list's IndexError.
            Value::Array(items) => items
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "Truth media attachments that are not lists are not supported natively".into(),
                ));
            }
        };
        let media_url = attachment
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| super::clyp::key_error(&video_id, "url"))?;
        let media_url = Url::parse(media_url)?;
        let ext = direct::determine_ext(media_url.as_str())
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        let mut info = direct::with_format(
            direct::build_at(media_url.clone(), &media_url, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
        }
        info.direct = None;
        info.id = video_id.clone();
        info.title = String::new();
        let description = status
            .get("content")
            .and_then(Value::as_str)
            .map(crate::html::clean_html)
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        let timestamp = status
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, false));
        let account = |key: &str| {
            status
                .get("account")
                .and_then(|account| account.get(key))
                .cloned()
        };
        let counts = |key: &str| super::streamable::int_or_none(status.get(key));
        put(
            &mut info,
            "description",
            description.map_or(Value::Null, Value::from),
        );
        put(
            &mut info,
            "timestamp",
            timestamp.map_or(Value::Null, Value::from),
        );
        put(
            &mut info,
            "uploader",
            stripped(account("display_name").as_ref()).map_or(Value::Null, Value::from),
        );
        put(
            &mut info,
            "uploader_id",
            uploader_id.as_deref().map_or(Value::Null, Value::from),
        );
        put(
            &mut info,
            "uploader_url",
            uploader_id.as_deref().map_or(Value::Null, |id| {
                format!("https://truthsocial.com/@{id}").into()
            }),
        );
        put(&mut info, "repost_count", counts("reblogs_count"));
        put(&mut info, "like_count", counts("favourites_count"));
        put(&mut info, "comment_count", counts("replies_count"));
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.webpage_url = url.clone();
        Ok(Some(crate::info_order::mark_info(
            info,
            "untyped-dict;id,url,title,description,timestamp,uploader,uploader_id,uploader_url,repost_count,like_count,comment_count",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://truthsocial.com/@realDonaldTrump/posts/108779000807761862"
        ));
        assert!(!suitable("https://truthsocial.com/@realDonaldTrump"));
    }
}
