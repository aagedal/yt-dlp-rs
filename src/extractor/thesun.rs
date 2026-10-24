//! Upstream `TheSunIE` (`yt_dlp/extractor/thesun.py`): the article's
//! `data-video-id-pending` videos as a playlist of `url` results of their
//! accounts' Brightcove players, titled by the OpenGraph title.
use super::{Extractor, download_webpage};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TheSun;

const NAME: &str = "TheSun";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?the-?sun(\.co\.uk|\.com)/[^/]+/(?P<id>\d+)").unwrap()
});
static VIDEO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<video[^>]+data-video-id-pending=[^>]+>").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `url_result(url, 'BrightcoveNew', video_id)`, resolved when processed.
fn url_entry(target: &str, video_id: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), "BrightcoveNew".into());
    entry.insert("id".into(), video_id.into());
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

impl Extractor for TheSun {
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
        let article_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &article_id)?;
        let mut entries = Vec::new();
        for video in VIDEO.find_iter(&webpage) {
            let attrs = crate::html_elements::extract_attributes(video.as_str());
            let text = |key: &str| match attrs.get(key) {
                Some(Value::String(value)) => Some(value.clone()),
                _ => None,
            };
            // `attrs['data-video-id-pending']`: the regex guarantees the
            // attribute, though a valueless one has no text.
            let Some(video_id) = text("data-video-id-pending") else {
                return Err(crate::Error::UnsupportedHtml5(
                    "TheSun videos with a valueless data-video-id-pending".into(),
                ));
            };
            let account_id = text("data-account").unwrap_or_else(|| "5067014667001".into());
            entries.push(url_entry(
                &format!(
                    "http://players.brightcove.net/{account_id}/default_default/index.html?videoId={video_id}"
                ),
                &video_id,
            )?);
        }
        // `_og_search_title(webpage, fatal=False)`.
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            super::search_warning(NAME, "OpenGraph title");
        }
        let keys = if title.is_some() { "id,title" } else { "id" };
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: article_id,
            title: title.unwrap_or_default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.thesun.co.uk/tvandshowbiz/2261604/orlando-bloom-and-katy-perry-post-adorable-instagram-video-together-celebrating-thanksgiving-after-split-rumours/"
        ));
        assert!(suitable(
            "https://www.the-sun.com/entertainment/7611415/1000lb-sisters-fans-rip-amy-dangerous-health-decision/"
        ));
        assert!(!suitable("https://www.thesun.co.uk/tvandshowbiz/"));
    }
}
