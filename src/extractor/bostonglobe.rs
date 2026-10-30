//! Upstream `BostonGlobeIE` (`yt_dlp/extractor/bostonglobe.py`): an
//! article's `<video data-brightcove-video-id ...>` players as Brightcove
//! `url` results (one directly, several as a playlist titled by the
//! OpenGraph title).
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BostonGlobe;

const NAME: &str = "BostonGlobe";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"(?i)^https?://(?:www\.)?bostonglobe\.com/.*/(?P<id>[^/]+)/\w+(?:\.html)?",
    )
    .unwrap()
});
static VIDEO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(<video[^>]+>)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `url_result(url, 'BrightcoveNew')`, resolved when processed.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), "BrightcoveNew".into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": "BrightcoveNew",
            "extractor": "brightcove:new",
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for BostonGlobe {
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
        let page_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &page_id)?;
        // `_og_search_title(webpage, default=None)`.
        let page_title = crate::html::og_search_property(&webpage, "title");
        let mut targets = Vec::new();
        for video in VIDEO.find_iter(&webpage) {
            let attrs = crate::html_elements::extract_attributes(video.as_str());
            // A valueless attribute is `None`; an empty one is falsy.
            let text = |key: &str| match attrs.get(key) {
                Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
                _ => None,
            };
            if let (Some(video_id), Some(account_id), Some(player_id), Some(embed)) = (
                text("data-brightcove-video-id"),
                text("data-account"),
                text("data-player"),
                text("data-embed"),
            ) {
                targets.push(format!(
                    "http://players.brightcove.net/{account_id}/{player_id}_{embed}/index.html?videoId={video_id}"
                ));
            }
        }
        match targets.len() {
            // Upstream hands the page to the generic extractor.
            0 => Err(Error::UnsupportedHtml5(
                "BostonGlobe pages without Brightcove videos (upstream's generic fallback) are not supported natively"
                    .into(),
            )),
            1 => {
                // `url_result(entries[0], 'BrightcoveNew')`, followed.
                let mut follow = super::direct::build_at(url.clone(), url, String::new());
                follow.formats.clear();
                follow.direct = None;
                follow
                    .extra
                    .insert(crate::FOLLOW_URL.into(), targets[0].clone().into());
                Ok(Some(ExtractResult::Video(follow)))
            }
            _ => {
                let entries = targets
                    .iter()
                    .map(|target| url_entry(target))
                    .collect::<Result<Vec<_>>>()?;
                let keys = if page_title.is_some() {
                    "id,title"
                } else {
                    "id"
                };
                let mut extra = Map::new();
                extra.insert(
                    crate::info_order::INFO_SHAPE.into(),
                    format!("playlist-dict;{keys}").into(),
                );
                Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                    id: page_id,
                    title: page_title.unwrap_or_default(),
                    webpage_url: url.clone(),
                    extractor: NAME.into(),
                    extractor_key: NAME.into(),
                    nested: Vec::new(),
                    pending: None,
                    entries,
                    extra,
                })))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.bostonglobe.com/metro/2017/02/11/tree-finally-succumbs-disease-leaving-hole-neighborhood/h1b4lviqzMTIn9sVy8F3gP/story.html"
        ));
        assert!(!suitable("https://www.bostonglobe.com/"));
    }
}
