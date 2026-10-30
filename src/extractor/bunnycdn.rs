//! Upstream `BunnyCdnIE` (`yt_dlp/extractor/bunnycdn.py`): a
//! `mediadelivery.net` embed page's HTML5 sources (HLS masters), its
//! `originalUrl` source when it answers, and the `#main-video` title and
//! poster merged with the page's JSON-LD. MediaCage activation and ping
//! streams are not supported natively.
use super::{Extractor, dict_format, dict_info, download_webpage_note_headers, html5};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BunnyCdn;

const NAME: &str = "BunnyCdn";
const DEFAULT_REFERER: &str = "https://iframe.mediadelivery.net/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:iframe|player)\.mediadelivery\.net|video\.bunnycdn\.com)/(?:embed|play)/(?P<library_id>\d+)/(?P<id>[\da-f-]+)",
    )
    .unwrap()
});
static ORIGINAL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:var|const|let)\s+originalUrl\s*=\s*["']([^"']+)["']"#).unwrap()
});
static SRC_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.setAttribute\(['"]src['"],\s*['"]([^'"]+)['"]\)"#).unwrap());
static ACTIVATE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"loadUrl\(['"]([^'"]+/activate)['"]"#).unwrap());
static PING_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"loadUrl\(['"]([^'"]+/ping)['"]"#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `parse_qs(text).get(key)[0]` for a non-blank value.
fn query_value(text: &str, key: &str) -> Option<String> {
    let (_, query) = text.split_once('?')?;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(name, value)| name == key && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// `{'key': value}` merged the way Python's `{**a, **b}` keeps the first
/// position of a repeated key.
fn merge(fields: &mut Vec<(String, Value)>, key: &str, value: Value) {
    match fields.iter_mut().find(|(name, _)| name == key) {
        Some((_, slot)) => *slot = value,
        None => fields.push((key.to_owned(), value)),
    }
}

impl Extractor for BunnyCdn {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let library_id = found["library_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let mut embed = Url::parse(&format!(
            "https://iframe.mediadelivery.net/embed/{library_id}/{video_id}"
        ))?;
        // `query=traverse_obj(parse_qs(url), {'token', 'expires'})`.
        let pairs: Vec<(&str, String)> = ["token", "expires"]
            .into_iter()
            .filter_map(|key| query_value(url.as_str(), key).map(|value| (key, value)))
            .collect();
        if !pairs.is_empty() {
            let updates: Vec<(&str, &str)> = pairs
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect();
            embed = super::update_url_query(&embed, &updates);
        }
        // `headers={'Referer': smuggled Referer or the iframe's own host}`.
        let webpage = download_webpage_note_headers(
            client,
            &embed,
            NAME,
            &video_id,
            "Downloading webpage",
            &[("Referer", DEFAULT_REFERER)],
        )?;
        let referer = url.as_str();

        // `_html_extract_title(webpage) == '403'` is what the upstream
        // condition tests, so only a 403 title raises.
        if crate::html::html_extract_title(&webpage).as_deref() == Some("403") {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: This video is inaccessible. Setting a Referer header might be required to access the video"
            )));
        }

        let sources =
            html5::parse_html5_media_entries(client, url, &webpage, &video_id, NAME, None, None)?;
        let (mut formats, subtitles) = sources
            .into_iter()
            .next()
            .map_or_else(Default::default, |entry| (entry.formats, entry.subtitles));

        let headers = [("Referer", referer)];
        if let Some(original) = ORIGINAL_URL
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .and_then(|text| super::json_ld::url_or_none(&text))
        {
            // `HEADRequest(original_url)`, `fatal=False`, `expected_status=(403, 404)`.
            let original = Url::parse(&original)?;
            if let Ok(response) =
                super::direct::head_expecting(client, &original, &headers, &[403, 404])
                && response.status().as_u16() == 200
            {
                let content_length = response
                    .headers()
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.trim().parse::<i64>().ok());
                let ext = super::direct::urlhandle_detect_ext(&response, Some("mp4"));
                formats.push(dict_format(
                    vec![
                        ("url", original.as_str().into()),
                        ("format_id", "source".into()),
                        ("quality", 1.into()),
                        ("http_headers", json!({"Referer": referer})),
                        ("ext", ext.into()),
                        ("filesize", content_length.map_or(Value::Null, Value::from)),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }

        // MediaCage streams need an activation request and pings during
        // download, which this port does not perform.
        let src_url = SRC_URL.captures(&webpage).map(|found| found[1].to_owned());
        if let Some(src) = &src_url
            && ACTIVATE_URL.is_match(&webpage)
            && PING_URL.is_match(&webpage)
            && query_value(src, "secret").is_some()
            && query_value(src, "contextId").is_some()
        {
            return Err(Error::UnsupportedHtml5(
                "BunnyCdn MediaCage activation and ping streams are not supported natively".into(),
            ));
        }

        // `{'id', 'formats', 'subtitles', **main-video fields, **json_ld}`.
        let mut fields: Vec<(String, Value)> = vec![
            ("id".into(), video_id.clone().into()),
            ("formats".into(), Value::Null),
            ("subtitles".into(), Value::Null),
        ];
        let main_video = crate::html_elements::element_html_by_id("main-video", &webpage)
            .ok()
            .flatten();
        if let Some(element) = main_video {
            let attributes = crate::html_elements::extract_attributes(&element);
            if let Some(Value::String(config)) = attributes.get("data-plyr-config") {
                let config: Value = serde_json::from_str(config).map_err(|_| {
                    Error::UnsupportedHtml5(
                        "BunnyCdn player configs that are not JSON are not supported natively"
                            .into(),
                    )
                })?;
                if let Some(Value::String(title)) = config.get("title") {
                    merge(&mut fields, "title", title.clone().into());
                }
            }
            if let Some(Value::String(poster)) = attributes.get("data-poster")
                && let Some(poster) = super::json_ld::url_or_none(poster)
            {
                merge(&mut fields, "thumbnail", poster.into());
            }
        }
        let json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract JSON-LD ; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        for (key, value) in json_ld {
            merge(&mut fields, &key, value);
        }

        let fields: Vec<(&str, Value)> = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://iframe.mediadelivery.net/embed/113933/e73edec1-e381-4c8b-ae73-717a140e0924"
        ));
        assert!(suitable(
            "https://iframe.mediadelivery.net/play/136145/32e34c4b-0d72-437c-9abb-05e67657da34"
        ));
        assert!(suitable(
            "https://player.mediadelivery.net/embed/519128/875880a9-bcc2-4038-9e05-e5024bba9b70"
        ));
        assert!(!suitable("https://iframe.mediadelivery.net/"));
    }

    #[test]
    fn query_values_skip_blank_parameters() {
        assert_eq!(
            query_value("https://x/embed/1/2?token=abc&expires=", "token").as_deref(),
            Some("abc")
        );
        assert_eq!(query_value("https://x/embed/1/2?expires=", "expires"), None);
    }
}
