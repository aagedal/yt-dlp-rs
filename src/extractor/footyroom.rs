//! Upstream `FootyRoomIE` (`yt_dlp/extractor/footyroom.py`): FootyRoom
//! match pages as playlists of the Streamable embeds in the page's
//! `DataStore.media` payloads, with the OpenGraph title.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct FootyRoom;

const NAME: &str = "FootyRoom";
const STREAMABLE: &str = "Streamable";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://footyroom\.com/matches/(?P<id>\d+)").unwrap()
});
static MEDIA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"DataStore\.media\s*=\s*([^;]+)").unwrap());
static STREAMABLE_EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+\bsrc=(?:"(?P<a>(?:https?:)?//streamable\.com/.+?)"|'(?P<b>(?:https?:)?//streamable\.com/.+?)')"#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("FootyRoom {what} are not supported natively"))
}

/// `StreamableIE._extract_url(payload)`: the first embed, unescaped, not
/// joined to any page (`urljoin(None, ...)`) and suitable, so
/// protocol-relative embeds are dropped.
fn streamable_url(payload: &str) -> Option<String> {
    STREAMABLE_EMBED.captures_iter(payload).find_map(|found| {
        let raw = found.name("a").or_else(|| found.name("b"))?.as_str();
        let embed = crate::html::unescape_html(raw);
        super::streamable::suitable(&embed).then_some(embed)
    })
}

/// `url_result(streamable_url, StreamableIE.ie_key())`.
fn streamable_entry(target: &str) -> Result<VideoInfo> {
    let absolute = target.to_owned();
    let url = Url::parse(&absolute)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // A flat `url` result's ID is its extractor's temporary ID.
    if let Some(id) = super::streamable::temp_id(&absolute) {
        info.id = id;
    }
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": STREAMABLE,
            "extractor": STREAMABLE,
            "entry": {"ie_key": STREAMABLE, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for FootyRoom {
    fn key(&self) -> &'static str {
        NAME
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
        let webpage = download_webpage(client, url, NAME, &playlist_id)?;
        let raw = MEDIA
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {playlist_id}: Unable to extract media data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let playlist: Value = crate::json::from_str(&raw).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {playlist_id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            super::search_warning(NAME, "OpenGraph title");
        }
        let videos = match &playlist {
            Value::Array(videos) => videos,
            _ => return Err(unsupported("media data that are not lists")),
        };
        let mut entries = Vec::new();
        for video in videos {
            let Value::Object(video) = video else {
                return Err(unsupported("media items that are not objects"));
            };
            let payload = match video.get("payload") {
                None | Some(Value::Null) => continue,
                Some(Value::String(payload)) if payload.is_empty() => continue,
                Some(Value::String(payload)) => payload,
                Some(_) => return Err(unsupported("payloads that are not strings")),
            };
            if let Some(target) = streamable_url(payload) {
                entries.push(streamable_entry(&target)?);
            }
        }
        let title = title.filter(|title| !title.is_empty());
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            if title.is_some() {
                "playlist-dict;id,title"
            } else {
                "playlist-dict;id"
            }
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
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
            "http://footyroom.com/matches/79922154/hull-city-vs-chelsea/review"
        ));
        assert!(!suitable("http://footyroom.com/news/1"));
    }
}
