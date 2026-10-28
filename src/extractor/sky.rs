//! Upstream `SkySportsIE`, `SkyNewsStoryIE` and `SkySportsNewsIE`
//! (`yt_dlp/extractor/sky.py`): Sky Sports videos and Sky News/Sky Sports
//! articles, whose `sdc-article-video`/`sdc-site-video` elements become
//! `url_transparent` results of their Brightcove players (articles as
//! playlists).
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct SkySports;
pub struct SkyNews;
pub struct SkyNewsStory;
pub struct SkySportsNews;

const SPORTS_NAME: &str = "sky:sports";
const SPORTS_KEY: &str = "SkySports";
const NEWS_NAME: &str = "sky:news";
const NEWS_KEY: &str = "SkyNews";
const STORY_NAME: &str = "sky:news:story";
const STORY_KEY: &str = "SkyNewsStory";
const SPORTS_NEWS_NAME: &str = "sky:sports:news";
const SPORTS_NEWS_KEY: &str = "SkySportsNews";

static SPORTS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?skysports\.com/watch/video/([^/]+/)*(?P<id>[0-9]+)").unwrap()
});
static NEWS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://news\.sky\.com/video/[0-9a-z-]+-(?P<id>[0-9]+)").unwrap()
});
static STORY_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://news\.sky\.com/story/[0-9a-z-]+-(?P<id>[0-9]+)").unwrap()
});
static SPORTS_NEWS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?skysports\.com/([^/]+/)*news/\d+/(?P<id>\d+)").unwrap()
});
static SDC_EL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)(<div[^>]+data-(?:component-name|fn)="sdc-(?:articl|sit)e-video"[^>]*>)"#)
        .unwrap()
});

/// `SkySportsIE._match_id`.
pub fn sports_suitable(url: &str) -> bool {
    SPORTS_URL.is_match(url)
}

/// `SkyNewsIE._match_id`.
pub fn news_suitable(url: &str) -> bool {
    NEWS_URL.is_match(url)
}

/// `SkyNewsStoryIE._match_id`.
pub fn story_suitable(url: &str) -> bool {
    STORY_URL.is_match(url)
}

/// `SkySportsNewsIE._match_id`.
pub fn sports_news_suitable(url: &str) -> bool {
    SPORTS_NEWS_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `_process_video_element`: the Brightcove player URL and video ID. Other
/// providers leave `video_id` unbound upstream (an uncaught error).
fn video_element(element: &str, temp_id: &str) -> Result<(String, String)> {
    let sdc = crate::html_elements::extract_attributes(element);
    if sdc.get("data-provider").and_then(Value::as_str) != Some("brightcove") {
        return Err(Error::Reported(
            "cannot access local variable 'video_id' where it is not associated with a value"
                .into(),
        ));
    }
    let video_id = match sdc.get("data-video-id") {
        Some(Value::String(id)) => id.clone(),
        _ => {
            return Err(Error::Reported(format!(
                "{temp_id}: An extractor error has occurred. (caused by KeyError('data-video-id')); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
    };
    let text = |key: &str, default: &str| {
        sdc.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(default)
            .to_owned()
    };
    Ok((
        format!(
            "http://players.brightcove.net/{}/{}_default/index.html?videoId={video_id}",
            text("data-account-id", "6058004172001"),
            text("data-player-id", "RC9PQUaJ6")
        ),
        video_id,
    ))
}

/// `_og_search_title(webpage)` (non-fatal, warning without an ID).
fn og_title(name: &str, webpage: &str) -> Option<String> {
    let title = crate::html::og_search_property(webpage, "title");
    if title.is_none() {
        crate::screen::warning(&format!(
            "[{name}] unable to extract OpenGraph title; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    title
}

/// `SkyBaseIE._real_extract`: the page's first video element as a
/// `url_transparent` result with the page's title and description.
fn single(
    client: &Client,
    url: &Url,
    (name, regex): (&'static str, &Regex),
) -> Result<Option<ExtractResult>> {
    let Some(found) = regex.captures(url.as_str()) else {
        return Ok(None);
    };
    let video_id = found["id"].to_owned();
    start(name, url);
    let webpage = download_webpage(client, url, name, &video_id)?;
    let element = SDC_EL
        .captures(&webpage)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{name}] {video_id}: Unable to extract sdc element; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    let (target, _) = video_element(&element, &video_id)?;
    let title = og_title(name, &webpage);
    let description = crate::html::og_search_property(&webpage, "description");
    if description.is_none() {
        crate::screen::warning(&format!(
            "[{name}] unable to extract OpenGraph description; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    // `url_transparent` with the page's title and stripped description.
    let mut fields = Map::new();
    if let Some(title) = title {
        fields.insert("title".into(), title.into());
    }
    if let Some(description) = description {
        fields.insert("description".into(), description.trim().into());
    }
    let mut follow = super::direct::build_at(url.clone(), url, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow.extra.insert(
        crate::FOLLOW_TRANSPARENT.into(),
        json!({"url": target, "fields": Value::Object(fields)}),
    );
    Ok(Some(ExtractResult::Video(follow)))
}

impl Extractor for SkySports {
    fn key(&self) -> &'static str {
        SPORTS_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        single(client, url, (SPORTS_NAME, &SPORTS_URL))
    }
}

impl Extractor for SkyNews {
    fn key(&self) -> &'static str {
        NEWS_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        single(client, url, (NEWS_NAME, &NEWS_URL))
    }
}

/// `_process_video_element` as a playlist entry.
fn transparent_entry(target: &str, video_id: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = video_id.to_owned();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": "BrightcoveNew",
            "extractor": "brightcove:new",
            "entry": {"_type": "url_transparent", "id": video_id, "url": target, "ie_key": "BrightcoveNew"},
        }),
    );
    Ok(info)
}

/// `SkyNewsStoryIE`/`SkySportsNewsIE`: every video element as a playlist.
fn article(
    client: &Client,
    url: &Url,
    (name, key, regex): (&'static str, &'static str, &Regex),
) -> Result<Option<ExtractResult>> {
    let Some(found) = regex.captures(url.as_str()) else {
        return Ok(None);
    };
    let article_id = found["id"].to_owned();
    start(name, url);
    let webpage = download_webpage(client, url, name, &article_id)?;
    let mut entries = Vec::new();
    for element in SDC_EL.captures_iter(&webpage) {
        let (target, video_id) = video_element(&element[1], &article_id)?;
        entries.push(transparent_entry(&target, &video_id)?);
    }
    let title = og_title(name, &webpage).filter(|title| !title.is_empty());
    let description = ["og:description", "description"]
        .iter()
        .find_map(|meta| crate::html::html_search_meta(&webpage, meta));
    if description.is_none() {
        crate::screen::warning(&format!(
            "[{name}] unable to extract og:description; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    let mut keys = vec!["id"];
    if title.is_some() {
        keys.push("title");
    }
    let mut extra = Map::new();
    if let Some(description) = description {
        keys.push("description");
        extra.insert("description".into(), description.into());
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    Ok(Some(ExtractResult::Playlist(PlaylistInfo {
        id: article_id,
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })))
}

impl Extractor for SkyNewsStory {
    fn key(&self) -> &'static str {
        STORY_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        article(client, url, (STORY_NAME, STORY_KEY, &STORY_URL))
    }
}

impl Extractor for SkySportsNews {
    fn key(&self) -> &'static str {
        SPORTS_NEWS_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        article(
            client,
            url,
            (SPORTS_NEWS_NAME, SPORTS_NEWS_KEY, &SPORTS_NEWS_URL),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(sports_suitable(
            "https://www.skysports.com/watch/video/sports/f1/12160544/abu-dhabi-gp-the-notebook"
        ));
        assert!(news_suitable(
            "https://news.sky.com/video/russian-plane-inspected-after-deadly-fire-11712962"
        ));
        assert!(story_suitable(
            "https://news.sky.com/story/car-crash-captured-on-wigan-fans-live-stream-12345678"
        ));
        assert!(sports_news_suitable(
            "https://www.skysports.com/golf/news/12176/10871916/dustin-johnson"
        ));
        assert!(!sports_suitable("https://www.skysports.com/watch/video"));
    }
}
