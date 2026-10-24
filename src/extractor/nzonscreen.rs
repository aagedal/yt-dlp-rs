//! Upstream `NZOnScreenIE` (`yt_dlp/extractor/nzonscreen.py`): a title
//! page's `data-video-config` JSON, its `h264` `lo_res`/`hi_res` URLs as
//! ranked formats with the site's `Referer`/`Origin` headers.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NzOnScreen;

const NAME: &str = "NZOnScreen";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://www\.nzonscreen\.com/title/(?P<id>[^/?#]+)").unwrap());
static CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"data-video-config='([^']+)'").unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<title\b[^>]*>([^<]+)</title>").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `strip_or_none`: a string's stripped text, else `None`.
fn strip(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => text.trim().into(),
        _ => Value::Null,
    }
}

/// `float_or_none(value, invscale=scale)`.
fn float(value: Option<&Value>, scale: f64) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        Some(Value::Bool(value)) => Some(f64::from(u8::from(*value))),
        _ => None,
    };
    number.map_or(Value::Null, |number| (number * scale).into())
}

/// `int_or_none(value)`.
fn int(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

impl Extractor for NzOnScreen {
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
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `_parse_json(_html_search_regex(...), video_id)`.
        let config = CONFIG
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract media data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let playlist: Value = serde_json::from_str(&config)
            .map_err(|_| Error::Reported(format!("[{NAME}] {video_id}: Failed to parse JSON")))?;
        let Some(uuid) = playlist.get("uuid") else {
            return Err(Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('uuid')); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let Some(uuid) = uuid.as_str() else {
            return Err(Error::UnsupportedHtml5(
                "NZOnScreen uuids that are not strings".into(),
            ));
        };
        // `_html_extract_title(webpage, default=None) or _og_search_title(webpage)`.
        let page_title = TITLE
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .filter(|title| !title.is_empty());
        let page_title = match page_title {
            Some(title) => title,
            None => crate::html::og_search_property(&webpage, "title")
                .filter(|title| !title.is_empty())
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] {video_id}: Unable to extract OpenGraph title; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?,
        };
        let alt_title = page_title
            .strip_suffix(" | NZ On Screen")
            .unwrap_or(&page_title)
            .trim()
            .to_owned();
        let headers = json!({
            "Referer": "https://www.nzonscreen.com/",
            "Origin": "https://www.nzonscreen.com/",
        });
        let h264 = playlist.get("h264");
        let mut formats = Vec::new();
        for (id, key) in [("lo", "lo_res"), ("hi", "hi_res")] {
            let Some(media) = h264
                .and_then(|h264| h264.get(key))
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let hi = id == "hi";
            let mut format = dict_format(
                vec![
                    ("url", media.into()),
                    ("format_id", id.into()),
                    ("ext", "mp4".into()),
                    ("quality", Value::from(formats.len())),
                    (
                        "height",
                        if hi {
                            int(playlist.get("height"))
                        } else {
                            Value::Null
                        },
                    ),
                    (
                        "width",
                        if hi {
                            int(playlist.get("width"))
                        } else {
                            Value::Null
                        },
                    ),
                    (
                        "filesize_approx",
                        float(
                            h264.and_then(|h264| h264.get(format!("{id}_res_mb"))),
                            1024.0 * 1024.0,
                        ),
                    ),
                ],
                NAME,
                uuid,
            )?;
            if let Value::Object(headers) = &headers {
                format.http_headers = Some(headers.clone());
            }
            formats.push(format);
        }
        let mut fields = vec![
            ("id", uuid.into()),
            ("display_id", video_id.into()),
            ("title", strip(playlist.get("label"))),
            ("description", strip(playlist.get("description"))),
            ("alt_title", alt_title.into()),
            (
                "thumbnail",
                playlist
                    .pointer("/thumbnail/path")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            ("duration", float(playlist.get("duration"), 1.0)),
            ("formats", Value::Null),
            ("http_headers", headers),
        ];
        // A `None` duration is dropped like upstream's.
        fields.retain(|(key, value)| !(*key == "duration" && value.is_null()));
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.nzonscreen.com/title/shoop-shoop-diddy-wop-cumma-cumma-wang-dang-1982"
        ));
        assert!(suitable(
            "https://www.nzonscreen.com/title/shes-a-mod-1964?collection=best-of-the-60s"
        ));
        assert!(!suitable("https://nzonscreen.com/title/x"));
    }
}
