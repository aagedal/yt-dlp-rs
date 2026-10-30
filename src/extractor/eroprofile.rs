//! Upstream `EroProfileAlbumIE` (`yt_dlp/extractor/eroprofile.py`): an
//! album's `/m/videos/view/` links across its `?pnum=` pages as url results.
//! The single-video `EroProfileIE` (login, `glbUpdViews` ID) is not ported.
use super::{Extractor, download_webpage_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct EroProfileAlbum;

const NAME: &str = "EroProfile:album";
const VIEW_IE: &str = "EroProfile";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?eroprofile\.com/m/videos/album/(?P<id>[^/]+)").unwrap()
});
static VIEW_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"href=".*?(/m/videos/view/[^"]+)""#).unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<title>Album: (.*) - EroProfile</title>").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// A `url_result(url, 'EroProfile')` playlist entry, resolved when processed.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), VIEW_IE.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": VIEW_IE,
            "extractor": VIEW_IE.to_lowercase(),
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `_extract_from_page`: the page's video links as url results.
fn page_entries(page: &str) -> Result<Vec<VideoInfo>> {
    VIEW_LINK
        .captures_iter(page)
        .map(|found| url_entry(&format!("https://www.eroprofile.com{}", &found[1])))
        .collect()
}

impl Extractor for EroProfileAlbum {
    fn key(&self) -> &'static str {
        "EroProfileAlbum"
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

        let first_page =
            download_webpage_note(client, url, NAME, &playlist_id, "Downloading playlist")?;
        let Some(title) = TITLE.captures(&first_page).map(|found| found[1].to_owned()) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {playlist_id}: Unable to extract playlist_title; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };

        let mut entries = page_entries(&first_page)?;
        let pages = Regex::new(&format!(
            r#"href=".*?(/m/videos/album/{}\?pnum=(\d+))""#,
            regex::escape(&playlist_id)
        ))
        .map_err(|error| Error::Reported(error.to_string()))?;
        let max_page = pages
            .captures_iter(&first_page)
            .filter_map(|found| found[2].parse::<u64>().ok())
            .max()
            .ok_or_else(|| Error::Reported("max() arg is an empty sequence".into()))?;

        for number in 2..=max_page {
            let page_url = Url::parse(&format!(
                "https://www.eroprofile.com/m/videos/album/{playlist_id}?pnum={number}"
            ))?;
            let page = download_webpage_note(
                client,
                &page_url,
                NAME,
                &playlist_id,
                &format!("Downloading playlist page {}", number - 1),
            )?;
            entries.extend(page_entries(&page)?);
        }

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title,
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: "EroProfileAlbum".into(),
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
            "https://www.eroprofile.com/m/videos/album/BBW-2-893"
        ));
        assert!(!suitable(
            "https://www.eroprofile.com/m/videos/view/sexy-babe-softcore"
        ));
    }
}
