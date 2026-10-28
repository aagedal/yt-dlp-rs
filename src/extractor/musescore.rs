//! Upstream `MuseScoreIE` (`yt_dlp/extractor/musescore.py`): a score page's
//! `og:url` score ID, the `api/jmuse` MP3 URL (with the ID's MD5
//! authorization token) and the page's OpenGraph and `musescore:` metadata.
use super::{Extractor, dict_format, dict_info, download_json_note_headers};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MuseScore;

const NAME: &str = "MuseScore";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?musescore\.com/(?:user/\d+|[^/]+)(?:/scores)?/(?P<id>[^#&?]+)")
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MuseScore {what} are not supported natively"))
}

/// `_generate_auth_token(video_id)`.
fn auth_token(video_id: &str) -> String {
    let mut digest = crate::md5::hex(format!("{video_id}mp30gs").as_bytes());
    digest.truncate(4);
    digest
}

/// A non-fatal search's value, or upstream's warning and `None`.
fn or_warn(value: Option<String>, name: &str) -> Value {
    match value {
        Some(value) => value.into(),
        None => {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
            Value::Null
        }
    }
}

impl Extractor for MuseScore {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        // `get_temp_id(url)` for errors raised before the score ID.
        let temp_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_download_webpage(url, None)`.
        crate::screen::screen(&format!("[{NAME}] Downloading webpage"));
        let webpage = super::download_webpage_quiet(client, url, NAME, &temp_id)?;
        // `_og_search_url(webpage)` is fatal.
        let page_url = crate::html::og_search_property(&webpage, "url").ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {temp_id}: Unable to extract OpenGraph url; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let page_url = if page_url.is_empty() {
            url.as_str().to_owned()
        } else {
            page_url
        };
        let video_id = VALID_URL
            .captures(&page_url)
            .map(|found| found["id"].to_owned())
            .ok_or_else(|| unsupported("og:url values that do not match score URLs"))?;
        let api = super::update_url_query(
            &Url::parse("https://musescore.com/api/jmuse")?,
            &[("id", &video_id), ("index", "0"), ("type", "mp3")],
        );
        let token = auth_token(&video_id);
        let data = download_json_note_headers(
            client,
            &api,
            NAME,
            &video_id,
            "Downloading JSON metadata",
            &[("authorization", &token)],
        )?;
        let key_error = |key: &str| {
            Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let info = data.get("info").ok_or_else(|| key_error("info"))?;
        let mp3_url = match info.get("url") {
            Some(Value::String(mp3_url)) => mp3_url.clone(),
            Some(_) => return Err(unsupported("non-string MP3 URLs")),
            None if info.is_object() => return Err(key_error("url")),
            None => return Err(unsupported("non-object jmuse info")),
        };
        let format = dict_format(
            vec![
                ("url", mp3_url.into()),
                ("ext", "mp3".into()),
                ("vcodec", "none".into()),
            ],
            NAME,
            &video_id,
        )?;
        let fields: Vec<(&str, Value)> = vec![
            ("id", video_id.into()),
            ("formats", Value::Null),
            (
                "title",
                or_warn(
                    crate::html::og_search_property(&webpage, "title"),
                    "OpenGraph title",
                ),
            ),
            (
                "description",
                or_warn(
                    crate::html::html_search_meta_clean(&webpage, "description"),
                    "description",
                ),
            ),
            (
                "thumbnail",
                or_warn(
                    crate::html::og_search_property(&webpage, "image"),
                    "thumbnail URL",
                ),
            ),
            (
                "uploader",
                or_warn(
                    crate::html::html_search_meta_clean(&webpage, "musescore:author"),
                    "uploader",
                ),
            ),
            (
                "creator",
                or_warn(
                    crate::html::html_search_meta_clean(&webpage, "musescore:composer"),
                    "composer",
                ),
            ),
        ];
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (NAME, NAME),
            fields,
            vec![format],
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://musescore.com/user/73797/scores/142975"));
        assert!(suitable("https://musescore.com/classicman/fur-elise"));
        assert!(suitable("https://musescore.com/minh_cuteee/scores/6555384"));
        assert!(!suitable("https://musescore.com/classicman"));
    }

    #[test]
    fn auth_tokens_follow_upstream() {
        // hashlib.md5(b'142975mp30gs').hexdigest()[:4]
        assert_eq!(auth_token("142975"), "096c");
        assert_eq!(auth_token("33816"), "5d02");
    }
}
