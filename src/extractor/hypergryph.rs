//! Upstream `MonsterSirenHypergryphMusicIE` (`yt_dlp/extractor/hypergryph.py`):
//! Monster Siren songs from the song API (its error messages, source URL,
//! artists and lyrics) with the album's name, intro and cover.
use super::{Extractor, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct MonsterSirenHypergryphMusic;

const NAME: &str = "monstersiren";
const KEY: &str = "MonsterSirenHypergryphMusic";
const API_BASE: &str = "https://monster-siren.hypergryph.com/api";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://monster-siren\.hypergryph\.com/music/(?P<id>\d+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `_download_json(..., fatal=False)`: a request error is a warning.
fn album_json(client: &Client, album_id: &str) -> Result<Option<Value>> {
    let api = Url::parse(&format!("{API_BASE}/album/{album_id}/detail"))?;
    match download_json(client, &api, NAME, album_id) {
        Ok(album) => Ok(Some(album)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {album_id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

impl Extractor for MonsterSirenHypergryphMusic {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let audio_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!("{API_BASE}/song/{audio_id}"))?;
        let song = download_json(client, &api, NAME, &audio_id)?;
        // `traverse_obj(song, 'code') != 0` (Python's `False == 0`).
        let ok = matches!(song.get("code"), Some(Value::Number(code)) if code.as_f64() == Some(0.0))
            || song.get("code") == Some(&Value::Bool(false));
        if !ok {
            let message = song
                .get("msg")
                .and_then(Value::as_str)
                .filter(|message| !message.is_empty());
            return Err(Error::Reported(match message {
                Some(message) => format!("[{NAME}] {audio_id}: {message}"),
                None => format!(
                    "[{NAME}] {audio_id}: API returned an error response; {}",
                    crate::BUG_REPORTS_MESSAGE
                ),
            }));
        }
        let data = song.get("data").filter(|data| data.is_object());
        let album = match data
            .and_then(|data| data.get("albumCid"))
            .and_then(Value::as_str)
        {
            Some(album_id) => album_json(client, album_id)?,
            None => None,
        };
        let mut fields: Vec<(&str, Value)> =
            vec![("id", audio_id.into()), ("vcodec", "none".into())];
        let mut lyrics = None;
        if let Some(data) = data {
            if let Some(title) = data.get("name").filter(|name| name.is_string()) {
                fields.push(("title", title.clone()));
            }
            let artists: Vec<Value> = match data.get("artists") {
                Some(Value::Array(artists)) => artists
                    .iter()
                    .filter(|artist| artist.is_string())
                    .cloned()
                    .collect(),
                _ => Vec::new(),
            };
            // Empty branches are dropped from the traversal dict.
            if !artists.is_empty() {
                fields.push(("artists", Value::Array(artists)));
            }
            lyrics = url_or_none(data.get("lyricUrl"));
            if lyrics.is_some() {
                fields.push(("subtitles", Value::Null));
            }
            if let Some(source) = url_or_none(data.get("sourceUrl")) {
                fields.push(("url", source.into()));
            }
        }
        if let Some(album) = album
            .as_ref()
            .and_then(|album| album.get("data"))
            .filter(|data| data.is_object())
        {
            if let Some(name) = album.get("name").filter(|name| name.is_string()) {
                fields.push(("album", name.clone()));
            }
            if let Some(intro) = album.get("intro").and_then(Value::as_str) {
                fields.push(("description", crate::html::clean_html(intro).into()));
            }
            if let Some(cover) = url_or_none(album.get("coverUrl")) {
                fields.push(("thumbnail", cover.into()));
            }
        }
        let media = fields
            .iter()
            .find(|(key, _)| *key == "url")
            .and_then(|(_, value)| value.as_str())
            .map(str::to_owned);
        let formats = match media {
            Some(media) => {
                let ext =
                    super::direct::determine_ext(&media).unwrap_or_else(|| "unknown_video".into());
                let mut format: crate::VideoFormat = serde_json::from_value(
                    json!({"format_id": "", "url": media, "ext": ext, "vcodec": "none"}),
                )
                .map_err(|error| Error::Reported(format!("[{NAME}] {error}")))?;
                crate::info_order::mark_format(&mut format, "url-only");
                vec![format]
            }
            None => Vec::new(),
        };
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        // Without a `title` key, processing adds the generic one after
        // `fulltitle`.
        let untitled = if keys.contains(&"title") {
            ""
        } else {
            "-untitled"
        };
        let shape = format!("untyped-dict;{}{untitled}", keys.join(","));
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        if let Some(lyrics) = lyrics {
            info.subtitles.insert(
                Some("en".into()),
                vec![
                    serde_json::from_value(json!({
                        "url": lyrics,
                        // Processing's `determine_ext(url)`.
                        "ext": super::direct::determine_ext(&lyrics).unwrap_or_default(),
                    }))
                    .map_err(|error| Error::Reported(format!("[{NAME}] {error}")))?,
                ],
            );
        }
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://monster-siren.hypergryph.com/music/514562"
        ));
        assert!(!suitable("https://monster-siren.hypergryph.com/album/1"));
    }
}
