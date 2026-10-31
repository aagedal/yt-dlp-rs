//! Upstream `OneNewsNZIE` (`yt_dlp/extractor/onenewsnz.py`): an article's
//! `Fusion.globalContent` video (Brightcove) and YouTube elements as a
//! playlist of `url` results.
use super::{Extractor, download_webpage, search_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct OneNewsNz;

const NAME: &str = "1News";
const KEY: &str = "OneNewsNZ";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?(?:1|one)news\.co\.nz/\d+/\d+/\d+/(?P<id>[^/?#&]+)",
    )
    .unwrap()
});
static FUSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Fusion\.globalContent\s*=\s*").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `url_result(target, ie)`: an untitled, ID-less `url` entry.
fn url_entry(target: &str, ie_key: &str, extractor: &str) -> Result<VideoInfo> {
    // A bare YouTube ID resolves as its watch page.
    let url = Url::parse(target)
        .or_else(|_| Url::parse(&format!("https://www.youtube.com/watch?v={target}")))?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ie_key.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": url.as_str(),
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": extractor,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// A string-or-number traversal result as Python's `%s` formats it.
fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// Python truthiness of a string traversal result.
fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(text) => !text.is_empty(),
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    })
}

impl Extractor for OneNewsNz {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let fusion = match search_json(&FUSION, &webpage, false) {
            Some(Some(fusion)) => fusion,
            Some(None) => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract fusion metadata - Failed to parse JSON"
                )));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract fusion metadata; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let mut entries = Vec::new();
        let items = match fusion.get("content_elements") {
            Some(Value::Array(items)) => items.as_slice(),
            _ => &[],
        };
        for item in items {
            match item.get("subtype").and_then(Value::as_str) {
                Some("video") => {
                    let config = item.pointer("/embed/config");
                    let field = |key: &str| config.and_then(|config| config.get(key));
                    let account = truthy(field("brightcoveAccount"))
                        .map_or(Some("963482464001".to_owned()), |value| text(Some(value)));
                    let (Some(account), video_id) = (account, text(field("brightcoveVideoId")))
                    else {
                        return Err(Error::UnsupportedHtml5(
                            "1News Brightcove accounts that are not strings".into(),
                        ));
                    };
                    let target = format!(
                        "http://players.brightcove.net/{account}/0xpHIR6IB_default/index.html?videoId={}",
                        video_id.as_deref().unwrap_or("None")
                    );
                    entries.push(url_entry(&target, "BrightcoveNew", "brightcove:new")?);
                }
                Some("youtube") => {
                    let id = truthy(item.pointer("/referent/id"))
                        .or_else(|| truthy(item.pointer("/raw_oembed/_id")));
                    if let Some(id) = id {
                        let Some(id) = id.as_str() else {
                            return Err(Error::UnsupportedHtml5(
                                "1News YouTube IDs that are not strings".into(),
                            ));
                        };
                        entries.push(url_entry(id, "Youtube", "youtube")?);
                    }
                }
                _ => {}
            }
        }
        if entries.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: This article does not have a video."
            )));
        }
        let title = match truthy(fusion.pointer("/headlines/basic")) {
            Some(Value::String(title)) => Some(title.clone()),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "1News headlines that are not strings".into(),
                ));
            }
            None => super::html5::page_title(&webpage),
        };
        let keys = if title.is_some() { "id,title" } else { "id" };
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.1news.co.nz/2022/09/29/cows-painted-green-on-parliament-lawn-in-climate-protest/"
        ));
        assert!(suitable(
            "https://www.onenews.co.nz/2022/09/30/now-is-the-time-to-get-moving/"
        ));
        assert!(!suitable("https://www.1news.co.nz/news/"));
    }
}
