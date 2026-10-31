//! Upstream `WebcameraplIE` (`yt_dlp/extractor/webcamerapl.py`): a camera
//! page's ROT13-encoded live HLS manifest and its heading.
use super::{Extractor, download_webpage, extract_m3u8_fatal_live};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Webcamerapl;

const NAME: &str = "Webcamerapl";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?P<id>[\w-]+)\.webcamera\.pl").unwrap()
});
static MANIFEST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-src\s*=\s*"(uggc[^"]+\.z3h8)""#).unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<h1\b[^>]*>([^>]+)</h1>").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `codecs.decode(text, 'rot-13')`.
fn rot13(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            c => c,
        })
        .collect()
}

impl Extractor for Webcamerapl {
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
        // `raise_no_formats(..., expected=True)`.
        let manifest = MANIFEST
            .captures(&webpage)
            .map(|found| rot13(&found[1]))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: No video/audio found at the provided url"
                ))
            })?;
        let manifest = Url::parse(&manifest)?;
        let mut info = extract_m3u8_fatal_live(client, &manifest, NAME, &id, None, true)?;
        super::hls::without_ext(&mut info);
        let title = TITLE
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract title; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(true));
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,formats,subtitles,is_live",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://warszawa-plac-zamkowy.webcamera.pl"));
        assert!(suitable("https://gdansk-stare-miasto.webcamera.pl/"));
        assert!(!suitable("https://webcamera.pl/"));
    }

    #[test]
    fn manifests_are_rot13_decoded() {
        assert_eq!(rot13("uggcf://n.o/p.z3h8"), "https://a.b/c.m3u8");
    }
}
