//! Upstream `PopcorntimesIE` (`yt_dlp/extractor/popcorntimes.py`):
//! Popcorntimes movies from the page's ROT13-shifted base64 `PCTMLOC`, with
//! the heading (or original-name meta) title, description, preview image and
//! the video meta tags.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Popcorntimes;

const NAME: &str = "Popcorntimes";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://popcorntimes\.tv/[^/]+/m/(?P<id>[^/]+)/(?P<display_id>[^/?#&]+)")
        .unwrap()
});
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<h1>([^<]+)").unwrap());
static LOC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"PCTMLOC\s*=\s*(["'])"#).unwrap());
static PREVIEW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<img[^>]+class=["']video-preview[^>]+\bsrc=(["'])"#).unwrap());
static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div[^>]+class=["']pt-movie-desc[^>]+>(.+?)</div>"#).unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `START(["'])(?P<value>(?:(?!\1).)+)\1`: the first quoted nonempty value.
fn quoted(start: &Regex, text: &str) -> Option<String> {
    let mut from = 0;
    while let Some(found) = start.captures_at(text, from) {
        let whole = found.get(0)?;
        let quote = found[1].chars().next()?;
        let rest = &text[whole.end()..];
        let value: String = rest
            .chars()
            .take_while(|c| *c != quote && *c != '\n')
            .collect();
        if !value.is_empty() && rest[value.len()..].starts_with(quote) {
            return Some(value);
        }
        from = whole.start() + 1;
    }
    None
}

/// The ROT13 letter shift before base64 decoding.
fn rot13(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            _ => c,
        })
        .collect()
}

impl Extractor for Popcorntimes {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let fail = |name: &str| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let title = HEADING
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .filter(|title| !title.is_empty())
            .or_else(|| crate::html::html_search_meta(&webpage, "ya:ovs:original_name"))
            .ok_or_else(|| fail("title"))?;
        let loc = quoted(&LOC, &webpage).ok_or_else(|| fail("loc"))?;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(rot13(&loc))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Popcorntimes locations that do not decode are not supported natively".into(),
                )
            })?;
        let description = DESCRIPTION
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]));
        if description.is_none() {
            super::search_warning(NAME, "description");
        }
        let thumbnail = quoted(&PREVIEW, &webpage).or_else(|| {
            let og = crate::html::og_search_property(&webpage, "image");
            if og.is_none() {
                super::search_warning(NAME, "thumbnail URL");
            }
            og
        });
        let meta = |name: &str| crate::html::html_search_meta(&webpage, name);
        let int_meta = |name: &str| {
            meta(name)
                .and_then(|value| value.trim().parse::<i64>().ok())
                .map_or(Value::Null, Value::from)
        };
        let release_date = meta("video:release_date")
            .filter(|date| !date.is_empty())
            .map(|date| date.replace('-', ""));
        let tbr = int_meta("ya:ovs:bitrate");
        let width = int_meta("og:video:width");
        let height = int_meta("og:video:height");
        let headers = json!({"Referer": url.as_str()});
        let ext = super::direct::determine_ext(&decoded)
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        let mut format_json =
            json!({"format_id": "", "url": decoded, "ext": ext, "http_headers": headers});
        for (key, value) in [("tbr", &tbr), ("width", &width), ("height", &height)] {
            if !value.is_null() {
                format_json[key] = value.clone();
            }
        }
        let mut format: crate::VideoFormat = serde_json::from_value(format_json)
            .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "url-only");
        crate::info_order::mark_int_rates(&mut format);
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let fields = vec![
            ("id", Value::from(video_id)),
            ("display_id", display_id.into()),
            ("url", decoded.into()),
            ("title", title.into()),
            ("description", optional(description)),
            ("thumbnail", optional(thumbnail)),
            ("creator", optional(meta("video:director"))),
            ("release_date", optional(release_date)),
            ("duration", int_meta("video:duration")),
            ("tbr", tbr),
            ("width", width),
            ("height", height),
            ("http_headers", headers),
        ];
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("untyped-dict;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(
            dict_info(url, (NAME, NAME), fields, vec![format]),
            &shape,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://popcorntimes.tv/de/m/A1XCFvz/haensel-und-gretel-opera-fantasy"
        ));
        assert!(!suitable("https://popcorntimes.tv/de/genre/x"));
        assert_eq!(rot13("Uryyb-123"), "Hello-123");
    }
}
