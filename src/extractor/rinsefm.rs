//! Upstream `RinseFMIE` and `RinseFMArtistPlaylistIE`
//! (`yt_dlp/extractor/rinsefm.py`): the query API's episode entry as an
//! untyped audio result, and a show's audio episodes as a playlist of them.
use super::{Extractor, download_json_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct RinseFm;
pub struct RinseFmArtistPlaylist;

const NAME: &str = "RinseFM";
const PLAYLIST_NAME: &str = "RinseFMArtistPlaylist";
const API_BASE: &str = "https://rinse.fm/api/query/v1";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?rinse\.fm/episodes/(?P<id>[^/?#]+)").unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?rinse\.fm/shows/(?P<id>[^/?#]+)").unwrap());

/// `RinseFMIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `RinseFMArtistPlaylistIE._match_id`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Rinse FM {what} are not supported natively"))
}

fn text<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

/// `RinseFMBaseIE._parse_entry`.
fn parse_entry(entry: &Value, fallback: &Url) -> Result<VideoInfo> {
    let id = text(entry, "id").ok_or_else(|| unsupported("episodes without a string ID"))?;
    let media = text(entry, "fileUrl")
        .and_then(super::json_ld::url_or_none)
        .ok_or_else(|| unsupported("episodes without a media URL"))?;
    let title = text(entry, "title").map(str::to_owned);
    let release_timestamp = text(entry, "episodeDate").and_then(crate::dates::parse_iso8601);
    let thumbnail = entry
        .get("featuredImage")
        .and_then(|images| images.get(0))
        .and_then(|image| text(image, "filename"))
        .filter(|name| !name.is_empty())
        .map(|name| format!("https://rinse.imgix.net/media/{name}"));
    let webpage_url = text(entry, "slug")
        .filter(|slug| !slug.is_empty())
        .map(|slug| format!("https://rinse.fm/episodes/{slug}"));
    let media = Url::parse(&media)?;
    let ext =
        super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
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
    info.id = id.to_owned();
    let mut keys = vec!["id"];
    match title {
        Some(title) => {
            info.title = title;
            keys.push("title");
        }
        None => {
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
    }
    keys.push("url");
    if let Some(timestamp) = release_timestamp {
        crate::metadata_parser::set_field(&mut info, "release_timestamp", timestamp.into());
        keys.push("release_timestamp");
    }
    if let Some(thumbnail) = thumbnail {
        info.metadata.thumbnail = Some(thumbnail);
        keys.push("thumbnail");
    }
    match webpage_url {
        Some(page) => {
            info.webpage_url = Url::parse(&page)?;
            keys.push("webpage_url");
        }
        None => info.webpage_url = fallback.clone(),
    }
    keys.extend(["vcodec", "extractor_key", "extractor"]);
    crate::metadata_parser::set_field(&mut info, "vcodec", "none".into());
    info.direct = None;
    info.extractor = NAME.into();
    info.extractor_key = NAME.into();
    Ok(crate::info_order::mark_info(
        info,
        &format!(
            "untyped-dict;{}{}",
            keys.join(","),
            if keys.contains(&"title") {
                ""
            } else {
                "-untitled"
            }
        ),
    ))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for RinseFm {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        announce(NAME, url);
        let api = Url::parse(&format!("{API_BASE}/episodes/{display_id}"))?;
        let data = download_json_note(
            client,
            &api,
            NAME,
            &display_id,
            "Downloading episode data from API",
        )?;
        let entry = data
            .get("entry")
            .ok_or_else(|| key_error(&display_id, "entry"))?;
        parse_entry(entry, url).map(Some)
    }
}

/// `determine_ext(v['fileUrl']) in MEDIA_EXTENSIONS.audio`.
fn is_audio(media: &str) -> bool {
    const AUDIO: [&str; 22] = [
        "aac", "ape", "asf", "f4a", "f4b", "m4b", "m4r", "oga", "ogx", "spx", "vorbis", "wma",
        "weba", "aiff", "alac", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav",
    ];
    super::direct::determine_ext(media).is_some_and(|ext| AUDIO.contains(&ext.as_str()))
}

impl Extractor for RinseFmArtistPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        announce(PLAYLIST_NAME, url);
        let api = Url::parse(&format!("{API_BASE}/shows/{playlist_id}"))?;
        let data = download_json_note(
            client,
            &api,
            PLAYLIST_NAME,
            &playlist_id,
            "Downloading show data from API",
        )?;
        let mut entries = Vec::new();
        if let Some(Value::Array(episodes)) = data.get("episodes") {
            for episode in episodes {
                if text(episode, "fileUrl").is_some_and(is_audio) {
                    entries.push(parse_entry(episode, url)?);
                }
            }
        }
        let show = data.get("entry");
        let title = show.and_then(|show| text(show, "title"));
        let description = show.and_then(|show| text(show, "description"));
        let mut keys = Vec::new();
        let mut extra = Map::new();
        if title.is_some() {
            keys.push("title");
        }
        if let Some(description) = description {
            keys.push("description");
            extra.insert("description".into(), description.into());
        }
        keys.push("id");
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default().to_owned(),
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_NAME.into(),
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
            "https://rinse.fm/episodes/club-glow-15-12-2023-2000/"
        ));
        assert!(playlist_suitable("https://www.rinse.fm/shows/esk"));
        assert!(!suitable("https://rinse.fm/shows/esk"));
        assert!(!playlist_suitable("https://rinse.fm/episodes/x"));
    }
}
