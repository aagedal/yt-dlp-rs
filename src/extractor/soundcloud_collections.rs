//! Upstream `SoundcloudSetIE`, `SoundcloudPlaylistIE`, `SoundcloudUserIE`,
//! `SoundcloudUserPermalinkIE`, `SoundcloudTrackStationIE` and
//! `SoundcloudRelatedIE` (`yt_dlp/extractor/soundcloud.py`): API-v2 playlists
//! whose entries are URL results (a set's tracks are `url_transparent` with
//! the album fields), and the paged `collection`s of users, stations and
//! related tracks, read eagerly by following `next_href`.
use super::Extractor;
use super::login::Session;
use super::soundcloud::{API_V2_BASE, Soundcloud, call_api, int, thumbnails};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

const SC: &str = "soundcloud";
/// `SoundcloudIE.ie_key()`.
const TRACK_KEY: &str = "Soundcloud";

pub struct SoundcloudSet;
pub struct SoundcloudPlaylist;
pub struct SoundcloudUser;
pub struct SoundcloudUserPermalink;
pub struct SoundcloudTrackStation;
pub struct SoundcloudRelated;
pub struct SoundcloudSearch;

const SET_NAME: &str = "soundcloud:set";
const PLAYLIST_NAME: &str = "soundcloud:playlist";
const USER_NAME: &str = "soundcloud:user";
const PERMALINK_NAME: &str = "soundcloud:user:permalink";
const STATION_NAME: &str = "soundcloud:trackstation";
const RELATED_NAME: &str = "soundcloud:related";
const SEARCH_NAME: &str = "soundcloud:search";
/// `SoundcloudSearchIE`'s `_MAX_RESULTS_PER_PAGE`.
const MAX_RESULTS_PER_PAGE: usize = 200;

static SET_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|m)\.)?soundcloud\.com/(?P<uploader>[\w\d-]+)/sets/(?P<slug_title>[:\w\d-]+)(?:/(?P<token>[^?/]+))?").unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://api(?:-v2)?\.soundcloud\.com/playlists/(?:soundcloud(?:%3A|:)playlists(?:%3A|:))?(?P<id>[0-9]+)(?:/?\?secret_token=(?P<token>[^&]+?))?$").unwrap()
});
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|m)\.)?soundcloud\.com/(?P<user>[^/]+)(?:/(?P<rsrc>tracks|albums|sets|reposts|likes|spotlight|comments))?/?(?:[?#].*)?$").unwrap()
});
static PERMALINK_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://api\.soundcloud\.com/users/(?P<id>\d+)").unwrap());
static STATION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|m)\.)?soundcloud\.com/stations/track/[^/]+/(?P<id>[^/?#&]+)")
        .unwrap()
});
static RELATED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|m)\.)?soundcloud\.com/(?P<slug>[\w\d-]+/[\w\d-]+)/(?P<relation>albums|sets|recommended)")
        .unwrap()
});
static SEARCH_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^scsearch(?P<prefix>|[1-9][0-9]*|all):(?P<query>[\s\S]+)").unwrap()
});
static TRACK_STATION_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"soundcloud:track-stations:(\d+)").unwrap());
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"]+)"|([^ ]+)"#).unwrap());

/// `SoundcloudSetIE.suitable`.
pub fn set_suitable(url: &str) -> bool {
    SET_URL.is_match(url)
}

/// `SoundcloudPlaylistIE.suitable`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `SoundcloudUserIE.suitable`.
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

/// `SoundcloudUserPermalinkIE.suitable`.
pub fn user_permalink_suitable(url: &str) -> bool {
    PERMALINK_URL.is_match(url)
}

/// `SoundcloudTrackStationIE.suitable`.
pub fn track_station_suitable(url: &str) -> bool {
    STATION_URL.is_match(url)
}

/// `SoundcloudRelatedIE.suitable`.
pub fn related_suitable(url: &str) -> bool {
    RELATED_URL.is_match(url)
}

/// `SoundcloudSearchIE.suitable` (`scsearch[N|all]:QUERY`).
pub fn search_suitable(url: &str) -> bool {
    SEARCH_URL.is_match(url)
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `url_result(target, ie, video_id, video_title)` (or `url_transparent`
/// with `fields`) as a playlist entry. Its `entry` keys keep upstream's
/// order: the fields, `ie_key`, `id`, the title, `_type`, `url`.
fn entry(
    target: &str,
    ie_key: Option<&'static str>,
    transparent: Option<&[(&str, Value)]>,
    id: Option<&str>,
    title: Option<&str>,
) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    if let Some(id) = id {
        info.id = id.to_owned();
    }
    let mut fields = Map::new();
    let mut entry = Map::new();
    for (key, value) in transparent.unwrap_or(&[]) {
        fields.insert((*key).to_owned(), value.clone());
        entry.insert((*key).to_owned(), value.clone());
    }
    if let Some(ie_key) = ie_key {
        entry.insert("ie_key".into(), ie_key.into());
    }
    if let Some(id) = id {
        entry.insert("id".into(), id.into());
    }
    if let Some(title) = title {
        entry.insert("title".into(), title.into());
    }
    let kind = if transparent.is_some() {
        "url_transparent"
    } else {
        "url"
    };
    entry.insert("_type".into(), kind.into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": Value::Object(fields),
            "ie_key": ie_key,
            "extractor": ie_key,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `Soundcloud.initialize` for the subclasses: the login hooks are the base
/// class's (`_NETRC_MACHINE`, `_perform_login`, `_real_initialize`).
macro_rules! login_hooks {
    () => {
        fn netrc_machine(&self) -> Option<&'static str> {
            Some(SC)
        }

        fn performs_login(&self) -> bool {
            true
        }

        fn initialize_pre_login(&self, client: &Client, session: &Session) -> Result<()> {
            Soundcloud.initialize_pre_login(client, session)
        }

        fn perform_login(
            &self,
            client: &Client,
            session: &Session,
            username: &str,
            password: &str,
        ) -> Result<()> {
            Soundcloud.perform_login(client, session, username, password)
        }

        fn real_initialize(&self, client: &Client, session: &Session) -> Result<()> {
            Soundcloud.real_initialize(client, session)
        }
    };
}

/// `_call_api(url, id, note, query)` of a paged `collection` (`limit`,
/// `linked_partitioning`, then `offset` on the first page only), following
/// `next_href` and yielding each item's playable URL.
fn paged_entries(
    client: &Client,
    name: &'static str,
    url: &str,
    id: &str,
) -> Result<Vec<VideoInfo>> {
    // `impersonate=self._browser_impersonate_target`: without a target,
    // upstream warns once and sends the requests plain.
    let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
        crate::screen::warning_once(&format!("[{name}] {message}"));
    });
    let mut entries = Vec::new();
    let mut next = url.to_owned();
    let mut query: Vec<(&str, &str)> = vec![
        ("limit", "200"),
        ("linked_partitioning", "1"),
        ("offset", "0"),
    ];
    let mut page = 1;
    loop {
        let response = call_api(
            client,
            &next,
            id,
            &format!("Downloading track page {page}"),
            &query,
        )?;
        for item in response
            .get("collection")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(resolved) = resolve_entry(item)? {
                entries.push(resolved);
            }
        }
        let Some(href) = response
            .get("next_href")
            .and_then(Value::as_str)
            .filter(|href| !href.is_empty())
        else {
            break;
        };
        next = href.to_owned();
        query.retain(|(key, _)| *key != "offset");
        page += 1;
    }
    Ok(entries)
}

/// `SoundcloudSearchIE._get_collection('search/tracks', query, limit=n)`:
/// each track is a flat `url` result carrying its info dict (`formats` is
/// `None`), until `n` results.
fn search_entries(client: &Client, query: &str, n: Option<usize>) -> Result<Vec<VideoInfo>> {
    let limit = n.map_or(MAX_RESULTS_PER_PAGE, |n| n.min(MAX_RESULTS_PER_PAGE));
    let limit = limit.to_string();
    let mut entries = Vec::new();
    let mut next = format!("{API_V2_BASE}search/tracks");
    let mut query_pairs: Vec<(&str, &str)> = vec![
        ("limit", &limit),
        ("q", query),
        ("linked_partitioning", "1"),
        ("offset", "0"),
    ];
    let mut page = 1;
    loop {
        let response = call_api(
            client,
            &next,
            query,
            &format!("Downloading page {page}"),
            &query_pairs,
        )?;
        for item in response
            .get("collection")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.as_object().is_some_and(|object| !object.is_empty()))
        {
            let Some(uri) = item.get("uri").and_then(Value::as_str) else {
                continue;
            };
            let track_id = str_or_none(item.get("id")).unwrap_or_default();
            let mut fields = super::soundcloud::track_dict(client, item, &track_id);
            fields.insert("formats".into(), Value::Null);
            entries.push(flat_entry(uri, fields)?);
            if n.is_some_and(|n| entries.len() >= n) {
                return Ok(entries);
            }
        }
        let Some(href) = response
            .get("next_href")
            .and_then(Value::as_str)
            .filter(|href| !href.is_empty())
        else {
            break;
        };
        next = href.to_owned();
        query_pairs.clear();
        page += 1;
    }
    Ok(entries)
}

/// A flat `url` result whose entry carries the info dict `fields`, then
/// `ie_key`, `_type` and `url`.
fn flat_entry(target: &str, fields: Map<String, Value>) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    if let Some(id) = fields.get("id").and_then(Value::as_str) {
        info.id = id.to_owned();
    }
    let mut entry = fields;
    entry.insert("ie_key".into(), TRACK_KEY.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": TRACK_KEY,
            "extractor": TRACK_KEY,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `resolve_entry(*candidates)`: the first candidate (the item, its `track`
/// or its `playlist`) with a `permalink_url`.
fn resolve_entry(item: &Value) -> Result<Option<VideoInfo>> {
    for candidate in [Some(item), item.get("track"), item.get("playlist")]
        .into_iter()
        .flatten()
    {
        if !candidate.is_object() {
            continue;
        }
        let Some(permalink) = candidate
            .get("permalink_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let id = str_or_none(candidate.get("id"));
        let title = candidate.get("title").and_then(Value::as_str);
        // `SoundcloudIE.ie_key() if SoundcloudIE.suitable(permalink_url)`.
        let ie_key = super::soundcloud::suitable(&permalink).then_some(TRACK_KEY);
        return entry(&permalink, ie_key, None, id.as_deref(), title).map(Some);
    }
    Ok(None)
}

/// `playlist_result(entries, id, title, description, **kwargs)`. The keys
/// keep upstream's order through the `playlist-dict` shape: the kwargs
/// present, then `id`, `title` and `description` (a set or playlist), or
/// `_type`, `id` and `title` (the paged collections, which are generators;
/// a search's results are a list, `id` and `title` only).
fn playlist(
    url: &Url,
    (name, key): (&'static str, &'static str),
    (id, title): (String, Option<String>),
    description: Option<Value>,
    fields: Vec<(String, Value)>,
    entries: Vec<VideoInfo>,
    generator: bool,
) -> ExtractResult {
    let mut extra = Map::new();
    let mut keys: Vec<String> = Vec::new();
    for (field, value) in fields {
        if !value.is_null() {
            keys.push(field.clone());
            extra.insert(field, value);
        }
    }
    let shape = match description {
        Some(description) => {
            extra.insert("description".into(), description);
            keys.extend(["id", "title", "description"].map(str::to_owned));
            format!("playlist-dict;{}", keys.join(","))
        }
        None if generator => "playlist-dict;_type,id,title".to_owned(),
        None => "playlist-dict;id,title".to_owned(),
    };
    let mut shape = shape;
    if generator {
        shape.push_str(crate::info_order::GENERATOR);
    }
    extra.insert(crate::info_order::INFO_SHAPE.into(), shape.into());
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

/// `_extract_playlist(base_url, playlist_id, playlist_title)`.
fn paged_playlist(
    client: &Client,
    url: &Url,
    (name, key): (&'static str, &'static str),
    api_url: &str,
    playlist_id: String,
    title: String,
) -> Result<ExtractResult> {
    let entries = paged_entries(client, name, api_url, &playlist_id)?;
    Ok(playlist(
        url,
        (name, key),
        (playlist_id, Some(title)),
        None,
        Vec::new(),
        entries,
        true,
    ))
}

/// `_extract_set(playlist, token)`.
fn set(
    client: &Client,
    url: &Url,
    (name, key): (&'static str, &'static str),
    info: &Value,
    token: Option<&str>,
) -> Result<ExtractResult> {
    let playlist_id = str_or_none(info.get("id")).unwrap_or_default();
    let mut tracks: Vec<Value> = info
        .get("tracks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let all_permalinks = tracks
        .iter()
        .all(|track| track.get("permalink_url").and_then(Value::as_str).is_some());
    if !all_permalinks && let Some(token) = token {
        let ids: Vec<String> = tracks
            .iter()
            .filter_map(|track| str_or_none(track.get("id")))
            .collect();
        let ids = ids.join(",");
        let fetched = call_api(
            client,
            &format!("{API_V2_BASE}tracks"),
            &playlist_id,
            "Downloading tracks",
            &[
                ("ids", &ids),
                ("playlistId", &playlist_id),
                ("playlistSecretToken", token),
            ],
        )?;
        tracks = fetched.as_array().cloned().unwrap_or_default();
    }
    let text = |key: &str| info.get(key).and_then(Value::as_str).map(str::to_owned);
    let album_title = text("title");
    let album_artist = info
        .pointer("/user/username")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let album_type = text("set_type").filter(|kind| !kind.is_empty());
    let album_type = album_type.unwrap_or_else(|| "playlist".into());
    let mut album: Vec<(&str, Value)> = Vec::new();
    if let Some(title) = &album_title {
        album.push(("album", title.as_str().into()));
    }
    if let Some(artist) = &album_artist {
        album.push(("album_artist", artist.as_str().into()));
    }
    album.push(("album_type", album_type.as_str().into()));

    let mut entries = Vec::new();
    for track in &tracks {
        let track_id = str_or_none(track.get("id"));
        let mut target = track
            .get("permalink_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        if target.is_none() {
            let Some(track_id) = &track_id else {
                continue;
            };
            let mut built = format!("{API_V2_BASE}tracks/{track_id}");
            if let Some(token) = token {
                built.push_str("?secret_token=");
                built.push_str(token);
            }
            target = Some(built);
        }
        if let Some(target) = target {
            entries.push(entry(
                &target,
                Some(TRACK_KEY),
                Some(album.as_slice()),
                track_id.as_deref(),
                None,
            )?);
        }
    }

    let mut fields: Vec<(String, Value)> = album
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect();
    let user = |key: &str| info.pointer(&format!("/user/{key}"));
    fields.push((
        "uploader".into(),
        user("username")
            .and_then(Value::as_str)
            .map_or(Value::Null, Value::from),
    ));
    fields.push((
        "uploader_id".into(),
        str_or_none(user("id")).map_or(Value::Null, Value::from),
    ));
    fields.push((
        "uploader_url".into(),
        user("permalink_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map_or(Value::Null, Value::from),
    ));
    let timestamp = |date: &str| crate::dates::unified_timestamp(date, false);
    fields.push((
        "timestamp".into(),
        text("created_at")
            .and_then(|date| timestamp(&date))
            .map_or(Value::Null, Value::from),
    ));
    let release = ["release_date", "published_at"]
        .iter()
        .find_map(|key| text(key).and_then(|date| timestamp(&date)));
    fields.push((
        "release_timestamp".into(),
        release.map_or(Value::Null, Value::from),
    ));
    fields.push((
        "modified_timestamp".into(),
        text("last_modified")
            .and_then(|date| timestamp(&date))
            .map_or(Value::Null, Value::from),
    ));
    fields.push((
        "duration".into(),
        info.get("duration")
            .and_then(Value::as_f64)
            .map_or(Value::Null, |millis| json!(millis / 1000.0)),
    ));
    fields.push((
        "license".into(),
        text("license").map_or(Value::Null, Value::from),
    ));
    fields.push((
        "like_count".into(),
        int(info.get("likes_count")).map_or(Value::Null, Value::from),
    ));
    fields.push((
        "repost_count".into(),
        int(info.get("reposts_count")).map_or(Value::Null, Value::from),
    ));
    let genre = text("genre").filter(|genre| !genre.is_empty());
    fields.push((
        "genres".into(),
        genre.map_or(Value::Null, |genre| json!([genre])),
    ));
    let tags = text("tag_list").map(|tags| {
        TAGS.captures_iter(&tags)
            .flat_map(|found| {
                [found.get(1), found.get(2)]
                    .into_iter()
                    .flatten()
                    .map(|tag| tag.as_str().to_owned())
                    .collect::<Vec<_>>()
            })
            .filter(|tag| !tag.is_empty())
            .map(Value::from)
            .collect::<Vec<Value>>()
    });
    fields.push((
        "tags".into(),
        tags.filter(|tags| !tags.is_empty())
            .map_or(Value::Null, Value::Array),
    ));
    let thumbnails = thumbnails(client, info, &playlist_id);
    fields.push(("thumbnails".into(), thumbnails));

    Ok(playlist(
        url,
        (name, key),
        (playlist_id, album_title.clone()),
        Some(text("description").map_or(Value::Null, Value::from)),
        fields,
        entries,
        false,
    ))
}

/// `_resolv_url(url)` through `call_api`.
fn resolve(client: &Client, url: &str, id: &str) -> Result<Value> {
    call_api(
        client,
        &format!("{API_V2_BASE}resolve?url={url}"),
        id,
        "Downloading info JSON",
        &[],
    )
}

/// `str.capitalize()`.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.as_str().to_lowercase().chars())
            .collect(),
        None => String::new(),
    }
}

/// Runs an `extract_result` body with the common preamble: the extractor
/// screen and `_real_initialize`.
fn begin(client: &Client, name: &'static str, url: &Url, extractor: &dyn Extractor) -> Result<()> {
    crate::screen::set_extractor(name);
    extractor.initialize(client, name, "")?;
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    Ok(())
}

impl Extractor for SoundcloudSet {
    fn key(&self) -> &'static str {
        "SoundcloudSet"
    }

    login_hooks!();

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SET_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(client, SET_NAME, url, self)?;
        let mut full_title = format!("{}/sets/{}", &found["uploader"], &found["slug_title"]);
        let token = found.name("token").map(|token| token.as_str().to_owned());
        if let Some(token) = &token {
            full_title.push('/');
            full_title.push_str(token);
        }
        let info = resolve(
            client,
            &format!("https://soundcloud.com/{full_title}"),
            &full_title,
        )?;
        if let Some(errors) = info.get("errors").and_then(Value::as_array) {
            let messages: Vec<String> = errors
                .iter()
                .map(|error| str_or_none(error.get("error_message")).unwrap_or_default())
                .collect();
            return Err(Error::Reported(format!(
                "[{SET_NAME}] unable to download video webpage: {}",
                messages.join(",")
            )));
        }
        set(
            client,
            url,
            (SET_NAME, "SoundcloudSet"),
            &info,
            token.as_deref(),
        )
        .map(Some)
    }
}

impl Extractor for SoundcloudPlaylist {
    fn key(&self) -> &'static str {
        "SoundcloudPlaylist"
    }

    login_hooks!();

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(client, PLAYLIST_NAME, url, self)?;
        let playlist_id = found["id"].to_owned();
        let token = found.name("token").map(|token| token.as_str().to_owned());
        let query: Vec<(&str, &str)> = match &token {
            Some(token) => vec![("secret_token", token.as_str())],
            None => Vec::new(),
        };
        let info = call_api(
            client,
            &format!("{API_V2_BASE}playlists/{playlist_id}"),
            &playlist_id,
            "Downloading playlist",
            &query,
        )?;
        set(
            client,
            url,
            (PLAYLIST_NAME, "SoundcloudPlaylist"),
            &info,
            token.as_deref(),
        )
        .map(Some)
    }
}

impl Extractor for SoundcloudUser {
    fn key(&self) -> &'static str {
        "SoundcloudUser"
    }

    login_hooks!();

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = USER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(client, USER_NAME, url, self)?;
        let uploader = found["user"].to_owned();
        let user = resolve(
            client,
            &format!("https://soundcloud.com/{uploader}"),
            &uploader,
        )?;
        let resource = found.name("rsrc").map_or("all", |rsrc| rsrc.as_str());
        let path = match resource {
            "tracks" => "users/%s/tracks",
            "albums" => "users/%s/albums",
            "sets" => "users/%s/playlists",
            "reposts" => "stream/users/%s/reposts",
            "likes" => "users/%s/likes",
            "spotlight" => "users/%s/spotlight",
            "comments" => "users/%s/comments",
            _ => "stream/users/%s",
        };
        let user_id = str_or_none(user.get("id")).unwrap_or_default();
        let api = format!("{API_V2_BASE}{}", path.replace("%s", &user_id));
        let username = user
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = format!("{username} ({})", capitalize(resource));
        paged_playlist(
            client,
            url,
            (USER_NAME, "SoundcloudUser"),
            &api,
            user_id,
            title,
        )
        .map(Some)
    }
}

impl Extractor for SoundcloudUserPermalink {
    fn key(&self) -> &'static str {
        "SoundcloudUserPermalink"
    }

    login_hooks!();

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PERMALINK_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(client, PERMALINK_NAME, url, self)?;
        let user_id = found["id"].to_owned();
        let user = resolve(client, url.as_str(), &user_id)?;
        let id = str_or_none(user.get("id")).unwrap_or_default();
        let api = format!("{API_V2_BASE}users/{id}/tracks");
        let title = user
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        paged_playlist(
            client,
            url,
            (PERMALINK_NAME, "SoundcloudUserPermalink"),
            &api,
            id,
            title,
        )
        .map(Some)
    }
}

impl Extractor for SoundcloudTrackStation {
    fn key(&self) -> &'static str {
        "SoundcloudTrackStation"
    }

    login_hooks!();

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = STATION_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(client, STATION_NAME, url, self)?;
        let track_name = found["id"].to_owned();
        let track = resolve(client, url.as_str(), &track_name)?;
        let station_id = track.get("id").and_then(Value::as_str).unwrap_or_default();
        let Some(numeric) = TRACK_STATION_ID.captures(station_id) else {
            return Err(Error::Reported(format!(
                "[{STATION_NAME}] {track_name}: Unable to extract track id; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let track_id = numeric[1].to_owned();
        let title = track
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let api = format!("{API_V2_BASE}stations/{station_id}/tracks");
        paged_playlist(
            client,
            url,
            (STATION_NAME, "SoundcloudTrackStation"),
            &api,
            track_id,
            format!("Track station: {title}"),
        )
        .map(Some)
    }
}

impl Extractor for SoundcloudRelated {
    fn key(&self) -> &'static str {
        "SoundcloudRelated"
    }

    login_hooks!();

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = RELATED_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(client, RELATED_NAME, url, self)?;
        let slug = found["slug"].to_owned();
        let relation = found["relation"].to_owned();
        let track = resolve(client, &format!("https://soundcloud.com/{slug}"), &slug)?;
        if let Some(errors) = track.get("errors").and_then(Value::as_array)
            && !errors.is_empty()
        {
            let messages: Vec<String> = errors
                .iter()
                .map(|error| str_or_none(error.get("error_message")).unwrap_or_default())
                .collect();
            return Err(Error::Reported(format!(
                "{RELATED_NAME} said: {}",
                messages.join(",")
            )));
        }
        let track_id = str_or_none(track.get("id")).unwrap_or_default();
        let path = match relation.as_str() {
            "albums" => format!("tracks/{track_id}/albums"),
            "sets" => format!("tracks/{track_id}/playlists_without_albums"),
            _ => format!("tracks/{track_id}/related"),
        };
        let name = track
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .unwrap_or(&slug)
            .to_owned();
        paged_playlist(
            client,
            url,
            (RELATED_NAME, "SoundcloudRelated"),
            &format!("{API_V2_BASE}{path}"),
            track_id,
            format!("{name} ({})", capitalize(&relation)),
        )
        .map(Some)
    }
}

impl Extractor for SoundcloudSearch {
    fn key(&self) -> &'static str {
        "SoundcloudSearch"
    }

    login_hooks!();

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    /// `SearchInfoExtractor._real_extract`: the prefix's number of results
    /// (`scsearch` one, `scsearchall` every result).
    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SEARCH_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(client, SEARCH_NAME, url, self)?;
        let query = found["query"].to_owned();
        let n = match &found["prefix"] {
            "" => Some(1),
            "all" => None,
            prefix => prefix.parse::<usize>().ok(),
        };
        if n == Some(0) {
            return Err(Error::Reported(format!(
                "invalid download number 0 for query \"{query}\""
            )));
        }
        let entries = search_entries(client, &query, n)?;
        Ok(Some(playlist(
            url,
            (SEARCH_NAME, "SoundcloudSearch"),
            (query.clone(), Some(query)),
            None,
            Vec::new(),
            entries,
            false,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(set_suitable(
            "https://soundcloud.com/the-concept-band/sets/the-royal-concept-ep"
        ));
        assert!(set_suitable(
            "https://soundcloud.com/the-concept-band/sets/the-royal-concept-ep/token"
        ));
        assert!(playlist_suitable(
            "https://api.soundcloud.com/playlists/4110309"
        ));
        assert!(playlist_suitable(
            "https://api.soundcloud.com/playlists/soundcloud%3Aplaylists%3A1759227795"
        ));
        assert!(user_suitable("https://soundcloud.com/soft-cell-official"));
        assert!(user_suitable(
            "https://soundcloud.com/soft-cell-official/tracks"
        ));
        assert!(!user_suitable(
            "https://soundcloud.com/stations/track/foo/bar"
        ));
        assert!(user_permalink_suitable(
            "https://api.soundcloud.com/users/30909869"
        ));
        assert!(track_station_suitable(
            "https://soundcloud.com/stations/track/officialsundial/your-text"
        ));
        assert!(related_suitable(
            "https://soundcloud.com/wajang/sexapil-pingers-5/recommended"
        ));
        assert!(!related_suitable(
            "https://soundcloud.com/wajang/sexapil-pingers-5"
        ));
        assert!(search_suitable("scsearch15:post-avant jazzcore"));
        assert!(search_suitable("scsearchall:x"));
        assert!(!search_suitable("ytsearch:x"));
    }

    #[test]
    fn capitalize_like_python() {
        assert_eq!(capitalize("all"), "All");
        assert_eq!(capitalize("tracks"), "Tracks");
    }
}
