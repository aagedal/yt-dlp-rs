//! Upstream `AltCensoredIE` and `AltCensoredChannelIE`
//! (`yt_dlp/extractor/altcensored.py`): a watch page's category and view
//! count as a `url_transparent` Archive.org item, and a channel's paged
//! watch links as url entries.
use super::{Extractor, download_webpage, download_webpage_note};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct AltCensored;
pub struct AltCensoredChannel;

const NAME: &str = "altcensored";
const KEY: &str = "AltCensored";
const CHANNEL_NAME: &str = "altcensored:channel";
const CHANNEL_KEY: &str = "AltCensoredChannel";
const PAGE_SIZE: usize = 24;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?altcensored\.com/(?:watch\?v=|embed/)(?P<id>[^/?#]+)").unwrap()
});
static CHANNEL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?altcensored\.com/channel/(?P<id>[^/?#]+)").unwrap()
});
static CATEGORY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a href="/category/\d+">([^<]+)</a>"#).unwrap());
static VIEWS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"YouTube Views:(?:\s|&nbsp;)*([\d,]+)").unwrap());
/// The page links, the last first; the upstream back-reference `(?:\1)` is
/// checked by hand.
static PAGE_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+href="/channel/[\w-]+/page/(\d+)">(\d+)</a>"#).unwrap());
static WATCH_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+href="(/watch\?v=[^"]+)"#).unwrap());

/// `AltCensoredIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `AltCensoredChannelIE._match_valid_url` (`(?!page|table)`).
pub fn channel_suitable(url: &str) -> bool {
    CHANNEL_URL
        .captures(url)
        .is_some_and(|found| !found["id"].starts_with("page") && !found["id"].starts_with("table"))
}

/// `url_result(target, ie=ie_key)` as a playlist entry.
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

/// Python's `str_to_int`: separators and `+` removed.
fn str_to_int(text: &str) -> Option<i64> {
    text.replace([',', '.', '+'], "").parse().ok()
}

impl Extractor for AltCensored {
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
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let category = CATEGORY
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .filter(|category| !category.is_empty());
        let view_count = VIEWS
            .captures(&webpage)
            .and_then(|found| str_to_int(&found[1]));
        let target = format!("https://archive.org/details/youtube-{video_id}");
        let fields = json!({
            "view_count": view_count,
            "categories": category.map(|category| vec![category]),
        });
        // The `url_transparent` result: the Archive.org item with these fields.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": fields}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

impl Extractor for AltCensoredChannel {
    fn key(&self) -> &'static str {
        CHANNEL_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CHANNEL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        if !channel_suitable(url.as_str()) {
            return Ok(None);
        }
        let channel_id = found["id"].to_owned();
        crate::screen::set_extractor(CHANNEL_NAME);
        crate::screen::screen(&format!(
            "[{CHANNEL_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage_note(
            client,
            url,
            CHANNEL_NAME,
            &channel_id,
            "Download channel webpage",
        )
        .map_err(|error| match error {
            crate::Error::ExtractorRequest { source, .. } => {
                crate::Error::Reported(super::request_errnote_text(
                    CHANNEL_NAME,
                    &channel_id,
                    "Unable to get channel webpage",
                    &source,
                ))
            }
            error => error,
        })?;
        let title = crate::html::html_search_meta(&webpage, "altcen_title");
        if title.is_none() {
            super::search_warning(CHANNEL_NAME, "title");
        }
        // The last page is listed first; `(?:\1)` requires its text to be its number.
        let page_count = PAGE_LINK
            .captures_iter(&webpage)
            .find(|found| found[1] == found[2])
            .and_then(|found| found[1].parse::<i64>().ok())
            .unwrap_or(1);

        // Upstream's `InAdvancePagedList` iteration stops after the first page
        // with fewer than `_PAGE_SIZE` entries.
        let mut entries = Vec::new();
        for page_num in 1..=page_count {
            let page_url = Url::parse(&format!(
                "https://altcensored.com/channel/{channel_id}/page/{page_num}"
            ))?;
            let page = download_webpage_note(
                client,
                &page_url,
                CHANNEL_NAME,
                &channel_id,
                &format!("Downloading page {page_num}"),
            )?;
            let mut seen = Vec::new();
            for found in WATCH_LINK.captures_iter(&page) {
                let path = found[1].to_owned();
                if !seen.contains(&path) {
                    seen.push(path.clone());
                    let target = Url::parse("https://www.altcensored.com")?.join(&path)?;
                    entries.push(url_entry(target.as_str(), KEY, NAME)?);
                }
            }
            if seen.len() < PAGE_SIZE {
                break;
            }
        }

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: channel_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: CHANNEL_NAME.into(),
            extractor_key: CHANNEL_KEY.into(),
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
        assert!(suitable("https://www.altcensored.com/watch?v=k0srjLSkga8"));
        assert!(suitable("https://altcensored.com/embed/k0srjLSkga8"));
        assert!(!suitable(
            "https://www.altcensored.com/channel/UCFPTO55xxHqFqkzRZHu4kcw"
        ));
    }

    #[test]
    fn channel_urls_exclude_page_and_table() {
        assert!(channel_suitable(
            "https://www.altcensored.com/channel/UCFPTO55xxHqFqkzRZHu4kcw"
        ));
        assert!(!channel_suitable(
            "https://www.altcensored.com/channel/page"
        ));
        assert!(!channel_suitable(
            "https://www.altcensored.com/channel/table1"
        ));
    }
}
