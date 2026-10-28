//! Upstream `MangomoloVideoIE` and `MangomoloLiveIE`
//! (`yt_dlp/extractor/mangomolo.py`): Mangomolo players
//! (`player.mangomolo.com/v1/video|live`, the analytics embed path) from the
//! player page's `playlist.m3u8` as Wowza formats (SMIL skipped; HLS entries
//! of live streams use the `m3u8` protocol), with the hidden `userid` and
//! `duration`. Live channel IDs are base64; their iframes on generic pages
//! are `url_transparent` embeds with the ID and the page's host as uploader.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MangomoloVideo;
pub struct MangomoloLive;

pub(crate) const VIDEO_NAME: &str = "mangomolo:video";
pub(crate) const VIDEO_KEY: &str = "MangomoloVideo";
pub(crate) const LIVE_NAME: &str = "mangomolo:live";
pub(crate) const LIVE_KEY: &str = "MangomoloLive";

const BASE: &str = r"(?:https?:)?//(?:admin\.mangomolo\.com/analytics/index\.php/customers/embed/|player\.mangomolo\.com/v1/)";
const VIDEO_SLUG: &str = r"video\?.*?\bid=(?P<id>\d+)";
const LIVE_SLUG: &str = r"(?:live|index)\?.*?\bchannelid=(?P<id>(?:[A-Za-z0-9+/=]|%2B|%2F|%3D)+)";

static VIDEO_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^{BASE}{VIDEO_SLUG}")).unwrap());
static LIVE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^{BASE}{LIVE_SLUG}")).unwrap());
// `<iframe[^>]+src=(["\'])(?P<url>{_VALID_URL}.+?)\1`.
static VIDEO_EMBED: LazyLock<Regex> = LazyLock::new(|| embed_regex(VIDEO_SLUG));
static LIVE_EMBED: LazyLock<Regex> = LazyLock::new(|| embed_regex(LIVE_SLUG));
static VIDEO_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(VIDEO_SLUG).unwrap());
static LIVE_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(LIVE_SLUG).unwrap());
static FORMAT_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:file|src)\s*:\s*"(https?://[^"]+?/playlist\.m3u8)"#).unwrap()
});
static UPLOADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:https?://)?([^/]*)/.*").unwrap());

fn embed_regex(slug: &str) -> Regex {
    let quoted = |quote: &str, group: &str| {
        // The slug's `id` group is renamed: one name per alternative.
        let slug = slug.replace("(?P<id>", &format!("(?P<id_{group}>"));
        format!(r#"{quote}(?P<{group}>{BASE}{slug}[^\n]+?){quote}"#)
    };
    Regex::new(&format!(
        r#"<iframe[^>]+src=(?:{}|{})"#,
        quoted("\"", "dq"),
        quoted("'", "sq")
    ))
    .unwrap()
}

/// `MangomoloVideoIE._match_id`.
pub fn video_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

/// `MangomoloLiveIE._match_id`.
pub fn live_suitable(url: &str) -> bool {
    LIVE_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn video_temp_id(url: &str) -> Option<String> {
    VIDEO_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `get_temp_id`.
pub(crate) fn live_temp_id(url: &str) -> Option<String> {
    LIVE_URL.captures(url).map(|found| found["id"].to_owned())
}

fn embed_urls(regex: &Regex, url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in regex.captures_iter(webpage) {
        let Some(raw) = captures.name("dq").or_else(|| captures.name("sq")) else {
            continue;
        };
        let Ok(embed) = url.join(&crate::html::unescape_html(raw.as_str())) else {
            continue;
        };
        if !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

/// `MangomoloVideoIE._extract_embed_urls`.
pub(crate) fn video_embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    embed_urls(&VIDEO_EMBED, url, webpage)
}

/// `MangomoloLiveIE._extract_embed_urls`.
pub(crate) fn live_embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    embed_urls(&LIVE_EMBED, url, webpage)
}

/// `MangomoloBaseIE._extract_from_webpage`'s `url_transparent` fields: the
/// slug's `id` in the embed URL and the page URL's host as `uploader`.
fn embed_fields(slug: &Regex, page: &Url, embed: &Url) -> serde_json::Map<String, Value> {
    let mut fields = serde_json::Map::new();
    let id = slug
        .captures(embed.as_str())
        .map(|found| found["id"].to_owned());
    fields.insert("id".into(), id.into());
    let uploader = UPLOADER
        .captures(page.as_str())
        .map(|found| found[1].to_owned());
    fields.insert("uploader".into(), uploader.into());
    fields
}

/// [`embed_fields`] of `MangomoloVideoIE`.
pub(crate) fn video_embed_fields(page: &Url, embed: &Url) -> serde_json::Map<String, Value> {
    embed_fields(&VIDEO_ID, page, embed)
}

/// [`embed_fields`] of `MangomoloLiveIE`.
pub(crate) fn live_embed_fields(page: &Url, embed: &Url) -> serde_json::Map<String, Value> {
    embed_fields(&LIVE_ID, page, embed)
}

/// Python's non-strict `base64.b64decode` (`binascii.a2b_base64`):
/// non-alphabet characters are skipped and decoding stops at completing
/// padding.
fn b64decode(text: &str) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let (mut quad_pos, mut pads, mut left) = (0u8, 0u8, 0u8);
    for byte in text.bytes() {
        if byte == b'=' {
            if quad_pos >= 2 {
                pads += 1;
                if quad_pos + pads >= 4 {
                    return Ok(out);
                }
            }
            continue;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        pads = 0;
        match quad_pos {
            0 => {
                quad_pos = 1;
                left = value;
            }
            1 => {
                quad_pos = 2;
                out.push((left << 2) | (value >> 4));
                left = value & 0x0f;
            }
            2 => {
                quad_pos = 3;
                out.push((left << 4) | (value >> 2));
                left = value & 0x03;
            }
            _ => {
                quad_pos = 0;
                out.push((left << 6) | value);
            }
        }
    }
    match quad_pos {
        0 => Ok(out),
        1 => Err(format!(
            "Invalid base64-encoded string: number of data characters ({}) cannot be 1 more than a multiple of 4",
            out.len() / 3 * 4 + 1
        )),
        _ => Err("Incorrect padding".into()),
    }
}

/// Python's `bytes.decode()` (strict UTF-8) error text.
fn utf8_error(bytes: &[u8], error: std::str::Utf8Error) -> String {
    let position = error.valid_up_to();
    let byte = bytes[position];
    match error.error_len() {
        None if bytes.len() - position > 1 => format!(
            "'utf-8' codec can't decode bytes in position {position}-{}: unexpected end of data",
            bytes.len() - 1
        ),
        None => format!(
            "'utf-8' codec can't decode byte 0x{byte:02x} in position {position}: unexpected end of data"
        ),
        Some(_) => {
            let reason = if matches!(byte, 0x80..=0xc1 | 0xf5..=0xff) {
                "invalid start byte"
            } else {
                "invalid continuation byte"
            };
            format!("'utf-8' codec can't decode byte 0x{byte:02x} in position {position}: {reason}")
        }
    }
}

/// `MangomoloLiveIE._get_real_id`:
/// `base64.b64decode(urllib.parse.unquote(page_id)).decode()`.
fn live_real_id(page_id: &str) -> Result<String> {
    let unquoted = percent_encoding::percent_decode_str(page_id).decode_utf8_lossy();
    let bytes = b64decode(&unquoted).map_err(Error::Reported)?;
    String::from_utf8(bytes.clone())
        .map_err(|error| Error::Reported(utf8_error(&bytes, error.utf8_error())))
}

/// Python's `int(text)` for `int_or_none`: surrounding whitespace allowed.
fn int_or_none(text: Option<&str>) -> Value {
    text.map(|text| text.trim_matches(crate::batch::is_py_space))
        .and_then(|text| text.parse::<i64>().ok())
        .map_or(Value::Null, Value::from)
}

/// `MangomoloBaseIE._real_extract`.
fn extract(
    client: &Client,
    url: &Url,
    (name, key, kind): (&'static str, &'static str, &str),
    page_id: String,
) -> Result<VideoInfo> {
    let live = kind == "live";
    // `url.split('?')[1]`.
    let query = url.as_str().split('?').nth(1).unwrap_or_default();
    let player = Url::parse(&format!("https://player.mangomolo.com/v1/{kind}?{query}"))?;
    let webpage = download_webpage(client, &player, name, &page_id)?;
    let hidden = super::login::hidden_inputs(&webpage);
    let input = |name: &str| {
        hidden
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let Some(found) = FORMAT_URL.captures(&webpage) else {
        return Err(Error::Reported(format!(
            "[{name}] {page_id}: Unable to extract format url; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let format_url = crate::html::clean_html(&found[1]);
    let mut formats = super::extract_wowza_formats(client, &format_url, name, &page_id, &["smil"])?;
    if live {
        // `m3u8_entry_protocol='m3u8'`.
        for format in &mut formats {
            if format.protocol.as_deref() == Some("m3u8_native") {
                format.protocol = Some("m3u8".into());
            }
        }
    }
    Ok(dict_info(
        url,
        (name, key),
        vec![
            ("id", page_id.clone().into()),
            ("title", page_id.into()),
            (
                "uploader_id",
                input("userid").map_or(Value::Null, Value::from),
            ),
            ("duration", int_or_none(input("duration"))),
            ("is_live", live.into()),
            ("formats", Value::Null),
        ],
        formats,
    ))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for MangomoloVideo {
    fn key(&self) -> &'static str {
        VIDEO_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(page_id) = video_temp_id(url.as_str()) else {
            return Ok(None);
        };
        announce(VIDEO_NAME, url);
        extract(client, url, (VIDEO_NAME, VIDEO_KEY, "video"), page_id).map(Some)
    }
}

impl Extractor for MangomoloLive {
    fn key(&self) -> &'static str {
        LIVE_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(temp_id) = live_temp_id(url.as_str()) else {
            return Ok(None);
        };
        announce(LIVE_NAME, url);
        let page_id = live_real_id(&temp_id)?;
        extract(client, url, (LIVE_NAME, LIVE_KEY, "live"), page_id).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert_eq!(
            video_temp_id("https://player.mangomolo.com/v1/video?id=29431242&user_id=168")
                .as_deref(),
            Some("29431242")
        );
        assert_eq!(
            live_temp_id("https://player.mangomolo.com/v1/live?channelid=MjM%3D&x=1").as_deref(),
            Some("MjM%3D")
        );
        assert!(live_suitable(
            "https://admin.mangomolo.com/analytics/index.php/customers/embed/index?channelid=MjM="
        ));
        assert!(!video_suitable("https://player.mangomolo.com/v1/live?id=1"));
    }

    #[test]
    fn base64_follows_python() {
        assert_eq!(b64decode("MjM=").unwrap(), b"23");
        assert_eq!(b64decode("MjM").unwrap_err(), "Incorrect padding");
        assert_eq!(
            b64decode("MjMxN").unwrap_err(),
            "Invalid base64-encoded string: number of data characters (5) cannot be 1 more than a multiple of 4"
        );
        assert_eq!(b64decode("Mj=M=").unwrap(), b"23");
        assert_eq!(live_real_id("MjM%3D").unwrap(), "23");
    }
}
