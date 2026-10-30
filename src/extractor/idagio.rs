//! Upstream `IdagioTrackIE`, `IdagioRecordingIE`, `IdagioAlbumIE`,
//! `IdagioPlaylistIE` and `IdagioPersonalPlaylistIE`
//! (`yt_dlp/extractor/idagio.py`): a track's content URL with its metadata,
//! and recordings, albums and playlists as playlists of track URL results.
use super::{Extractor, direct, download_json, streamable::int_or_none};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct IdagioTrack;
pub struct IdagioRecording;
pub struct IdagioAlbum;
pub struct IdagioPlaylist;
pub struct IdagioPersonalPlaylist;

const TRACK_NAME: &str = "IdagioTrack";
const PREFIX: &str = r"^https?://(?:www\.)?app\.idagio\.com(?:/[a-z]{2})?";

static TRACK_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{PREFIX}/recordings/\d+\?(?:[^#]+&)?trackId=(?P<id>\d+)"
    ))
    .unwrap()
});
static RECORDING_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{PREFIX}/recordings/(?P<id>\d+)")).unwrap());
static TRACK_ID_QUERY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[&?]trackId=\d+").unwrap());
static ALBUM_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{PREFIX}/albums/(?P<id>[\w-]+)")).unwrap());
static PLAYLIST_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{PREFIX}/playlists/(?P<id>[\w-]+)")).unwrap());
static PERSONAL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"{PREFIX}/playlists/personal/(?P<id>[\da-f-]+)")).unwrap()
});

/// `_match_valid_url` of [`IdagioTrack`].
pub fn track_suitable(url: &str) -> bool {
    TRACK_URL.is_match(url)
}

/// `_match_valid_url` of [`IdagioRecording`]: a recording without a
/// `trackId` query parameter (upstream's negative lookahead).
pub fn recording_suitable(url: &str) -> bool {
    let Some(found) = RECORDING_URL.find(url) else {
        return false;
    };
    let rest = &url[found.end()..];
    let before_fragment = rest.split('#').next().unwrap_or_default();
    !TRACK_ID_QUERY.is_match(before_fragment)
}

/// `_match_valid_url` of [`IdagioAlbum`].
pub fn album_suitable(url: &str) -> bool {
    ALBUM_URL.is_match(url)
}

/// `_match_valid_url` of [`IdagioPlaylist`] (not a personal playlist).
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url) && !url.contains("/playlists/personal/")
}

/// `_match_valid_url` of [`IdagioPersonalPlaylist`].
pub fn personal_suitable(url: &str) -> bool {
    PERSONAL_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> crate::Error {
    super::clyp::key_error(id, key)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `traverse_obj` of a `str` value (`None` unless a non-empty string).
fn text(value: Option<&Value>) -> Option<Value> {
    value.and_then(Value::as_str).map(Value::from)
}

/// A list of the `name` (or `title`) strings of the given objects, empty
/// names filtered out.
fn names(values: &[&Value], key: &str) -> Vec<Value> {
    values
        .iter()
        .filter_map(|value| value.get(key).and_then(Value::as_str))
        .filter(|name| !name.is_empty())
        .map(Value::from)
        .collect()
}

/// The `artists` of a recording: conductor, ensembles, then soloists.
fn artists(recording: &Value) -> Vec<Value> {
    let mut found = Vec::new();
    if let Some(conductor) = recording.get("conductor").filter(|value| value.is_object()) {
        found.push(conductor);
    }
    for key in ["ensembles", "soloists"] {
        if let Some(list) = recording.get(key).and_then(Value::as_array) {
            found.extend(list.iter().filter(|value| value.is_object()));
        }
    }
    names(&found, "name")
}

/// `genres`: the work's genre and subgenre titles.
fn genres(work: Option<&Value>) -> Vec<Value> {
    let Some(work) = work else {
        return Vec::new();
    };
    ["genre", "subgenre"]
        .iter()
        .filter_map(|key| work.get(*key).filter(|value| value.is_object()))
        .filter_map(|value| value.get("title").and_then(Value::as_str))
        .filter(|title| !title.is_empty())
        .map(Value::from)
        .collect()
}

/// `IdagioTrackIE._real_extract`: the track's MP3 with its metadata.
fn track_info(client: &Client, url: &Url, track_id: &str) -> Result<VideoInfo> {
    let metadata_url = Url::parse(&format!(
        "https://api.idagio.com/v2.0/metadata/tracks/{track_id}"
    ))?;
    // `fatal=False, expected_status=406`: the error code's body is read.
    crate::screen::screen(&format!(
        "[{TRACK_NAME}] {track_id}: Downloading JSON metadata"
    ));
    let track = direct::request_expecting(client, &metadata_url, &[], &[406])
        .and_then(|response| Ok(crate::json::from_str::<Value>(&response.text()?).ok()))
        .ok()
        .flatten()
        .unwrap_or(Value::Null);
    if track.get("error_code").and_then(Value::as_str) == Some("idagio.error.blocked.location") {
        return Err(crate::geo::restricted(
            &format!(
                "[{TRACK_NAME}] {track_id}: This video is not available from your location due to geo restriction"
            ),
            &[],
        ));
    }

    let content = download_json(
        client,
        &Url::parse(&format!(
            "https://api.idagio.com/v1.8/content/track/{track_id}?quality=0&format=2&client_type=web-4"
        ))?,
        TRACK_NAME,
        track_id,
    )?;
    let media = content
        .get("url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .and_then(|url| Url::parse(&url).ok());

    let result = track.get("result").cloned().unwrap_or(Value::Null);
    let recording = result.get("recording");
    let piece = result.get("piece");
    let work = piece
        .and_then(|piece| piece.get("workpart"))
        .and_then(|part| part.get("work"));

    let mut fields: Vec<(&str, Value)> = vec![
        ("ext", "mp3".into()),
        ("vcodec", "none".into()),
        ("id", track_id.into()),
        (
            "url",
            media
                .as_ref()
                .map_or(Value::Null, |url| url.as_str().into()),
        ),
    ];
    let timestamp = int_or_none(recording.and_then(|recording| recording.get("created_at")))
        .as_i64()
        .map_or(Value::Null, |millis| (millis / 1000).into());
    let duration = int_or_none(result.get("duration"));
    let composers = work
        .and_then(|work| work.get("composer"))
        .map(|composer| names(&[composer], "name"))
        .unwrap_or_default();
    let optional = |value: Option<Value>| value.unwrap_or(Value::Null);
    for (key, value) in [
        (
            "title",
            optional(text(piece.and_then(|piece| piece.get("title")))),
        ),
        ("timestamp", timestamp),
        (
            "location",
            optional(text(recording.and_then(|r| r.get("location")))),
        ),
        ("duration", duration),
        (
            "track",
            optional(text(piece.and_then(|piece| piece.get("title")))),
        ),
    ] {
        if !value.is_null() {
            fields.push((key, value));
        }
    }
    let artists = recording.map(artists).unwrap_or_default();
    if !artists.is_empty() {
        fields.push(("artists", Value::Array(artists)));
    }
    if !composers.is_empty() {
        fields.push(("composers", Value::Array(composers)));
    }
    let genres = genres(work);
    if !genres.is_empty() {
        fields.push(("genres", Value::Array(genres)));
    }

    let media = media.unwrap_or_else(|| url.clone());
    Ok(direct_info(
        url,
        &media,
        "mp3",
        (TRACK_NAME, TRACK_NAME),
        fields,
    ))
}

/// A direct-media info (`url` as its one format) whose dict keys follow
/// `fields`, with `None` keys left out.
fn direct_info(
    page: &Url,
    media: &Url,
    ext: &str,
    (extractor, key): (&str, &str),
    fields: Vec<(&str, Value)>,
) -> VideoInfo {
    let mut info = direct::with_format(direct::build_at(media.clone(), media, ext.to_owned()), "");
    for format in &mut info.formats {
        format.ext = ext.to_owned();
        format.format_id = String::new();
        format.vcodec = Some("none".into());
    }
    info.format_id = None;
    info.webpage_url = page.clone();
    info.direct = None;
    info.extractor = extractor.to_owned();
    info.extractor_key = key.to_owned();
    let mut keys = Vec::new();
    for (name, value) in fields {
        keys.push(name.to_owned());
        match name {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            _ => crate::metadata_parser::set_field(&mut info, name, value),
        }
    }
    crate::info_order::mark_info(info, &format!("untyped-dict;{}", keys.join(",")))
}

/// A track's `url` playlist entry (`url_result(..., ie=IdagioTrackIE, video_id=...)`).
fn track_entry(recording_id: &str, track_id: &str) -> Result<VideoInfo> {
    let target = format!("https://app.idagio.com/recordings/{recording_id}?trackId={track_id}");
    let url = super::target_url(&target)?;
    let mut info = direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = track_id.to_owned();
    let mut entry = Map::new();
    entry.insert("ie_key".into(), TRACK_NAME.into());
    entry.insert("id".into(), track_id.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.clone().into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": TRACK_NAME,
            "extractor": "IdagioTrack",
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `_entries(playlist_info)`: a track entry for each track with a recording.
fn entries(playlist: &Value) -> Result<Vec<VideoInfo>> {
    let mut found = Vec::new();
    for track in playlist
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let track_id = match track.get("id") {
            Some(Value::String(text)) if !text.is_empty() => text.clone(),
            Some(Value::Number(number)) => number.to_string(),
            _ => continue,
        };
        let Some(recording_id) = track
            .get("recording")
            .and_then(|recording| recording.get("id"))
            .and_then(|id| match id {
                Value::String(text) if !text.is_empty() => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
        else {
            continue;
        };
        found.push(track_entry(&recording_id, &track_id)?);
    }
    Ok(found)
}

/// `IdagioPlaylistBaseIE._real_extract`: the API's `result` as a playlist
/// whose entries are its track URLs, keyed by the URL's ID (`key` is `id` or
/// `display_id`), then the metadata keys.
fn playlist(
    client: &Client,
    url: &Url,
    (name, id_key): (&'static str, &'static str),
    api: &str,
    playlist_id: &str,
    metadata: fn(&Value) -> Vec<(&'static str, Value)>,
) -> Result<ExtractResult> {
    announce(name, url);
    let result = download_json(
        client,
        &Url::parse(&api.replace("{}", playlist_id))?,
        name,
        playlist_id,
    )?
    .get("result")
    .cloned()
    .ok_or_else(|| key_error(playlist_id, "result"))?;
    let entries = entries(&result)?;
    let fields = metadata(&result);
    let mut title = String::new();
    let mut id = None;
    let mut extra = Map::new();
    // `{'_type': 'playlist', id_key, 'entries', **metadata}`.
    let mut keys = vec!["_type".to_owned(), id_key.to_owned(), "entries".to_owned()];
    for (key, value) in fields {
        keys.push(key.to_owned());
        match key {
            "title" => title = value.as_str().unwrap_or_default().to_owned(),
            "id" => id = value.as_str().map(str::to_owned),
            _ => {
                extra.insert(key.to_owned(), value);
            }
        }
    }
    if id_key == "display_id" {
        extra.insert("display_id".into(), playlist_id.into());
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("youtube-tab-plain;{}", keys.join(",")).into(),
    );
    Ok(ExtractResult::Playlist(PlaylistInfo {
        id: id.unwrap_or_else(|| playlist_id.to_owned()),
        title,
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: name.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    }))
}

fn recording_metadata(result: &Value) -> Vec<(&'static str, Value)> {
    let work = result.get("work");
    let mut fields = Vec::new();
    if let Some(title) = text(work.and_then(|work| work.get("title"))) {
        fields.push(("title", title));
    }
    if let Some(created) = int_or_none(result.get("created_at")).as_i64() {
        fields.push(("timestamp", (created / 1000).into()));
        fields.push(("modified_timestamp", (created / 1000).into()));
    }
    if let Some(location) = text(result.get("location")) {
        fields.push(("location", location));
    }
    let artists = artists(result);
    if !artists.is_empty() {
        fields.push(("artists", Value::Array(artists)));
    }
    let composers: Vec<Value> = work
        .and_then(|work| work.get("composer"))
        .map(|composer| names(&[composer], "name"))
        .unwrap_or_default();
    fields.push(("composers", Value::Array(composers)));
    let genres = genres(work);
    if !genres.is_empty() {
        fields.push(("genres", Value::Array(genres)));
    }
    let tags = string_list(result.get("tags"));
    if !tags.is_empty() {
        fields.push(("tags", Value::Array(tags)));
    }
    fields
}

/// `[..., {str}]`: the string items of a list.
fn string_list(value: Option<&Value>) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(Value::from))
        .collect()
}

fn album_metadata(result: &Value) -> Vec<(&'static str, Value)> {
    let mut fields = Vec::new();
    if let Some(id) = text(result.get("id")) {
        fields.push(("id", id));
    }
    if let Some(title) = text(result.get("title")) {
        fields.push(("title", title));
    }
    let published = text(result.get("publishDate"))
        .and_then(|date| crate::dates::unified_timestamp(date.as_str().unwrap_or_default(), false));
    if let Some(published) = published {
        fields.push(("timestamp", published.into()));
    }
    let modified = text(result.get("lastModified"))
        .and_then(|date| crate::dates::unified_timestamp(date.as_str().unwrap_or_default(), false));
    if let Some(modified) = modified {
        fields.push(("modified_timestamp", modified.into()));
    }
    if let Some(thumbnail) = text(result.get("imageUrl")).and_then(|url| {
        url.as_str()
            .and_then(super::json_ld::url_or_none)
            .map(Value::from)
    }) {
        fields.push(("thumbnail", thumbnail));
    }
    if let Some(description) = text(result.get("description")) {
        fields.push(("description", description));
    }
    let participants: Vec<&Value> = result
        .get("participants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let artists = names(&participants, "name");
    if !artists.is_empty() {
        fields.push(("artists", Value::Array(artists)));
    }
    let tags = string_list(result.get("tags"));
    if !tags.is_empty() {
        fields.push(("tags", Value::Array(tags)));
    }
    fields
}

fn playlist_metadata(result: &Value) -> Vec<(&'static str, Value)> {
    let mut fields = Vec::new();
    if let Some(id) = text(result.get("id")) {
        fields.push(("id", id));
    }
    if let Some(title) = text(result.get("title")) {
        fields.push(("title", title));
    }
    if let Some(thumbnail) = text(result.get("imageUrl")).and_then(|url| {
        url.as_str()
            .and_then(super::json_ld::url_or_none)
            .map(Value::from)
    }) {
        fields.push(("thumbnail", thumbnail));
    }
    if let Some(description) = text(result.get("description")) {
        fields.push(("description", description));
    }
    let curator: Vec<&Value> = result.get("curator").into_iter().collect();
    let creators = names(&curator, "name");
    if !creators.is_empty() {
        fields.push(("creators", Value::Array(creators)));
    }
    fields
}

fn personal_metadata(result: &Value) -> Vec<(&'static str, Value)> {
    let mut fields = Vec::new();
    if let Some(title) = text(result.get("title")) {
        fields.push(("title", title));
    }
    if let Some(thumbnail) = text(result.get("image_url")).and_then(|url| {
        url.as_str()
            .and_then(super::json_ld::url_or_none)
            .map(Value::from)
    }) {
        fields.push(("thumbnail", thumbnail));
    }
    if let Some(user) = text(result.get("user_id")) {
        fields.push(("creators", Value::Array(vec![user])));
    }
    if let Some(created) = int_or_none(result.get("created_at")).as_i64() {
        fields.push(("timestamp", (created / 1000).into()));
    }
    if let Some(updated) = int_or_none(result.get("updated_at")).as_i64() {
        fields.push(("modified_timestamp", (updated / 1000).into()));
    }
    fields
}

impl Extractor for IdagioTrack {
    fn key(&self) -> &'static str {
        TRACK_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = TRACK_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let track_id = found["id"].to_owned();
        announce(TRACK_NAME, url);
        Ok(Some(ExtractResult::Video(track_info(
            client, url, &track_id,
        )?)))
    }
}

impl Extractor for IdagioRecording {
    fn key(&self) -> &'static str {
        "IdagioRecording"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !recording_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = RECORDING_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        Ok(Some(playlist(
            client,
            url,
            ("IdagioRecording", "id"),
            "https://api.idagio.com/v2.0/metadata/recordings/{}",
            &id,
            recording_metadata,
        )?))
    }
}

impl Extractor for IdagioAlbum {
    fn key(&self) -> &'static str {
        "IdagioAlbum"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ALBUM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        Ok(Some(playlist(
            client,
            url,
            ("IdagioAlbum", "display_id"),
            "https://api.idagio.com/v2.0/metadata/albums/{}",
            &id,
            album_metadata,
        )?))
    }
}

impl Extractor for IdagioPlaylist {
    fn key(&self) -> &'static str {
        "IdagioPlaylist"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !playlist_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        Ok(Some(playlist(
            client,
            url,
            ("IdagioPlaylist", "display_id"),
            "https://api.idagio.com/v2.0/playlists/{}",
            &id,
            playlist_metadata,
        )?))
    }
}

impl Extractor for IdagioPersonalPlaylist {
    fn key(&self) -> &'static str {
        "IdagioPersonalPlaylist"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PERSONAL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        Ok(Some(playlist(
            client,
            url,
            ("IdagioPersonalPlaylist", "id"),
            "https://api.idagio.com/v1.0/personal-playlists/{}",
            &id,
            personal_metadata,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(track_suitable(
            "https://app.idagio.com/recordings/30576934?trackId=30576943"
        ));
        assert!(recording_suitable(
            "https://app.idagio.com/de/recordings/20514467"
        ));
        assert!(!recording_suitable(
            "https://app.idagio.com/recordings/30576934?trackId=30576943"
        ));
        assert!(album_suitable(
            "https://app.idagio.com/albums/elgar-enigma-variations-in-the-south-serenade-for-strings"
        ));
        assert!(playlist_suitable(
            "https://app.idagio.com/playlists/beethoven-the-most-beautiful-piano-music"
        ));
        assert!(!playlist_suitable(
            "https://app.idagio.com/playlists/personal/99dad72e-7b3a-45a4-b216-867c08046ed8"
        ));
        assert!(personal_suitable(
            "https://app.idagio.com/playlists/personal/99dad72e-7b3a-45a4-b216-867c08046ed8"
        ));
    }
}
