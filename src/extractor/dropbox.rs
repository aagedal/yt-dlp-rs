//! Upstream `DropboxIE` (`yt_dlp/extractor/dropbox.py`): a shared file
//! page's base64 `registerStreamedPrefetch` parts, their transcode HLS
//! playlist and thumbnail, and the original file when anonymous downloads
//! are enabled. Password-protected files remain explicit errors.
use super::{Extractor, dict_format, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Dropbox;

const NAME: &str = "Dropbox";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?dropbox\.com/(?:(?:e/)?scl/f[io]|sh?)/(?P<id>\w+)").unwrap()
});
static PREFETCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"registerStreamedPrefetch\s*\(\s*"[\w/+=]+"\s*,\s*"([\w/+=]+)""#).unwrap()
});
static CONTENT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"content_id=([\w.+=/-]+)").unwrap());
static TRANSCODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n.?(https://[^\x03\x08\x12\n]+\.m3u8)").unwrap());
static THUMBNAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(https://www\.dropbox\.com/temp_thumb_from_token/[\w/?&=]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Dropbox {what} are not supported natively"))
}

/// `_yield_decoded_parts`: the prefetch payloads, last first, decoded as
/// UTF-8 with invalid bytes ignored.
fn decoded_parts(webpage: &str) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    for found in PREFETCH.captures_iter(webpage) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&found[1])
            .map_err(|_| unsupported("prefetch payloads that are not valid base64"))?;
        parts.push(
            String::from_utf8_lossy(&bytes)
                .chars()
                .filter(|c| *c != char::REPLACEMENT_CHARACTER)
                .collect(),
        );
    }
    parts.reverse();
    Ok(parts)
}

/// `os.path.splitext(urllib.parse.unquote(url_basename(url)))[0]`.
fn title(url: &Url) -> String {
    let basename = url
        .path()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let name = percent_encoding::percent_decode_str(basename)
        .decode_utf8_lossy()
        .into_owned();
    match name.rfind('.') {
        Some(index) if name[..index].chars().any(|c| c != '.') => name[..index].to_owned(),
        _ => name,
    }
}

impl Extractor for Dropbox {
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
        let title = title(url);
        let parts = decoded_parts(&webpage)?;
        if let Some(part) = parts.iter().find(|part| part.contains("/sm/password")) {
            if !CONTENT_ID.is_match(part) {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract content ID; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            if super::video_password().is_none() {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Password protected video, use --video-password <password>"
                )));
            }
            return Err(unsupported("password-protected files"));
        }
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let mut has_anonymous_download = false;
        let mut thumbnail = None;
        for part in &parts {
            if !has_anonymous_download {
                has_anonymous_download = part.contains("anonymous:\tanonymous");
            }
            let Some(found) = TRANSCODE.captures(part) else {
                continue;
            };
            let manifest = Url::parse(&found[1])?;
            let stream = extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, true)?;
            formats = stream.formats;
            subtitles = stream.subtitles;
            thumbnail = THUMBNAIL.captures(part).map(|found| found[1].to_owned());
            break;
        }
        if has_anonymous_download {
            formats.push(dict_format(
                vec![
                    (
                        "url",
                        super::update_url_query(url, &[("dl", "1")])
                            .to_string()
                            .into(),
                    ),
                    ("format_id", "original".into()),
                    ("format_note", "Original".into()),
                    ("quality", 1.into()),
                ],
                NAME,
                &video_id,
            )?);
        }
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.dropbox.com/s/nelirfsxnmcfbfh"));
        assert!(suitable(
            "https://www.dropbox.com/e/scl/fi/r2kd2skcy5ylbbta5y1pz/DJI_0003.MP4?dl=0&rlkey=wcdgqangn7t3lnmmv6li9mu9h"
        ));
        assert!(!suitable("https://www.dropbox.com/home"));
    }

    #[test]
    fn titles_follow_upstream_basenames() {
        let url = Url::parse(
            "https://www.dropbox.com/s/nelirfsxnmcfbfh/youtube-dl%20test%20video%20%27%C3%A4%22BaW_jenozKc.mp4?dl=0",
        )
        .unwrap();
        assert_eq!(title(&url), "youtube-dl test video '\u{e4}\"BaW_jenozKc");
    }
}
