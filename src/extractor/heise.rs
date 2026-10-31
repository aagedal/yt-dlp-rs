//! Upstream `HeiseIE` (`yt_dlp/extractor/heise.py`): an article page's
//! YouTube embeds as a playlist of YouTube `url` results, titled by the
//! page. Kaltura players and the `videout` feed (its direct formats) are not
//! ported; those pages are unsupported errors.
use super::{Extractor, download_webpage, search_warning, youtube};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, json};
use std::sync::LazyLock;
use url::Url;

pub struct Heise;

const NAME: &str = "Heise";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?heise\.de/(?:[^/]+/)+[^/]+-(?P<id>[0-9]+)\.html",
    )
    .unwrap()
});
static DATA_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div[^>]+class="videoplayerjw"[^>]+data-title="([^"]+)""#).unwrap()
});
static HEADLINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<h1[^>]+\bclass=["']article_page_title[^>]+>(.+?)<"#).unwrap());
// `entry-id=(["'])(?P<id>(?:(?!\1).)+)\1` without the backreference.
static KALTURA_ENTRY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"entry-id=(?:"[^"]+"|'[^']+')"#).unwrap());
static CONTAINER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div class="videoplayerjw"[^>]+data-container="([0-9]+)""#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `url_result(url, 'Youtube')`, resolved when processed.
fn youtube_entry(target: &Url) -> Result<VideoInfo> {
    let target_text = target.as_str();
    let mut info = super::direct::build_at(target.clone(), target, String::new());
    info.formats.clear();
    info.direct = None;
    // `YoutubeIE.get_temp_id(url)`.
    info.id = youtube::temp_id(target_text).unwrap_or_default();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target_text,
            "force_videoid": null,
            "fields": {},
            "ie_key": "Youtube",
            "extractor": "youtube",
            "entry": {"ie_key": "Youtube", "_type": "url", "url": target_text},
        }),
    );
    Ok(info)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!(
        "Heise {what} (Kaltura or videout players) are not supported natively"
    ))
}

impl Extractor for Heise {
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
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;

        // `extract_title(default=None)`.
        let mut title = ["fulltitle", "title"]
            .iter()
            .find_map(|prop| crate::html::html_search_meta_clean(&webpage, prop));
        if title.as_deref().is_none_or(|title| title == "c't") {
            title = DATA_TITLE
                .captures(&webpage)
                .map(|found| found[1].to_owned())
                .or(title);
        }
        if title.is_none() {
            title = HEADLINE
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]));
        }
        // `_og_search_description(default=None) or _html_search_meta(...)`.
        if crate::html::og_search_property(&webpage, "description")
            .or_else(|| crate::html::html_search_meta_clean(&webpage, "description"))
            .is_none()
        {
            search_warning(NAME, "description");
        }

        if !super::kaltura::embed_urls(url, &webpage).is_empty() || KALTURA_ENTRY.is_match(&webpage)
        {
            return Err(unsupported("embeds"));
        }

        let yt_urls = youtube::embed_urls(url, &webpage);
        if !yt_urls.is_empty() {
            let entries = yt_urls
                .iter()
                .map(youtube_entry)
                .collect::<Result<Vec<_>>>()?;
            let mut extra = Map::new();
            extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                if title.is_some() {
                    "playlist-dict;id,title"
                } else {
                    "playlist-dict;id"
                }
                .into(),
            );
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id: video_id,
                title: title.unwrap_or_default(),
                webpage_url: url.clone(),
                extractor: NAME.into(),
                extractor_key: NAME.into(),
                nested: Vec::new(),
                pending: None,
                entries,
                extra,
            })));
        }

        // The `videout` feed path needs its container and sequence IDs.
        if webpage.contains("/videout/feed.json?") || CONTAINER.is_match(&webpage) {
            return Err(unsupported("videout feeds"));
        }
        Err(Error::Reported(format!(
            "[{NAME}] {video_id}: Unable to extract container ID; {}",
            crate::BUG_REPORTS_MESSAGE
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.heise.de/newsticker/meldung/Netflix-In-20-Jahren-vom-Videoverleih-zum-TV-Revolutionaer-3814130.html"
        ));
        assert!(suitable(
            "https://www.heise.de/ct/artikel/c-t-uplink-20-8-Staubsaugerroboter-Xiaomi-Vacuum-2-AR-Brille-Meta-2-und-Android-rooten-3959893.html"
        ));
        assert!(!suitable("https://www.heise.de/newsticker/"));
    }
}
