//! Upstream `MeipaiIE` (`yt_dlp/extractor/meipai.py`): 美拍 videos from the
//! page's live-recording HLS URL (else its `data-video` URL), with the
//! OpenGraph and `<meta>` metadata.
use super::{Extractor, dict_format, dict_info, download_webpage, search_warning};
use crate::{Error, PyNumber, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Meipai;

const NAME: &str = "Meipai";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?meipai\.com/media/(?P<id>[0-9]+)").unwrap()
});
/// `file:\s*encodeURIComponent\((["\'])(?P<url>(?:(?!\1).)+)\1\)` without
/// backreferences.
static M3U8_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"file:\s*encodeURIComponent\((?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')\)"#).unwrap()
});
/// `data-video=(["\'])(?P<url>(?:(?!\1).)+)\1` without backreferences.
static VIDEO_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-video=(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Meipai {what} are not supported natively"))
}

/// The quoted `url` group of a backreference-free pattern.
fn quoted(regex: &Regex, webpage: &str) -> Option<String> {
    let found = regex.captures(webpage)?;
    found
        .name("dq")
        .or_else(|| found.name("sq"))
        .map(|found| found.as_str().to_owned())
}

/// `_og_search_property(prop, webpage, name, fatal=False)`.
fn og_property(webpage: &str, prop: &str, name: &str) -> Option<String> {
    let value = crate::html::og_search_property(webpage, prop);
    if value.is_none() {
        search_warning(NAME, name);
    }
    value
}

/// `_html_search_meta(prop, webpage, name)`.
fn search_meta(webpage: &str, prop: &str, name: &str) -> Option<String> {
    let value = crate::html::html_search_meta_clean(webpage, prop);
    if value.is_none() {
        search_warning(NAME, name);
    }
    value
}

impl Extractor for Meipai {
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
        // `_generic_title('', webpage)`: no URL basename to fall back on.
        let title = crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
            .or_else(|| crate::html::html_extract_title(&webpage))
            .filter(|title| !title.is_empty());

        let mut formats = Vec::new();
        // Recorded playback of live streaming (`_html_search_regex`).
        let m3u8_url = quoted(&M3U8_URL, &webpage)
            .map(|url| crate::html::clean_html(&url))
            .filter(|url| !url.is_empty());
        if let Some(m3u8_url) = m3u8_url {
            let manifest = Url::parse(&m3u8_url)
                .map_err(|_| unsupported("HLS URLs that are not absolute URLs"))?;
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
        if formats.is_empty() {
            // Regular uploaded video.
            if let Some(video_url) = quoted(&VIDEO_URL, &webpage) {
                Url::parse(&video_url)
                    .map_err(|_| unsupported("video URLs that are not absolute URLs"))?;
                formats.push(dict_format(
                    vec![("url", video_url.into()), ("format_id", "http".into())],
                    NAME,
                    &video_id,
                )?);
            }
        }

        let timestamp = og_property(&webpage, "video:release_date", "release date")
            .and_then(|date| crate::dates::unified_timestamp(&date, true));
        let tags: Vec<Value> = crate::html::og_search_property(&webpage, "video:tag")
            .unwrap_or_default()
            .split(',')
            .map(Value::from)
            .collect();
        let view_count = search_meta(&webpage, "interactionCount", "view count")
            .and_then(|count| count.trim().parse::<i64>().ok());
        let duration = search_meta(&webpage, "duration", "duration")
            .and_then(|duration| crate::parse_duration(&duration))
            .map_or(Value::Null, |duration| match duration {
                PyNumber::Int(seconds) => Value::from(seconds),
                PyNumber::Float(seconds) => Value::from(seconds),
            });
        let creator = og_property(&webpage, "video:director", "creator");
        let description = og_property(&webpage, "description", "OpenGraph description");
        let thumbnail = og_property(&webpage, "image", "thumbnail URL");
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", optional(title)),
                ("description", optional(description)),
                ("thumbnail", optional(thumbnail)),
                ("duration", duration),
                ("timestamp", timestamp.map_or(Value::Null, Value::from)),
                ("view_count", view_count.map_or(Value::Null, Value::from)),
                ("creator", optional(creator)),
                ("tags", Value::Array(tags)),
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
        assert!(suitable("http://www.meipai.com/media/531697625"));
        assert!(!suitable("http://www.meipai.com/user/531697625"));
    }
}
