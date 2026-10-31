//! Upstream `TMZIE` (`yt_dlp/extractor/tmz.py`): a TMZ page's JSON-LD
//! `VideoObject`. A page whose JSON-LD has no media URL hands off to a
//! YouTube video (`cueVideoById`) or to a tweet embedded as a blockquote.
use super::{Extractor, dict_info, direct, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Tmz;

const NAME: &str = "TMZ";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| crate::extractor::url_regex(r"^https?://(?:www\.)?tmz\.com/.*").unwrap());
static CUE_VIDEO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.cueVideoById\(\s*(?:'(?P<sq>.*?)'|"(?P<dq>.*?)")"#).unwrap());
static TWEET_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+href=\s*(?:"(?P<dq>[^"]*)"|'(?P<sq>[^']*)')"#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Tmz {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_download_webpage(url, url)`: the URL is also the ID.
        let webpage = download_webpage(client, url, NAME, url.as_str())?;
        let mut jsonld = super::json_ld::search(&webpage);
        if jsonld.is_empty() {
            // `_search_json_ld(webpage, url)` is fatal.
            return Err(Error::Reported(format!(
                "[{NAME}] Unable to extract JSON-LD; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        if !jsonld.contains_key("url") {
            return hand_off(&webpage, url);
        }
        // `'width': int_or_none(e.get('width'))` (and height) in `_json_ld`.
        for key in ["width", "height"] {
            if let Some(value) = jsonld.get(key).cloned() {
                let number = match &value {
                    Value::String(text) => text.trim().parse::<i64>().ok().map(Value::from),
                    Value::Number(number) => number.as_i64().map(Value::from),
                    _ => None,
                };
                jsonld.insert(key.into(), number.unwrap_or(Value::Null));
            }
        }
        // `if id not in jsonld: jsonld['id'] = url` (the builtin `id`).
        jsonld.insert("id".into(), url.as_str().into());
        let fields: Vec<(&str, Value)> = jsonld
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        // The JSON-LD `url` is upstream's bare `url` result, a url-only info.
        let media = match jsonld.get("url") {
            Some(Value::String(media)) => Some(Url::parse(media)?),
            _ => None,
        };
        let info = match media {
            Some(media) => url_info(url, &media, fields),
            None => dict_info(url, (NAME, NAME), fields, Vec::new()),
        };
        Ok(Some(ExtractResult::Video(info)))
    }
}

/// Upstream's bare `url` result: the media URL is the one format, and the
/// dict's keys keep `fields`' order with no `formats` key.
fn url_info(page: &Url, media: &Url, fields: Vec<(&str, Value)>) -> VideoInfo {
    // `determine_ext` lowercases the extension.
    let ext = direct::determine_ext(media.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let keys: Vec<String> = fields.iter().map(|(key, _)| (*key).to_owned()).collect();
    let mut formats =
        direct::with_format(direct::build_at(media.clone(), media, ext.clone()), "").formats;
    // The dict's `width` and `height` size the format (`resolution`).
    let size = |key: &str| {
        fields
            .iter()
            .find(|(name, _)| *name == key)
            .and_then(|(_, value)| value.as_u64())
            .and_then(|number| u32::try_from(number).ok())
    };
    for format in &mut formats {
        format.ext = ext.clone();
        format.format_id = String::new();
        format.width = size("width");
        format.height = size("height");
    }
    let mut info = dict_info(page, (NAME, NAME), fields, formats);
    info.format_id = None;
    crate::info_order::mark_info(info, &format!("untyped-dict;{}", keys.join(",")))
}

/// The YouTube (`cueVideoById`) or tweet (`twitter-tweet`) hand-off.
fn hand_off(webpage: &str, url: &Url) -> Result<Option<ExtractResult>> {
    if let Some(found) = CUE_VIDEO.captures(webpage) {
        let target = found
            .name("sq")
            .or_else(|| found.name("dq"))
            .map_or("", |id| id.as_str())
            .to_owned();
        return Ok(Some(super::follow_url(url, target)));
    }
    // `get_element_by_attribute('class', 'twitter-tweet', webpage)`.
    let blockquote = crate::html_elements::elements_text_and_html_by_attribute(
        "class",
        "twitter-tweet",
        webpage,
    )
    .ok()
    .and_then(|elements| elements.into_iter().next())
    .map(|(text, _)| text);
    if let Some(blockquote) = blockquote {
        for found in TWEET_LINK.captures_iter(&blockquote) {
            let link = found
                .name("dq")
                .or_else(|| found.name("sq"))
                .map_or("", |link| link.as_str());
            if link.contains("/status/") {
                return Ok(Some(super::follow_url(url, link.to_owned())));
            }
        }
    }
    Err(Error::Reported(format!(
        "[{NAME}] No video found!; {}",
        crate::BUG_REPORTS_MESSAGE
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.tmz.com/videos/0-cegprt2p/"));
        assert!(suitable("https://www.tmz.com/2020/11/05/some-story/"));
        assert!(!suitable("https://example.com/tmz.com/"));
    }

    #[test]
    fn youtube_hand_off_takes_the_cued_id() {
        let found = CUE_VIDEO
            .captures(r#"player.cueVideoById( "Dddb6IGe-ws" )"#)
            .unwrap();
        assert_eq!(found.name("dq").unwrap().as_str(), "Dddb6IGe-ws");
    }
}
