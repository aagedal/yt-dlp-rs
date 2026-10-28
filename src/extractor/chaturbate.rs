//! Upstream `ChaturbateIE` (`yt_dlp/extractor/chaturbate.py`): Chaturbate
//! rooms from the `get_edge_hls_url_ajax` API (room status errors, geo
//! restriction), else the room page's `initialRoomDossier` or embedded
//! `.m3u8` URLs (fast and regular live HLS) and its error text.
use super::{Extractor, dict_info, download_webpage_note_headers};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Chaturbate;

const NAME: &str = "Chaturbate";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:[^/]+\.)?chaturbate\.(?P<tld>com|eu|global)/(?:fullvideo/?\?.*?\bb=)?(?P<id>[^/?&#]+)",
    )
    .unwrap()
});
static DOSSIER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"initialRoomDossier\s*=\s*(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')"#).unwrap()
});
static ESCAPED_M3U8: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\u0022(?P<dq>http.+?\.m3u8.*?)\\u0022|\\u0027(?P<sq>http.+?\.m3u8.*?)\\u0027")
        .unwrap()
});
static QUOTED_M3U8: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""(?P<dq>http.+?\.m3u8.*?)"|'(?P<sq>http.+?\.m3u8.*?)'"#).unwrap()
});
static ERRORS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<span[^>]+class=(?:"desc_span"|'desc_span')[^>]*>(?P<error>[^<]+)</span>"#)
            .unwrap(),
        Regex::new(r#"<div[^>]+id=(?:"defchat"|'defchat')[^>]*>\s*<p><strong>(?P<error>[^<]+)<"#)
            .unwrap(),
    ]
});
static UNICODE_ESCAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\u[0-9a-fA-F]{4}").unwrap());

const OFFLINE: &str = "Room is currently offline";

/// `_ERROR_MAP`.
fn error_message(status: &str) -> Option<&'static str> {
    Some(match status {
        "offline" => OFFLINE,
        "private" => "Room is currently in a private show",
        "away" => "Performer is currently away",
        "password protected" => "Room is password protected",
        "hidden" => "Hidden session in progress",
        _ => return None,
    })
}

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `lowercase_escape`: `\uXXXX` escapes decoded.
fn lowercase_escape(text: &str) -> String {
    UNICODE_ESCAPE
        .replace_all(text, |found: &regex::Captures<'_>| {
            u32::from_str_radix(&found[0][2..], 16)
                .ok()
                .and_then(char::from_u32)
                .map_or_else(|| found[0].to_owned(), String::from)
        })
        .into_owned()
}

fn quoted<'a>(found: &regex::Captures<'a>) -> Option<&'a str> {
    found
        .name("dq")
        .or_else(|| found.name("sq"))
        .map(|value| value.as_str())
}

/// `with impersonation`.
fn impersonated<T>(request: impl FnOnce() -> Result<T>) -> Result<T> {
    let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
        crate::screen::warning_once(&format!("[{NAME}] {message}"));
    });
    request()
}

fn live_info(
    url: &Url,
    video_id: &str,
    age_limit: Value,
    formats: Vec<crate::VideoFormat>,
    api: bool,
) -> VideoInfo {
    let thumbnail = format!("https://roomimg.stream.highwebmedia.com/ri/{video_id}.jpg");
    let mut fields = vec![
        ("id", Value::from(video_id)),
        ("title", Value::from(video_id)),
        ("thumbnail", thumbnail.into()),
    ];
    if api {
        fields.push(("is_live", true.into()));
        fields.push(("age_limit", age_limit));
    } else {
        fields.push(("age_limit", age_limit));
        fields.push(("is_live", true.into()));
    }
    fields.push(("formats", Value::Null));
    dict_info(url, (NAME, NAME), fields, formats)
}

/// `_extract_from_api`: `None` to fall back to the page.
fn from_api(client: &Client, url: &Url, video_id: &str, tld: &str) -> Result<Option<VideoInfo>> {
    use std::io::Read;
    let api = Url::parse(&format!("https://chaturbate.{tld}/get_edge_hls_url_ajax/"))?;
    crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading JSON metadata"));
    let body: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("room_slug", video_id)
        .finish();
    // `_download_json(..., fatal=False, impersonate=True) or {}`.
    let response = impersonated(|| {
        super::direct::post(
            client,
            &api,
            &[
                ("X-Requested-With", "XMLHttpRequest"),
                ("Accept", "application/json"),
                ("Content-Type", "application/x-www-form-urlencoded"),
            ],
            body,
        )
    });
    let response = match response {
        Ok(mut response) => {
            let mut bytes = Vec::new();
            response
                .read_to_end(&mut bytes)
                .map_err(Error::ExtractionRead)?;
            let bytes =
                super::decode_content(response.headers(), bytes).map_err(Error::ExtractionRead)?;
            let text = super::decode_webpage(&bytes, response.headers());
            match crate::json::from_str::<Value>(&text) {
                Ok(value) => value,
                Err(_) => {
                    let detail = crate::json::python_decode_error_text(&text)
                        .unwrap_or_else(|| "Expecting value".into());
                    crate::screen::warning(&format!(
                        "[{NAME}] {video_id}: Failed to parse JSON: {detail}"
                    ));
                    Value::Null
                }
            }
        }
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&error)
            ));
            Value::Null
        }
        Err(error) => return Err(error),
    };
    let m3u8_url = response
        .get("url")
        .filter(|value| super::nuxt::truthy(value))
        .and_then(Value::as_str);
    let Some(m3u8_url) = m3u8_url else {
        let status = response.get("room_status");
        if let Some(message) = status.and_then(Value::as_str).and_then(error_message) {
            return Err(Error::Reported(format!("[{NAME}] {video_id}: {message}")));
        }
        if status.and_then(Value::as_str) == Some("public") {
            return Err(crate::geo::restricted(
                &format!(
                    "[{NAME}] {video_id}: This video is not available from your location due to geo restriction"
                ),
                &[],
            ));
        }
        let shown = match status {
            None | Some(Value::Null) => "None".to_owned(),
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
        };
        crate::screen::warning(&format!(
            "[{NAME}] Got status \"{shown}\" from API; falling back to webpage extraction"
        ));
        return Ok(None);
    };
    let stream = super::extract_m3u8_fatal_live(
        client,
        &Url::parse(m3u8_url).map_err(|_| unsupported("stream URLs that are not URLs"))?,
        NAME,
        video_id,
        None,
        false,
    )?;
    Ok(Some(live_info(
        url,
        video_id,
        18.into(),
        stream.formats,
        true,
    )))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Chaturbate {what} are not supported natively"))
}

/// `_extract_from_html`.
fn from_html(client: &Client, url: &Url, video_id: &str, tld: &str) -> Result<VideoInfo> {
    let page = Url::parse(&format!("https://chaturbate.{tld}/{video_id}/"))?;
    let webpage = impersonated(|| {
        download_webpage_note_headers(client, &page, NAME, video_id, "Downloading webpage", &[])
    })?;
    let mut found_urls: Vec<String> = Vec::new();
    if let Some(raw) = DOSSIER
        .captures(&webpage)
        .and_then(|found| quoted(&found).map(str::to_owned))
    {
        match crate::json::from_str::<Value>(&lowercase_escape(&raw)) {
            Ok(data) => {
                if let Some(m3u8_url) = data
                    .get("hls_source")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                {
                    found_urls.push(m3u8_url);
                }
            }
            Err(_) => {
                return Err(unsupported("room dossiers that do not parse"));
            }
        }
    }
    if found_urls.is_empty() {
        for found in ESCAPED_M3U8.captures_iter(&webpage) {
            if let Some(raw) = quoted(&found) {
                found_urls.push(lowercase_escape(raw));
            }
        }
    }
    if found_urls.is_empty() {
        for found in QUOTED_M3U8.captures_iter(&webpage) {
            if let Some(raw) = quoted(&found) {
                found_urls.push(raw.to_owned());
            }
        }
    }
    let mut m3u8_urls: Vec<String> = Vec::new();
    for found in &found_urls {
        for candidate in [found.clone(), found.replace("_fast", "")] {
            if !m3u8_urls.contains(&candidate) {
                m3u8_urls.push(candidate);
            }
        }
    }
    if m3u8_urls.is_empty() {
        let mut error = ERRORS
            .iter()
            .find_map(|pattern| {
                pattern
                    .captures(&webpage)
                    .map(|found| found["error"].to_owned())
            })
            .filter(|error| !error.is_empty());
        if error.is_none()
            && [OFFLINE, "offline_tipping", "tip_offline"]
                .iter()
                .any(|needle| webpage.contains(needle))
        {
            error = Some(OFFLINE.to_owned());
        }
        return Err(Error::Reported(match error {
            Some(error) => format!("[{NAME}] {video_id}: {error}"),
            None => format!(
                "[{NAME}] {video_id}: Unable to find stream URL; {}",
                crate::BUG_REPORTS_MESSAGE
            ),
        }));
    }
    let mut formats = Vec::new();
    for m3u8_url in &m3u8_urls {
        let m3u8_id = ["fast", "slow"]
            .into_iter()
            .find(|known| m3u8_url.contains(&format!("_{known}")));
        let Ok(manifest) = Url::parse(m3u8_url) else {
            return Err(unsupported("stream URLs that are not URLs"));
        };
        if let Some(stream) = super::extract_m3u8_optional_note(
            client,
            &manifest,
            NAME,
            video_id,
            m3u8_id,
            false,
            true,
            "Downloading m3u8 information",
        ) {
            for mut format in stream.formats {
                // ffmpeg skips segments for fast m3u8.
                if m3u8_id == Some("fast") {
                    format.extra.insert("preference".into(), Value::from(-10));
                }
                formats.push(format);
            }
        }
    }
    let age_limit = crate::html::rta_search(&webpage).map_or(Value::Null, Value::from);
    Ok(live_info(url, video_id, age_limit, formats, false))
}

impl Extractor for Chaturbate {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let tld = found["tld"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        if let Some(info) = from_api(client, url, &video_id, &tld)? {
            return Ok(Some(info));
        }
        from_html(client, url, &video_id, &tld).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.chaturbate.com/siswet19/"));
        assert!(suitable("https://chaturbate.eu/siswet19/"));
        assert!(suitable("https://chaturbate.com/fullvideo/?b=caylin"));
        assert!(!suitable("https://chaturbate.net/x/"));
    }

    #[test]
    fn escapes_decode_like_lowercase_escape() {
        assert_eq!(lowercase_escape(r"a\u0022b\u002Fc"), "a\"b/c");
    }
}
