//! Upstream `AudiusBaseIE`, `AudiusIE`, `AudiusTrackIE`, `AudiusPlaylistIE`
//! and `AudiusProfileIE` (`yt_dlp/extractor/audius.py`): the discovery host
//! list (one is picked at random), then a track's `resolve`d metadata as an
//! audio result, or a playlist's or profile's tracks as `url` entries.
//! Backslash-carrying titles are re-quoted the way upstream does.
use super::{Extractor, direct, download_json_note, nrk};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::hash::{BuildHasher, Hasher};
use std::sync::LazyLock;
use url::Url;

pub struct Audius;
pub struct AudiusTrack;
pub struct AudiusPlaylist;
pub struct AudiusProfile;

const NAME: &str = "Audius";
const TRACK_NAME: &str = "audius:track";
const TRACK_IE_KEY: &str = "AudiusTrack";
const PLAYLIST_NAME: &str = "audius:playlist";
const PLAYLIST_IE_KEY: &str = "AudiusPlaylist";
const PROFILE_NAME: &str = "audius:artist";
const PROFILE_IE_KEY: &str = "AudiusProfile";
const DISCOVERY: &str = "https://api.audius.co/";
const API_VERSION: &str = "/v1";

static TRACK_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?audius\.co/(?P<uploader>[\w\d-]+)/(?P<title>\S+)").unwrap()
});
static TRACK_ID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^audius:(?:https?://(?:www\.)?.+/v1/tracks/)?(?P<track_id>\w+)").unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?audius\.co/(?P<uploader>[\w\d-]+)/(?:album|playlist)/(?P<title>\S+)",
    )
    .unwrap()
});
static PROFILE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www)?audius\.co/(?P<id>[^/]+)/?(?:[?#]|$)").unwrap()
});

/// `AudiusIE`'s pattern without its `(?!/album|/playlist)` lookahead.
fn track_match(url: &str) -> Option<(String, String)> {
    let found = TRACK_URL.captures(url)?;
    let uploader_end = found.name("uploader")?.end();
    let rest = &url[uploader_end..];
    if rest.starts_with("/album") || rest.starts_with("/playlist") {
        return None;
    }
    Some((found["title"].to_owned(), url.to_owned()))
}

/// `urllib.parse.unquote` (invalid UTF-8 replaced).
fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[index + 1..index + 3])
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `AudiusBaseIE._prepare_url`: a title's `/` (and `%2F`) become `%5C`.
fn prepare_url(url: &str, title: &str) -> (String, String) {
    let url = unquote(url);
    let title = unquote(title);
    if title.contains('/') || title.contains("%2F") {
        let fixed = title.replace('/', "%5C").replace("%2F", "%5C");
        return (url.replace(&title, &fixed), title);
    }
    (url, title)
}

/// Percent-encodes what urllib's request line does for these URLs (every
/// byte except letters, digits and `-._~:/%`).
fn quote(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~:/%".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A random index into `len` items (`random.choice`).
fn random_index(len: usize) -> usize {
    let hasher = std::collections::hash_map::RandomState::new().build_hasher();
    (hasher.finish() as usize) % len
}

/// `_select_api_base`: a random discovery host.
fn select_api_base(client: &Client) -> Result<String> {
    let url = Url::parse(DISCOVERY)?;
    let response = download_json_note(client, &url, NAME, "", "Requesting available API hosts")?;
    let hosts = response
        .get("data")
        .and_then(Value::as_array)
        .filter(|hosts| !hosts.is_empty())
        .ok_or_else(|| Error::Reported(format!("[{NAME}] Unable to get available API hosts")))?;
    let host = hosts[random_index(hosts.len())]
        .as_str()
        .ok_or_else(|| Error::Reported(format!("[{NAME}] Unable to get available API hosts")))?;
    Ok(host.trim_end_matches('/').to_owned())
}

/// `_get_response_data`.
fn response_data(response: &Value) -> Result<Value> {
    if let Some(object) = response.as_object() {
        if let Some(data) = object.get("data").filter(|data| !data.is_null()) {
            return Ok(data.clone());
        }
        if object.len() == 1
            && let Some(message) = object.get("message")
        {
            let text = message
                .as_str()
                .map_or_else(|| message.to_string(), str::to_owned);
            return Err(Error::Reported(format!("[{NAME}] API error: {text}")));
        }
    }
    Err(Error::Reported(format!(
        "[{NAME}] Unexpected API response; {}",
        crate::BUG_REPORTS_MESSAGE
    )))
}

/// `_api_request(path, item_id, expected_status=...)`.
fn api_request(
    client: &Client,
    base: &str,
    path: &str,
    item_id: &str,
    expect_404: bool,
    note: &str,
) -> Result<Value> {
    let url = Url::parse(&format!("{base}{API_VERSION}{path}"))?;
    let response = if expect_404 {
        let mut response = direct::request_expecting(client, &url, &[], &[404])?;
        let mut body = Vec::new();
        std::io::Read::read_to_end(&mut response, &mut body).map_err(Error::ExtractionRead)?;
        serde_json::from_slice::<Value>(&body).map_err(|_| {
            Error::Reported(format!(
                "[{NAME}] {item_id}: An error occurred while receiving data. Try again"
            ))
        })?
    } else {
        download_json_note(client, &url, NAME, item_id, note)?
    };
    response_data(&response)
}

/// `_resolve_url(url, item_id)`.
fn resolve(client: &Client, base: &str, url: &str, item_id: &str) -> Result<Value> {
    api_request(
        client,
        base,
        &format!("/resolve?url={}", quote(url)),
        item_id,
        true,
        "Downloading JSON metadata",
    )
}

/// `_build_playlist`: a flat `AudiusTrack` entry per track.
fn build_playlist(tracks: &Value) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    for track in tracks.as_array().into_iter().flatten() {
        let Some(object) = track.as_object() else {
            return Err(Error::Reported(format!(
                "[{NAME}] Unexpected API response; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let id = match object.get("id") {
            Some(Value::String(id)) if !id.is_empty() => id.clone(),
            Some(Value::Number(number)) => number.to_string(),
            _ => {
                return Err(Error::Reported(format!(
                    "[{NAME}] Unable to get track ID from playlist; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        entries.push(nrk::url_entry(
            &format!("audius:{id}"),
            TRACK_IE_KEY,
            Some(&id),
            None,
            None,
        )?);
    }
    Ok(entries)
}

/// The shared `_real_extract` of the track classes.
fn track_info(
    client: &Client,
    url: &Url,
    track_id: Option<&str>,
    title_text: Option<&str>,
) -> Result<VideoInfo> {
    let base = select_api_base(client)?;
    let (title, track_data) = match track_id {
        None => {
            let title = title_text.unwrap_or_default();
            let (prepared, title) = prepare_url(url.as_str(), title);
            let data = resolve(client, &base, &prepared, &title)?;
            (Some(title), data)
        }
        Some(track_id) => (
            None,
            api_request(
                client,
                &base,
                &format!("/tracks/{track_id}"),
                track_id,
                false,
                "Downloading JSON metadata",
            )?,
        ),
    };
    let Some(track) = track_data.as_object() else {
        return Err(Error::Reported(format!(
            "[{NAME}] Unexpected API response; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let id = match track.get("id") {
        Some(Value::Null) | None => {
            return Err(Error::Reported(format!(
                "[{NAME}] Unable to get ID of the track; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        Some(value) => value
            .as_str()
            .map_or_else(|| value.to_string(), str::to_owned),
    };
    let mut thumbnails = Vec::new();
    if let Some(artwork) = track.get("artwork").and_then(Value::as_object) {
        for (quality, thumbnail) in artwork {
            let Some(thumbnail) = thumbnail.as_str().filter(|url| Url::parse(url).is_ok()) else {
                continue;
            };
            let mut entry = Map::new();
            entry.insert("url".into(), thumbnail.into());
            if let Some(preference) = match quality.as_str() {
                "150x150" => Some(150),
                "480x480" => Some(480),
                "1000x1000" => Some(1000),
                _ => None,
            } {
                entry.insert("preference".into(), preference.into());
            }
            thumbnails.push(Value::Object(entry));
        }
    }
    let field = |key: &str| track.get(key).cloned().unwrap_or(Value::Null);
    let title_value = match &title {
        Some(title) if !track.contains_key("title") => json!(title),
        _ => field("title"),
    };
    let artist = track
        .get("user")
        .and_then(|user| user.get("name"))
        .and_then(Value::as_str)
        .map_or(Value::Null, Value::from);
    let fields: Vec<(&str, Value)> = vec![
        ("id", id.clone().into()),
        ("title", title_value.clone()),
        (
            "url",
            format!("{base}{API_VERSION}/tracks/{id}/stream").into(),
        ),
        ("ext", "mp3".into()),
        ("description", field("description")),
        ("duration", field("duration")),
        ("track", title_value),
        ("artist", artist),
        ("genre", field("genre")),
        ("thumbnails", Value::Array(thumbnails)),
        ("view_count", field("play_count")),
        ("like_count", field("favorite_count")),
        ("repost_count", field("repost_count")),
    ];
    // Upstream's bare `url` result: the stream URL is the one format.
    let stream = Url::parse(&format!("{base}{API_VERSION}/tracks/{id}/stream"))?;
    Ok(url_info(url, &stream, fields))
}

/// Upstream's bare `url` result: the stream URL is the one `mp3` format, and
/// the dict's keys keep `fields`' order with no `formats` key.
fn url_info(page: &Url, media: &Url, fields: Vec<(&str, Value)>) -> VideoInfo {
    let keys: Vec<String> = fields.iter().map(|(key, _)| (*key).to_owned()).collect();
    let mut formats =
        direct::with_format(direct::build_at(media.clone(), media, "mp3".into()), "").formats;
    for format in &mut formats {
        format.ext = "mp3".into();
        format.format_id = String::new();
    }
    let mut info = super::dict_info(page, (NAME, NAME), fields, formats);
    info.format_id = None;
    crate::info_order::mark_info(info, &format!("untyped-dict;{}", keys.join(",")))
}

impl Extractor for Audius {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some((title, _)) = track_match(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        track_info(client, url, None, Some(&title)).map(Some)
    }
}

impl Extractor for AudiusTrack {
    fn key(&self) -> &'static str {
        TRACK_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = TRACK_ID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["track_id"].to_owned();
        crate::screen::set_extractor(TRACK_NAME);
        track_info(client, url, Some(&id), None).map(Some)
    }
}

/// `_build_playlist` plus `playlist_result(entries, id, title, description)`.
fn playlist_result(
    url: &Url,
    (extractor, key): (&'static str, &'static str),
    id: String,
    title: String,
    description: Option<Value>,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut extra = Map::new();
    let mut keys = vec!["id"];
    if !title.is_empty() {
        keys.push("title");
    }
    if let Some(description) = description.filter(|value| !value.is_null()) {
        keys.push("description");
        extra.insert("description".into(), description);
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title,
        webpage_url: url.clone(),
        extractor: extractor.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

impl Extractor for AudiusPlaylist {
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
        let title_text = found["title"].to_owned();
        crate::screen::set_extractor(PLAYLIST_NAME);
        let base = select_api_base(client)?;
        let (prepared, title) = prepare_url(url.as_str(), &title_text);
        let response = resolve(client, &base, &prepared, &title)?;
        let playlist = match response.as_array().map(Vec::as_slice) {
            Some([playlist]) if playlist.is_object() => playlist.clone(),
            _ => {
                return Err(Error::Reported(format!(
                    "[{NAME}] Unexpected API response; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let id = playlist
            .get("id")
            .filter(|id| !id.is_null())
            .map(|id| id.as_str().map_or_else(|| id.to_string(), str::to_owned))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] Unable to get playlist ID; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let tracks = api_request(
            client,
            &base,
            &format!("/playlists/{id}/tracks"),
            &title,
            false,
            "Downloading playlist tracks metadata",
        )?;
        if !tracks.is_array() {
            return Err(Error::Reported(format!(
                "[{NAME}] Unexpected API response; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let entries = build_playlist(&tracks)?;
        let name = playlist.get("playlist_name").map_or(title.clone(), |name| {
            name.as_str().unwrap_or_default().to_owned()
        });
        Ok(Some(playlist_result(
            url,
            (PLAYLIST_NAME, PLAYLIST_IE_KEY),
            id,
            name,
            playlist.get("description").cloned(),
            entries,
        )))
    }
}

impl Extractor for AudiusProfile {
    fn key(&self) -> &'static str {
        PROFILE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PROFILE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let profile_id = found["id"].to_owned();
        crate::screen::set_extractor(PROFILE_NAME);
        let base = select_api_base(client)?;
        let profile = api_request(
            client,
            &base,
            &format!("/full/users/handle/{profile_id}"),
            &profile_id,
            false,
            "Downloading JSON metadata",
        )
        .map_err(|error| {
            Error::Reported(format!("[{NAME}] Could not download profile info; {error}"))
        })?;
        let first = profile
            .as_array()
            .and_then(|items| items.first())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] Unexpected API response; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let audius_id = first
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] Unexpected API response; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let bio = first.get("bio").filter(|bio| !bio.is_null()).cloned();
        let tracks = api_request(
            client,
            &base,
            &format!("/full/users/handle/{profile_id}/tracks"),
            &profile_id,
            false,
            "Downloading JSON metadata",
        )?;
        let entries = build_playlist(&tracks)?;
        Ok(Some(playlist_result(
            url,
            (PROFILE_NAME, PROFILE_IE_KEY),
            audius_id,
            profile_id,
            bio,
            entries,
        )))
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    track_match(url).is_some()
}

/// URL matching for `--list-extractors`.
pub fn track_suitable(url: &str) -> bool {
    TRACK_ID_URL.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn profile_suitable(url: &str) -> bool {
    PROFILE_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(track_match("https://audius.co/voltra/radar-103692").is_some());
        assert!(track_match("https://audius.co/test_acc/playlist/test-playlist-22910").is_none());
        assert!(TRACK_ID_URL.is_match("audius:9RWlo"));
        assert!(PLAYLIST_URL.is_match("https://audius.co/test_acc/album/x"));
        assert!(PROFILE_URL.is_match("https://audius.co/pzl/"));
    }

    #[test]
    fn titles_with_slashes_are_requoted_like_upstream() {
        let (url, _) = prepare_url("https://audius.co/test_acc/x%2Fy", "x%2Fy");
        assert_eq!(url, "https://audius.co/test_acc/x%5Cy");
        assert_eq!(quote("a^b`c"), "a%5Eb%60c");
    }
}
