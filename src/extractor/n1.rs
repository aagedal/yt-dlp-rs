//! Upstream `N1InfoIIE` (`yt_dlp/extractor/n1.py`): an article's Brid.tv
//! players (HLS from `cdn-uc.brid.tv`) and its YouTube, Reddit and Facebook
//! iframes as URL entries. Upstream's last Brid video ID becomes the
//! playlist ID, and that is kept. The old player's `<video>` tags (which
//! point at `N1InfoAssetIE`) are not supported natively; the asset
//! extractor is not ported.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_master_optional, search_warning};
use crate::html_elements::extract_attributes;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct N1InfoArticle;

const NAME: &str = "N1Info:article";
const KEY: &str = "N1InfoI";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:\w+\.)?n1info\.\w+|nova\.rs)/(?:[^/?#]+/){1,2}(?P<id>[^/?#]+)",
    )
    .unwrap()
});
static PLAYER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\$bp\("(?:Brid|TargetVideo)_\d+",\s(.+)\);"#).unwrap());
static VIDEO_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)(<video[^>]+>)").unwrap());
static IFRAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(<iframe[^>]+>)").unwrap());
static SITE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"site:(\d+)").unwrap());
static PARTNER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"partners/(\d+)").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python's `str()` of a JSON value.
fn python_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

/// `url_result(target, ie=ie_key)` as a playlist entry (resolved when
/// processed, like the other URL entries).
fn url_entry(target: &str, ie_key: &str, extractor: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
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
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": extractor,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for N1InfoArticle {
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
        let page_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &page_id)?;

        // `_og_search_title(webpage) or _html_extract_title(webpage)`.
        let og_title = crate::html::og_search_property(&webpage, "title").filter(|t| !t.is_empty());
        if og_title.is_none() {
            search_warning(NAME, "OpenGraph title");
        }
        let title = og_title.or_else(|| crate::html::html_extract_title(&webpage));

        // `unified_timestamp(_og_search_property(...) or _html_search_meta(...))`.
        let published = crate::html::og_search_property(&webpage, "published_time")
            .filter(|value| !value.is_empty())
            .or_else(|| {
                let found = crate::html::html_search_meta_clean(&webpage, "article:published_time");
                if found.is_none() {
                    search_warning(NAME, "article:published_time");
                }
                found
            });
        let timestamp = published
            .and_then(|date| crate::dates::unified_timestamp(&date, true))
            .map_or(Value::Null, Value::from);

        let mut playlist_id = page_id.clone();
        let mut entries = Vec::new();
        let players: Vec<&str> = PLAYER
            .captures_iter(&webpage)
            .map(|found| found.get(1).map_or("", |m| m.as_str()))
            .collect();
        if !players.is_empty() {
            let site_id = match SITE.captures(&webpage) {
                Some(found) => found[1].to_owned(),
                None => {
                    let content_url = crate::html::html_search_meta_clean(&webpage, "contentUrl")
                        .ok_or_else(|| {
                        Error::Reported(format!(
                            "[{NAME}] {page_id}: Unable to extract contentUrl; {}",
                            crate::BUG_REPORTS_MESSAGE
                        ))
                    })?;
                    PARTNER
                        .captures(&content_url)
                        .map(|found| found[1].to_owned())
                        .ok_or_else(|| {
                            Error::Reported(format!(
                                "[{NAME}] {page_id}: Unable to extract site ID; {}",
                                crate::BUG_REPORTS_MESSAGE
                            ))
                        })?
                }
            };
            for player in players {
                let data: Value = crate::json::from_str(player).map_err(|error| {
                    Error::Reported(format!(
                        "[{NAME}] {page_id}: Failed to parse JSON (caused by {error})"
                    ))
                })?;
                let video = data
                    .get("video")
                    .cloned()
                    .ok_or_else(|| super::clyp::key_error(&page_id, "video"))?;
                let video_id = python_str(&video);
                playlist_id = video_id.clone();
                let thumbnail = crate::html::html_search_meta_clean(&webpage, "thumbnailURL")
                    .map_or(Value::Null, Value::from);
                if thumbnail.is_null() {
                    search_warning(NAME, "thumbnailURL");
                }
                let manifest = format!(
                    "https://cdn-uc.brid.tv/live/partners/{site_id}/streaming/{video_id}/{video_id}.m3u8"
                );
                let formats = match Url::parse(&manifest) {
                    Ok(manifest) => extract_m3u8_master_optional(
                        client, &manifest, NAME, &video_id, None, false, false,
                    )
                    .map(|stream| stream.formats)
                    .unwrap_or_default(),
                    Err(_) => Vec::new(),
                };
                let page_url = url.clone();
                let mut entry = dict_info(
                    &page_url,
                    (NAME, KEY),
                    vec![
                        ("id", video.clone()),
                        ("title", title.clone().map_or(Value::Null, Value::from)),
                        ("timestamp", timestamp.clone()),
                        ("thumbnail", thumbnail),
                        ("formats", Value::Null),
                    ],
                    formats,
                );
                entry.webpage_url = page_url;
                entries.push(entry);
            }
        } else if VIDEO_TAG.is_match(&webpage) {
            return Err(Error::UnsupportedHtml5(
                "N1Info articles with old-player <video> tags (N1InfoAsset) are not supported natively"
                    .into(),
            ));
        }

        for iframe in IFRAME.find_iter(&webpage) {
            let attributes = extract_attributes(iframe.as_str());
            let src = attributes
                .get("src")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let host = Url::parse(&src)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned));
            match host.as_deref() {
                Some("www.youtube.com") => entries.push(url_entry(&src, "Youtube", "youtube")?),
                Some("www.redditmedia.com") => entries.push(url_entry(&src, "Reddit", "reddit")?),
                Some("www.facebook.com") if src.contains("plugins/video") => {
                    entries.push(url_entry(&src, "FacebookPluginsVideo", "facebook:plugin")?)
                }
                _ => {}
            }
        }

        let mut extra = Map::new();
        extra.insert("timestamp".into(), timestamp);
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;_type,id,title,timestamp".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
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
            "https://sportklub.n1info.rs/tenis/us-open/glava-telo-igra-kako-je-novak-ispustio-istorijsku-sansu/"
        ));
        assert!(suitable(
            "https://n1info.si/novice/svet/v-srbiji-samo-ta-konec-tedna-vec-kot-200-pozarov/"
        ));
        assert!(suitable(
            "https://nova.rs/vesti/politika/zaklina-tatalovic-ani-brnabic-pricate-lazi-video/"
        ));
        assert!(!suitable("https://n1info.si/"));
    }
}
