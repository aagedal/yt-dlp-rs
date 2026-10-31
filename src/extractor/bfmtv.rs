//! Upstream `BFMTVIE`, `BFMTVLiveIE` and `BFMTVArticleIE`
//! (`yt_dlp/extractor/bfmtv.py`): BFM TV and RMC videos, live pages and
//! articles, whose `data-video-id` blocks become Brightcove player `url`
//! results (an article's as a playlist).
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Bfmtv;
pub struct BfmtvLive;
pub struct BfmtvArticle;

const NAME: &str = "bfmtv";
const KEY: &str = "BFMTV";
const LIVE_NAME: &str = "bfmtv:live";
const LIVE_KEY: &str = "BFMTVLive";
const ARTICLE_NAME: &str = "bfmtv:article";
const ARTICLE_KEY: &str = "BFMTVArticle";

const BASE: &str = r"^https?://(?:www\.|rmc\.)?bfmtv\.com/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"{BASE}(?:[^/]+/)*[^/?&#]+_V[A-Z]-(?P<id>\d{{12}})\.html"
    ))
    .unwrap()
});
static LIVE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{BASE}(?P<id>(?:[^/]+/)?en-direct)")).unwrap());
static ARTICLE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{BASE}(?:[^/]+/)*[^/?&#]+_A[A-Z]-(?P<id>\d{{12}})\.html"
    ))
    .unwrap()
});
static VIDEO_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(<div[^>]+\bdata-video-id=[^>]+>.*?</div>)").unwrap());

/// `BFMTVIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `BFMTVLiveIE._match_id`.
pub fn live_suitable(url: &str) -> bool {
    LIVE_URL.is_match(url)
}

/// `BFMTVArticleIE._match_id`.
pub fn article_suitable(url: &str) -> bool {
    ARTICLE_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `_extract_video(video_block)`: the Brightcove player URL and video ID,
/// or `None` without a `data-video-id`.
fn brightcove_target(block: &str) -> Option<(String, String)> {
    let attributes = crate::html_elements::extract_attributes(block);
    let text = |key: &str| {
        attributes
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let video_id = text("data-video-id")?;
    let account_id = text("data-account-id").unwrap_or_else(|| "876450612001".into());
    let player_id = text("playerid").unwrap_or_else(|| "default".into());
    Some((
        format!(
            "http://players.brightcove.net/{account_id}/{player_id}_default/index.html?videoId={video_id}"
        ),
        video_id,
    ))
}

/// `BFMTVIE`/`BFMTVLiveIE`: the first video block, fatally.
fn single(
    client: &Client,
    url: &Url,
    (name, regex): (&'static str, &Regex),
) -> Result<Option<ExtractResult>> {
    let Some(found) = regex.captures(url.as_str()) else {
        return Ok(None);
    };
    let bfmtv_id = found["id"].to_owned();
    start(name, url);
    let webpage = download_webpage(client, url, name, &bfmtv_id)?;
    let block = VIDEO_BLOCK
        .captures(&webpage)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{name}] {bfmtv_id}: Unable to extract video block; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    let (target, video_id) = brightcove_target(&block).ok_or_else(|| {
        Error::Reported(format!(
            "[{name}] {bfmtv_id}: Unable to extract Brightcove video id; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    // `url_result(url, 'BrightcoveNew', video_id)`.
    let mut follow = super::direct::build_at(url.clone(), url, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow.id = video_id;
    follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
    Ok(Some(ExtractResult::Video(follow)))
}

impl Extractor for Bfmtv {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        single(client, url, (NAME, &VALID_URL))
    }
}

impl Extractor for BfmtvLive {
    fn key(&self) -> &'static str {
        LIVE_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        single(client, url, (LIVE_NAME, &LIVE_URL))
    }
}

/// `url_result(url, 'BrightcoveNew', video_id)` as a playlist entry.
fn brightcove_entry(target: &str, video_id: &str) -> Result<VideoInfo> {
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
            "entry": {"ie_key": "BrightcoveNew", "id": video_id, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for BfmtvArticle {
    fn key(&self) -> &'static str {
        ARTICLE_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ARTICLE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let bfmtv_id = found["id"].to_owned();
        start(ARTICLE_NAME, url);
        let webpage = download_webpage(client, url, ARTICLE_NAME, &bfmtv_id)?;
        // `_og_search_title(webpage, fatal=False)` warns before the
        // generator's entries are read.
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            crate::screen::warning(&format!(
                "[{ARTICLE_NAME}] unable to extract OpenGraph title; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let description = ["og:description", "description"]
            .iter()
            .find_map(|name| crate::html::html_search_meta(&webpage, name));
        // `_html_search_meta`'s non-fatal search warns with the first name.
        if description.is_none() {
            crate::screen::warning(&format!(
                "[{ARTICLE_NAME}] unable to extract og:description; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let mut entries = Vec::new();
        for found in VIDEO_BLOCK.captures_iter(&webpage) {
            match brightcove_target(&found[1]) {
                Some((target, video_id)) => entries.push(brightcove_entry(&target, &video_id)?),
                None => crate::screen::warning(&format!(
                    "[{ARTICLE_NAME}] Unable to extract Brightcove video id"
                )),
            }
        }
        let title = title.filter(|title| !title.is_empty());
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
            format!(
                "playlist-dict;{}{}",
                keys.join(","),
                crate::info_order::GENERATOR
            )
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: bfmtv_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: ARTICLE_NAME.into(),
            extractor_key: ARTICLE_KEY.into(),
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
            "https://www.bfmtv.com/politique/emmanuel-macron-l-islam-est-une-religion-qui-vit-une-crise-aujourd-hui-partout-dans-le-monde_VN-202010020146.html"
        ));
        assert!(!suitable(
            "https://www.bfmtv.com/sante/covid-19_AV-202101060198.html"
        ));
        assert!(live_suitable("https://www.bfmtv.com/en-direct/"));
        assert!(live_suitable("https://www.bfmtv.com/economie/en-direct/"));
        assert!(article_suitable(
            "https://rmc.bfmtv.com/actualites/societe/transports/ce-n-est-plus-tout-rentable_AV-202301100268.html"
        ));
        assert!(article_suitable(
            "https://www.bfmtv.com/international/pour-bolsonaro_AD-202101060232.html"
        ));
    }
}
