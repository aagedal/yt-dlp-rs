//! Upstream `PeerTVIE` (`yt_dlp/extractor/peertv.py`): the page's player
//! key, the player script's session ID and the player page's HLS playlist.
use super::{Extractor, download_webpage, download_webpage_note, download_webpage_note_headers};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct PeerTv;

const NAME: &str = "peer.tv";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?peer\.tv/(?:de|it|en)/(?P<id>\d+)").unwrap()
});
static VIDEO_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"player\.peer\.tv/js/([a-zA-Z0-9]+)").unwrap());
static SESSION_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"["']session_id["']:\s*["']([a-zA-Z0-9]+)["']"#).unwrap());
static PLAYLIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"["']playlist_url["']:\s*(["'][^"']+["'])"#).unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<h1>(.+?)</h1>").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_regex(pattern, text, name)` (fatal).
fn search<'a>(pattern: &Regex, text: &'a str, id: &str, name: &str) -> Result<&'a str> {
    pattern
        .captures(text)
        .and_then(|found| found.get(1))
        .map(|found| found.as_str())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

impl Extractor for PeerTv {
    fn key(&self) -> &'static str {
        "PeerTV"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        // `_html_search_regex`.
        let video_key = crate::html::clean_html(search(&VIDEO_KEY, &webpage, &id, "video key")?)
            .trim()
            .to_owned();
        let js = download_webpage_note_headers(
            client,
            &Url::parse(&format!("https://player.peer.tv/js/{video_key}/"))?,
            NAME,
            &id,
            "Downloading session id",
            &[("Referer", "https://www.peer.tv/")],
        )?;
        let session_id = search(&SESSION_ID, &js, &id, "session id")?;
        let player = download_webpage_note(
            client,
            &Url::parse(&format!(
                "https://player.peer.tv/jsc/{video_key}/{session_id}?jsr=aHR0cHM6Ly93d3cucGVlci50di9kZS84NDE=&cs=UTF-8&mq=2&ua=0&webm=p&mp4=p&hls=1"
            ))?,
            NAME,
            &id,
            "Downloading player webpage",
        )?;
        let playlist = search(&PLAYLIST, &player, &id, "m3u8 url")?;
        let playlist = serde_json::from_str::<Value>(&crate::js_to_json::js_to_json(playlist))
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Failed to parse JSON; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let mut info = super::extract_m3u8_fatal(
            client,
            &Url::parse(&playlist)?,
            NAME,
            &id,
            Some("hls"),
            false,
        )?;
        super::hls::without_ext(&mut info);
        let title = crate::html::clean_html(search(&TITLE, &webpage, &id, "title")?)
            .trim()
            .replace('\u{a0}', " ");
        // `_html_search_meta((NAME, ...), webpage)` warns with the first name.
        let meta = |names: &[&str]| {
            let found = names
                .iter()
                .find_map(|name| crate::html::html_search_meta(&webpage, name));
            if found.is_none() {
                super::search_warning(NAME, names[0]);
            }
            found
        };
        let description = meta(&["og:description", "description"]);
        let thumbnail = meta(&["og:image", "image"]);
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = "PeerTV".into();
        info.metadata.description = description;
        info.metadata.thumbnail = thumbnail;
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,formats,description,thumbnail",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.peer.tv/de/841"));
        assert!(suitable("https://www.peer.tv/it/404"));
        assert!(!suitable("https://www.peer.tv/fr/404"));
    }
}
