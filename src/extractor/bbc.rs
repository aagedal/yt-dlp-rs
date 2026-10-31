//! Upstream `BBCCoUkPlaylistIE` (`yt_dlp/extractor/bbc.py`): a programme's
//! clips, episodes or broadcasts page: its `data-pid` programmes as `BBCCoUk`
//! url entries, following the `pagination_next` links (a `page` query
//! parameter stops after that page). `BBCIE`, `BBCCoUkIE` and the other BBC
//! classes are not ported.
use super::{Extractor, download_webpage, download_webpage_note, search_warning};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BbcCoUkPlaylist;

const NAME: &str = "bbc.co.uk:playlist";
const KEY: &str = "BBCCoUkPlaylist";
/// `BBCCoUkIE.ie_key()`.
const ENTRY_KEY: &str = "BBCCoUk";
/// `BBCCoUkIE._ID_REGEX`.
const ID_REGEX: &str = "(?:[pbml][0-9a-z]{7}|w[0-9a-z]{7,14})";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^https?://(?:www\.)?bbc\.co\.uk/programmes/(?P<id>{ID_REGEX})/(?:episodes|broadcasts|clips)"
    ))
    .unwrap()
});
static VIDEO_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r#"data-pid=["']({ID_REGEX})"#)).unwrap());
/// `_search_regex(<li ... class="pagination_+next" ... href=...)`, group 1 or 2.
static NEXT_PAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<li[^>]+class=(?:"pagination_+next"|'pagination_+next')[^>]*><a[^>]+href=(?:"([^"]+)"|'([^']+)')"#,
    )
    .unwrap()
});

/// `BBCCoUkPlaylistIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `url_result('http://www.bbc.co.uk/programmes/ID', ie=BBCCoUkIE)` as a playlist entry.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ENTRY_KEY.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ENTRY_KEY,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for BbcCoUkPlaylist {
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
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let mut webpage = download_webpage(client, url, NAME, &playlist_id)?;

        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            search_warning(NAME, "OpenGraph title");
        }
        let description = crate::html::og_search_property(&webpage, "description");

        // `'page' in parse_qs(query)`: a non-blank `page` stops after the first page.
        let single_page = url
            .query_pairs()
            .any(|(key, value)| key == "page" && !value.is_empty());
        let mut entries = Vec::new();
        for page_num in 2.. {
            let mut seen = Vec::new();
            for found in VIDEO_ID.captures_iter(&webpage) {
                seen.push(found[1].to_owned());
            }
            for video_id in seen {
                entries.push(url_entry(&format!(
                    "http://www.bbc.co.uk/programmes/{video_id}"
                ))?);
            }
            if single_page {
                break;
            }
            let Some(next) = NEXT_PAGE
                .captures(&webpage)
                .and_then(|found| found.get(1).or_else(|| found.get(2)))
                .map(|found| found.as_str().to_owned())
            else {
                break;
            };
            let next_url = url.join(&next)?;
            webpage = download_webpage_note(
                client,
                &next_url,
                NAME,
                &playlist_id,
                &format!("Downloading page {page_num}"),
            )?;
        }

        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        if description.is_some() {
            keys.push("description");
        }
        let mut extra = Map::new();
        if let Some(description) = description {
            extra.insert("description".into(), description.into());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
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
            "http://www.bbc.co.uk/programmes/b00mfl7n/clips?page=1"
        ));
        assert!(suitable(
            "http://www.bbc.co.uk/programmes/b05rcz9v/broadcasts/2016/06"
        ));
        assert!(!suitable("http://www.bbc.co.uk/programmes/b00mfl7n"));
    }
}
