//! Upstream `NekoHackerIE` (`yt_dlp/extractor/nekohacker.py`): a page's
//! `playlist` list of `data-audiopath` tracks (direct MP3 URLs) and the
//! page's `srp_player_params` artwork. Pages that only embed something are
//! errors here: Spotify embeds, and the generic hand-off, which is not ported.
use super::{Extractor, direct, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct NekoHacker;

const NAME: &str = "NekoHacker";
const ARTIST: &str = "Neko Hacker";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?nekohacker\.com/(?P<id>[\w-]+)").unwrap()
});
static IFRAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<iframe\b[^>]*>").unwrap());
static TRACK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(<li[^>]+data-audiopath[^>]+>)").unwrap());
static SPOTIFY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:\w+\.)?spotify\.com/").unwrap());
static PLAYER_PARAMS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+srp_player_params_[\da-f]+\s*=").unwrap());

/// `_match_valid_url`, with the `(?!free-dl)` lookahead on the ID.
pub fn suitable(url: &str) -> bool {
    VALID_URL
        .captures(url)
        .is_some_and(|found| !found["id"].starts_with("free-dl"))
}

/// `get_element_by_class('playlist', webpage)`: the element's HTML.
fn playlist_element(webpage: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class("playlist", webpage)
        .ok()?
        .into_iter()
        .next()
        .map(|(_, html)| html)
}

/// `_search_json(r'var srp_player_params_...\s*=', ..., default={})`.
fn player_params(webpage: &str) -> Map<String, Value> {
    super::search_json(&PLAYER_PARAMS, webpage, false)
        .flatten()
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

/// `re.match(r'\d{8}', x.replace('.', ''))`, group 0.
fn release_date(text: &str) -> Option<String> {
    let digits = text.replace('.', "");
    let head = digits.get(..8)?;
    head.chars()
        .all(|character| character.is_ascii_digit())
        .then(|| head.to_owned())
}

/// `url_or_none(text)`.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// One `data-audiopath` track: `untyped-dict` of its `traverse_obj` fields,
/// then the page's artwork, track number, artist and codecs. Also returns
/// the track's album, which names the playlist.
fn track_entry(
    page: &Url,
    attributes: &Map<String, Value>,
    thumbnail: Option<String>,
    track_number: i64,
) -> Option<(VideoInfo, Option<String>)> {
    let media_url = url_or_none(attributes.get("data-audiopath"))?;
    let media = Url::parse(&media_url).ok()?;
    let ext = direct::determine_ext(&media_url).unwrap_or_default();
    let text = |key: &str| attributes.get(key).and_then(Value::as_str);
    let mut keys = vec!["url", "ext"];
    let mut info = super::direct::with_format(
        super::direct::build_at(media.clone(), &media, ext.clone()),
        "",
    );
    for format in &mut info.formats {
        format.ext = ext.clone();
        format.format_id = String::new();
        format.vcodec = Some("none".into());
    }
    info.format_id = None;
    info.direct = None;
    info.webpage_url = page.clone();
    info.extractor = NAME.into();
    info.extractor_key = NAME.into();
    let mut set = |key: &'static str, value: Value| {
        crate::metadata_parser::set_field(&mut info, key, value);
        keys.push(key);
    };
    if let Some(id) = text("data-trackid") {
        set("id", id.into());
    }
    if let Some(title) = text("data-tracktitle") {
        set("title", title.into());
        set("track", title.into());
    }
    if let Some(album) = text("data-albumtitle") {
        set("album", album.into());
    }
    if let Some(duration) = text("data-tracktime")
        .and_then(crate::parse_duration)
        .and_then(|duration| serde_json::to_value(duration).ok())
    {
        set("duration", duration);
    }
    if let Some(date) = text("data-releasedate").and_then(release_date) {
        set("release_date", date.into());
    }
    if let Some(thumbnail) = thumbnail {
        set("thumbnail", thumbnail.into());
    }
    set("track_number", track_number.into());
    set("artists", Value::from(vec![ARTIST]));
    set("vcodec", "none".into());
    if ext == "mp3" {
        set("acodec", "mp3".into());
    }
    // `dict_info` marks the keys in upstream's order; the ID is set last so
    // the playlist's id comes from the page, as upstream does.
    let mut info = crate::info_order::mark_info(info, &format!("untyped-dict;{}", keys.join(",")));
    info.id = text("data-trackid").unwrap_or_default().to_owned();
    Some((info, text("data-albumtitle").map(str::to_owned)))
}

impl Extractor for NekoHacker {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        if playlist_id.starts_with("free-dl") {
            return Ok(None);
        }
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &playlist_id)?;
        let Some(playlist) = playlist_element(&webpage).filter(|html| !html.is_empty()) else {
            let iframe_src = IFRAME
                .find(&webpage)
                .map(|tag| crate::html_elements::extract_attributes(tag.as_str()))
                .and_then(|attributes| url_or_none(attributes.get("src")));
            let Some(iframe_src) = iframe_src else {
                return Err(Error::Reported(format!(
                    "[{NAME}] {playlist_id}: No playlist or embed found in webpage; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            };
            if SPOTIFY.is_match(&iframe_src) {
                return Err(Error::Reported(format!(
                    "[{NAME}] {playlist_id}: Spotify embeds are not supported"
                )));
            }
            return Err(Error::UnsupportedHtml5(
                "NekoHacker pages that hand off to the generic extractor are not supported natively".into(),
            ));
        };

        let params = player_params(&webpage);
        let thumbnail = url_or_none(params.get("artwork"));
        let mut entries = Vec::new();
        let mut album = None;
        for (index, found) in TRACK.find_iter(&playlist).enumerate() {
            let attributes = crate::html_elements::extract_attributes(found.as_str());
            if let Some((entry, entry_album)) =
                track_entry(url, &attributes, thumbnail.clone(), index as i64 + 1)
            {
                // `traverse_obj(entries, (0, 'album'))`.
                if entries.is_empty() {
                    album = entry_album;
                }
                entries.push(entry);
            }
        }
        Ok(Some(playlist_result(url, &playlist_id, album, entries)))
    }
}

/// `playlist_result(entries, playlist_id, title)`.
fn playlist_result(
    page: &Url,
    playlist_id: &str,
    title: Option<String>,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut keys = vec!["id"];
    if title.is_some() {
        keys.push("title");
    }
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id: playlist_id.to_owned(),
        title: title.unwrap_or_default(),
        webpage_url: page.clone(),
        extractor: NAME.into(),
        extractor_key: NAME.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_urls_match_upstream_valid_url() {
        assert!(suitable("https://nekohacker.com/nekoverse/"));
        assert!(suitable("https://www.nekohacker.com/susume/"));
        assert!(!suitable("https://nekohacker.com/free-dl-something/"));
        assert!(!suitable("https://example.com/nekoverse/"));
    }

    #[test]
    fn release_dates_keep_eight_leading_digits() {
        assert_eq!(release_date("2022.11.01").as_deref(), Some("20221101"));
        assert_eq!(release_date("20221101").as_deref(), Some("20221101"));
        assert_eq!(release_date("2022-11-01"), None);
    }
}
