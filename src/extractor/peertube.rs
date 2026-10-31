//! Upstream `PeerTubeIE` and `PeerTubePlaylistIE`
//! (`yt_dlp/extractor/peertube.py`): a known instance's video (or a
//! `peertube:HOST:ID` pseudo-URL) through the `/api/v1/videos/ID/` JSON (its
//! HLS master playlists, web-video and HLS files, the description and
//! captions endpoints), and an instance's accounts, video channels and video
//! playlists as paged `url` playlists, whose pages are pending entries
//! fetched while processing like upstream's `OnDemandPagedList`.
mod instances;

use super::{Extractor, dict_format, dict_info};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct PeerTube;
pub struct PeerTubePlaylist;

pub(crate) const NAME: &str = "PeerTube";
const PLAYLIST_NAME: &str = "PeerTube:Playlist";
const PLAYLIST_KEY: &str = "PeerTubePlaylist";
const PAGE_SIZE: usize = 30;

const UUID_RE: &str =
    r"[\da-zA-Z]{22}|[\da-fA-F]{8}-[\da-fA-F]{4}-[\da-fA-F]{4}-[\da-fA-F]{4}-[\da-fA-F]{12}";

/// `_INSTANCES_RE`.
fn instances_re() -> String {
    format!("(?:{})", instances::INSTANCES.join("|"))
}

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:peertube:(?P<host>[^:]+):|https?://(?P<host_2>{})/(?:videos/(?:watch|embed)|api/v\d/videos|w)/)(?P<id>{UUID_RE})",
        instances_re()
    ))
    .unwrap()
});
/// `PeerTubePlaylistIE._TYPES`, in upstream order.
const TYPES: [(&str, &str); 3] = [
    ("a", "accounts"),
    ("c", "video-channels"),
    ("w/p", "video-playlists"),
];
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^https?://(?P<host>{})/(?P<type>(?:a|c|w/p))/(?P<id>[^/]+)",
        instances_re()
    ))
    .unwrap()
});

/// `PeerTubeIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_EMBED_REGEX`. Upstream interpolates `_UUID_RE` ungrouped, so its
/// dashed alternative stands alone (a bare UUID `src`, which `suitable`
/// then refuses) and only short IDs match after `/videos/embed/`.
static EMBED: LazyLock<[Regex; 1]> = LazyLock::new(|| {
    [Regex::new(&format!(
        r#"<iframe[^>]+\bsrc=["'](?P<url>(?:https?:)?//{}/videos/embed/{UUID_RE})"#,
        instances_re()
    ))
    .unwrap()]
});

/// `PeerTubeIE._extract_embed_urls`: the `_EMBED_REGEX` embeds, else the
/// page itself as `peertube:HOST:ID` when its URL is a video URL and it
/// looks like a PeerTube page (`_extract_peertube_url`).
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let found = super::embed_regex_urls(url, webpage, &*EMBED, suitable);
    if !found.is_empty() {
        return found;
    }
    static PAGE_URL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(
            r"^https?://(?P<host>[^/]+)/(?:videos/(?:watch|embed)|w)/(?P<id>{UUID_RE})"
        ))
        .unwrap()
    });
    const MARKERS: [&str; 4] = [
        r#"meta property="og:platform" content="PeerTube""#,
        "<title>PeerTube<",
        "There will be other non JS-based clients to access PeerTube",
        ">We are sorry but it seems that PeerTube is not compatible with your web browser.<",
    ];
    PAGE_URL
        .captures(url.as_str())
        .filter(|_| MARKERS.iter().any(|marker| webpage.contains(marker)))
        .and_then(|found| Url::parse(&format!("peertube:{}:{}", &found["host"], &found["id"])).ok())
        .into_iter()
        .collect()
}

/// `PeerTubePlaylistIE._match_valid_url`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("PeerTube {what} are not supported natively"))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `self._get_headers()`: `x-peertube-video-password` with
/// `--video-password`.
fn video_headers() -> Option<(&'static str, String)> {
    super::video_password().map(|password| ("x-peertube-video-password", password))
}

/// `_call_api(host, video_id, path, note, errnote=None)`: upstream passes
/// `errnote=None`, so failures read `Unable to download webpage`.
fn call_api(
    client: &Client,
    host: &str,
    video_id: &str,
    path: &str,
    note: &str,
    fatal: bool,
) -> Result<Option<Value>> {
    let api = Url::parse(&format!("https://{host}/api/v1/videos/{video_id}/{path}"))?;
    let header = video_headers();
    let headers: Vec<(&str, &str)> = header
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    match super::download_json_note_headers(client, &api, NAME, video_id, note, &headers) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) if !fatal => {
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: Unable to download webpage: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(Error::ExtractorRequest {
            extractor,
            id,
            source,
            ..
        }) => Err(Error::ExtractorRequest {
            extractor,
            id,
            what: "webpage",
            source,
        }),
        Err(Error::Reported(message)) if !fatal && message.contains(": Failed to parse JSON ") => {
            crate::screen::warning(&message);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// `try_get(value, lambda x: x[a][b]...)` without a type check.
fn try_get<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
}

fn string_at(value: &Value, path: &[&str]) -> Value {
    try_get(value, path)
        .filter(|value| value.is_string())
        .cloned()
        .unwrap_or(Value::Null)
}

/// `str_or_none(try_get(..., int))`: Python's `str` of an `int` (a `bool`
/// is one too).
fn int_id_at(value: &Value, path: &[&str]) -> Value {
    match try_get(value, path) {
        Some(Value::Number(number)) if number.is_i64() || number.is_u64() => {
            Value::from(number.to_string())
        }
        Some(Value::Bool(flag)) => Value::from(if *flag { "True" } else { "False" }),
        _ => Value::Null,
    }
}

/// `url_or_none` of a string value.
fn url_value(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map_or(Value::Null, Value::from)
}

/// `unified_timestamp` of a string value.
fn timestamp(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(|date| crate::dates::unified_timestamp(date, true))
        .map_or(Value::Null, Value::from)
}

/// Python's `str(value)` of a JSON scalar, for f-strings and `str_or_none`.
fn py_str(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Null => Ok("None".into()),
        Value::Bool(flag) => Ok(if *flag { "True" } else { "False" }.into()),
        Value::Number(number) if number.is_i64() || number.is_u64() => Ok(number.to_string()),
        _ => Err(unsupported("identifiers that are not strings or integers")),
    }
}

/// `_get_subtitles(host, video_id)`.
fn subtitles(client: &Client, host: &str, video_id: &str) -> Result<crate::Subtitles> {
    let mut subtitles = crate::Subtitles::default();
    let Some(captions) = call_api(
        client,
        host,
        video_id,
        "captions",
        "Downloading captions JSON",
        false,
    )?
    else {
        return Ok(subtitles);
    };
    let Some(data) = captions.get("data").and_then(Value::as_array) else {
        return Ok(subtitles);
    };
    let base = format!("https://{host}");
    for caption in data {
        let language = string_at(caption, &["language", "id"]);
        let Some(caption_url) = caption
            .get("captionPath")
            .and_then(Value::as_str)
            .and_then(|path| super::videopress::urljoin(Some(&base), path))
        else {
            continue;
        };
        let caption_url = Url::parse(&caption_url)?;
        let ext = super::direct::determine_ext(caption_url.as_str())
            .unwrap_or_else(|| "unknown_video".into());
        let track: crate::SubtitleFormat =
            serde_json::from_value(json!({"url": caption_url, "ext": ext}))
                .map_err(|error| Error::Reported(error.to_string()))?;
        // `subtitles.setdefault(language_id or 'en', [])`.
        let language = match language.as_str() {
            Some(language) if !language.is_empty() => language.to_owned(),
            _ => "en".to_owned(),
        };
        match subtitles
            .entries
            .iter_mut()
            .find(|(existing, _)| existing.as_deref() == Some(language.as_str()))
        {
            Some((_, tracks)) => tracks.push(track),
            None => subtitles.entries.push((Some(language), vec![track])),
        }
    }
    Ok(subtitles)
}

/// A `files` entry's format dict, or `None` when it is skipped.
fn file_format(file: &Value, video_id: &str) -> Result<Option<crate::VideoFormat>> {
    if !file.is_object() {
        return Ok(None);
    }
    let file_url = url_value(file.get("fileUrl"));
    if file_url.is_null() {
        return Ok(None);
    }
    let format_id = string_at(file, &["resolution", "label"]);
    // `parse_resolution(format_id)`, then the `update`d keys.
    let mut fields = Vec::new();
    if let Some(label) = format_id.as_str() {
        let (width, height) = super::html5::parse_resolution(label);
        if let Some(width) = width {
            fields.push(("width", Value::from(width)));
        }
        if let Some(height) = height {
            fields.push(("height", Value::from(height)));
        }
    }
    let audio_only = format_id.as_str() == Some("0p");
    fields.extend([
        ("url", file_url),
        ("format_id", format_id),
        ("filesize", super::streamable::int_or_none(file.get("size"))),
    ]);
    if audio_only {
        fields.push(("vcodec", "none".into()));
    } else {
        fields.push(("fps", super::streamable::int_or_none(file.get("fps"))));
    }
    dict_format(fields, NAME, video_id).map(Some)
}

impl Extractor for PeerTube {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let host = found
            .name("host")
            .or_else(|| found.name("host_2"))
            .map(|host| host.as_str().to_owned())
            .unwrap_or_default();
        let video_id = found["id"].to_owned();
        announce(NAME, url);
        let video = call_api(client, &host, &video_id, "", "Downloading video JSON", true)?
            .unwrap_or(Value::Null);
        if !video.is_object() {
            return Err(unsupported("video JSON that is not an object"));
        }
        let title = video
            .get("name")
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&video_id, "name"))?;
        if !title.is_string() {
            return Err(unsupported("video names that are not strings"));
        }

        let header = video_headers();
        let headers: Vec<(&str, &str)> = header
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        let mut formats = Vec::new();
        let mut is_live = false;
        let mut files: Vec<Value> = match video.get("files") {
            Some(Value::Array(files)) => files.clone(),
            Some(files) if super::nuxt::truthy(files) => {
                return Err(unsupported("file lists that are not arrays"));
            }
            _ => Vec::new(),
        };
        let playlists = match video.get("streamingPlaylists") {
            Some(Value::Array(playlists)) => playlists.clone(),
            Some(playlists) if super::nuxt::truthy(playlists) => {
                return Err(unsupported("streaming playlist lists that are not arrays"));
            }
            _ => Vec::new(),
        };
        for playlist in &playlists {
            if !playlist.is_object() {
                continue;
            }
            if let Some(playlist_url) = url_value(playlist.get("playlistUrl")).as_str() {
                is_live = true;
                let manifest = Url::parse(playlist_url)?;
                if let Some(stream) = super::extract_m3u8_live_optional_headers(
                    client, &manifest, NAME, &video_id, &headers,
                ) {
                    formats.extend(stream.formats);
                }
            }
            match playlist.get("files") {
                Some(Value::Array(playlist_files)) if !playlist_files.is_empty() => {
                    files.extend(playlist_files.iter().cloned());
                }
                _ => {}
            }
        }
        for file in &files {
            if let Some(format) = file_format(file, &video_id)? {
                is_live = false;
                formats.push(format);
            }
        }

        let mut description = match video.get("description") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(text)) => Value::from(text.clone()),
            Some(_) => return Err(unsupported("descriptions that are not strings")),
        };
        // A description of 250 characters or more is shortened.
        if description
            .as_str()
            .is_some_and(|text| text.chars().count() >= 250)
            && let Some(full) = call_api(
                client,
                &host,
                &video_id,
                "description",
                "Downloading description JSON",
                false,
            )?
            && full.is_object()
        {
            // `str_or_none(full_description.get('description')) or description`.
            let full = match full.get("description") {
                None | Some(Value::Null) => None,
                Some(value) => Some(py_str(value)?),
            };
            if let Some(full) = full.filter(|text| !text.is_empty()) {
                description = Value::from(full);
            }
        }

        let subtitles = if super::subtitles_requested() {
            subtitles(client, &host, &video_id)?
        } else {
            crate::Subtitles::default()
        };

        let categories = match string_at(&video, &["category", "label"]) {
            Value::String(category) if !category.is_empty() => json!([category]),
            _ => Value::Null,
        };
        let webpage_url = format!("https://{host}/videos/watch/{video_id}");
        let thumbnail = video
            .get("thumbnailPath")
            .and_then(Value::as_str)
            .and_then(|path| super::videopress::urljoin(Some(&webpage_url), path))
            .map_or(Value::Null, Value::from);
        let tags = video
            .get("tags")
            .filter(|tags| tags.is_array())
            .cloned()
            .unwrap_or(Value::Null);
        let http_headers: Map<String, Value> = header
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Value::from(value.clone())))
            .collect();
        let int = |key: &str| super::streamable::int_or_none(video.get(key));
        let fields = vec![
            ("id", Value::from(video_id.clone())),
            ("title", title),
            ("description", description),
            ("thumbnail", thumbnail),
            ("timestamp", timestamp(video.get("publishedAt"))),
            ("uploader", string_at(&video, &["account", "displayName"])),
            ("uploader_id", int_id_at(&video, &["account", "id"])),
            (
                "uploader_url",
                url_value(try_get(&video, &["account", "url"])),
            ),
            ("channel", string_at(&video, &["channel", "displayName"])),
            ("channel_id", int_id_at(&video, &["channel", "id"])),
            (
                "channel_url",
                url_value(try_get(&video, &["channel", "url"])),
            ),
            ("language", string_at(&video, &["language", "id"])),
            ("license", string_at(&video, &["licence", "label"])),
            ("duration", int("duration")),
            ("view_count", int("views")),
            ("like_count", int("likes")),
            ("dislike_count", int("dislikes")),
            // Upstream's `nsfw is bool` is never true, so `age_limit` is
            // always `None`.
            ("age_limit", Value::Null),
            ("tags", tags),
            ("categories", categories),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("is_live", Value::Bool(is_live)),
            ("webpage_url", Value::from(webpage_url.clone())),
            ("http_headers", Value::Object(http_headers)),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.webpage_url = Url::parse(&webpage_url)?;
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

/// `url_result(f'https://{host}/w/{short_uuid}', PeerTubeIE.ie_key(),
/// video_id=short_uuid, video_title=video_title)`.
fn url_entry(target: &str, id: Option<String>, title: Option<Value>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // `ie.get_temp_id(url)` without an id.
    if let Some(id) = id.clone().or_else(|| {
        VALID_URL
            .captures(target)
            .map(|found| found["id"].to_owned())
    }) {
        info.id = id;
    }
    let mut entry = Map::new();
    entry.insert("ie_key".into(), NAME.into());
    if let Some(id) = id {
        entry.insert("id".into(), id.into());
    }
    if let Some(title) = title {
        entry.insert("title".into(), title);
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// Python's type name, for upstream's `AttributeError` on a non-dict.
fn py_type(value: Option<&Value>) -> &'static str {
    match value {
        None | Some(Value::Null) => "NoneType",
        Some(Value::Bool(_)) => "bool",
        Some(Value::Number(number)) if number.is_f64() => "float",
        Some(Value::Number(_)) => "int",
        Some(Value::String(_)) => "str",
        Some(Value::Array(_)) => "list",
        Some(Value::Object(_)) => "dict",
    }
}

/// Whether a playlist's pending state is a PeerTube playlist's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == "peertube"
}

/// `fetch_page(page)`: the `/videos?sort=-createdAt` page's `url` results.
fn fetch_page(
    client: &Client,
    host: &str,
    playlist_type: &str,
    playlist_id: &str,
    page: usize,
) -> Result<Vec<VideoInfo>> {
    let api = Url::parse(&format!(
        "https://{host}/api/v1/{playlist_type}/{playlist_id}/videos?sort=-createdAt&start={}&count={PAGE_SIZE}&nsfw=both",
        PAGE_SIZE * page
    ))?;
    let data = super::download_json_note(
        client,
        &api,
        PLAYLIST_NAME,
        playlist_id,
        &format!("Downloading page {}", page + 1),
    )?;
    if !data.is_object() {
        return Err(Error::Reported(format!(
            "'{}' object has no attribute 'get'",
            py_type(Some(&data))
        )));
    }
    let videos = match data.get("data") {
        None => &Vec::new(),
        Some(Value::Array(videos)) => videos,
        Some(_) => return Err(unsupported("page data that are not arrays")),
    };
    let mut fetched = Vec::with_capacity(videos.len());
    for video in videos {
        let field = |key: &str| {
            video
                .get(key)
                .filter(|value| super::nuxt::truthy(value))
                .or_else(|| try_get(video, &["video", key]))
                .filter(|value| !value.is_null())
        };
        let short_uuid = field("shortUUID").map(py_str).transpose()?;
        let target = format!(
            "https://{host}/w/{}",
            short_uuid.as_deref().unwrap_or("None")
        );
        fetched.push(url_entry(&target, short_uuid, field("name").cloned())?);
    }
    Ok(fetched)
}

/// `OnDemandPagedList(fetch_page, 30)` read by upstream's `PlaylistEntries`
/// (see [`super::paged`]).
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let text = |key: &str| state[key].as_str().unwrap_or_default().to_owned();
    let (host, playlist_type, playlist_id) =
        (text("host"), text("playlist_type"), text("playlist_id"));
    let mut fetch = |page| fetch_page(client, &host, &playlist_type, &playlist_id, page);
    let placeholder = || url_entry(&format!("https://{host}/w/None"), None, None);
    super::paged::fetch_requested(state, limit, entries, PAGE_SIZE, &mut fetch, &placeholder)
}

impl Extractor for PeerTubePlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let host = found["host"].to_owned();
        let playlist_id = found["id"].to_owned();
        let playlist_type = TYPES
            .iter()
            .find(|(short, _)| *short == &found["type"])
            .map(|(_, name)| *name)
            .unwrap_or_default();
        announce(PLAYLIST_NAME, url);
        // `call_api(..., fatal=False)`: a failed request is `False`, a body
        // that does not parse is `None`.
        let api = Url::parse(&format!(
            "https://{host}/api/v1/{playlist_type}/{playlist_id}"
        ))?;
        let info = match super::download_json_note(
            client,
            &api,
            PLAYLIST_NAME,
            &playlist_id,
            "Downloading playlist information",
        ) {
            Ok(info) => Some(info),
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{PLAYLIST_NAME}] {playlist_id}: Unable to download JSON metadata: {}",
                    crate::upstream_error_text(&source)
                ));
                Some(Value::Bool(false))
            }
            Err(Error::Reported(message)) if message.contains(": Failed to parse JSON ") => {
                crate::screen::warning(&message);
                None
            }
            Err(error) => return Err(error),
        };
        let info = match info {
            Some(info @ Value::Object(_)) => info,
            other => {
                return Err(Error::Reported(format!(
                    "'{}' object has no attribute 'get'",
                    py_type(other.as_ref())
                )));
            }
        };
        let get = |key: &str| info.get(key).filter(|value| !value.is_null());
        let title = match get("displayName") {
            None => None,
            Some(Value::String(title)) => Some(title.clone()).filter(|title| !title.is_empty()),
            Some(_) => return Err(unsupported("playlist names that are not strings")),
        };
        let description = get("description").cloned();
        let owner = |key: &str| {
            try_get(&info, &["ownerAccount", key]).filter(|value| super::nuxt::truthy(value))
        };
        let channel = owner("name")
            .or_else(|| get("displayName"))
            .cloned()
            .unwrap_or(Value::Null);
        let channel_id = match owner("id").or_else(|| get("id")) {
            None => Value::Null,
            Some(value) => Value::from(py_str(value)?),
        };
        // `format_field(info, 'thumbnailPath', f'https://{host}%s')`.
        let thumbnail = match get("thumbnailPath") {
            Some(Value::String(path)) if path.is_empty() => String::new(),
            Some(path) => format!("https://{host}{}", py_str(path)?),
            None => String::new(),
        };

        let mut extra = Map::new();
        extra.insert("timestamp".into(), timestamp(info.get("createdAt")));
        extra.insert("channel".into(), channel);
        extra.insert("channel_id".into(), channel_id);
        extra.insert("thumbnail".into(), thumbnail.into());
        let mut keys = vec!["timestamp", "channel", "channel_id", "thumbnail", "id"];
        if title.is_some() {
            keys.push("title");
        }
        if let Some(description) = description {
            extra.insert("description".into(), description);
            keys.push("description");
        }
        // `OnDemandPagedList`, whose full count, like a generator's, is
        // unknown unless a selection reads past its end.
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;{}{}",
                keys.join(","),
                crate::info_order::GENERATOR
            )
            .into(),
        );
        let state = json!({
            "kind": "peertube",
            "host": host,
            "playlist_type": playlist_type,
            "playlist_id": playlist_id,
            "random_access": true,
        });
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_KEY.into(),
            nested: Vec::new(),
            pending: Some(crate::PendingEntries { state }),
            entries: Vec::new(),
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "https://framatube.org/videos/watch/9c9de5e8-0a1e-484a-b099-e80766180a6d",
            "https://peertube2.cpy.re/w/122d093a-1ede-43bd-bd34-59d2931ffc5e",
            "https://peertube2.cpy.re/w/3fbif9S3WmtTP8gGsC5HBd",
            "https://peertube2.cpy.re/api/v1/videos/3fbif9S3WmtTP8gGsC5HBd",
            "peertube:spacepub.space:d8943b2d-8280-497b-85ec-bc282ec2afdc",
            "https://peertube.debian.social/videos/watch/0b04f13d-1e18-4f1d-814e-4979aa7c9c44",
            "https://vod.ksite.de/videos/embed/fed67262-6edb-4d1c-833b-daa9085c71d7",
            "https://peertube.tv/api/v1/videos/c1875674-97d0-4c94-a058-3f7e64c962e8",
            "peertube:framatube.org:b37a5b9f-e6b5-415c-b700-04a5cd6ec205",
            "https://videos.john-livingston.fr/w/mna1A6SxZ94cra4hMtjRQm",
            "https://FIGHTFORINFO.com/w/mna1A6SxZ94cra4hMtjRQm",
        ] {
            assert!(suitable(url), "{url}");
        }
        // Unlisted hosts (GenericIE's embed detection, not ported) and
        // upstream's case-sensitive host list.
        assert!(!suitable(
            "https://video.macver.org/w/6gvhZpUGQVd4SQ6oYDc9pC"
        ));
        assert!(!suitable("https://FRAMATUBE.org/w/mna1A6SxZ94cra4hMtjRQm"));
        assert!(!suitable(
            "https://framatube.org/w/p/hFdJoTuyhNJVa1cDWd1d12"
        ));
        for url in [
            "https://peertube.debian.social/w/p/hFdJoTuyhNJVa1cDWd1d12",
            "https://peertube2.cpy.re/a/chocobozzz/videos",
            "https://framatube.org/c/bf54d359-cfad-4935-9d45-9d6be93f63e8/videos",
            "https://peertube2.cpy.re/c/blender_open_movies@video.blender.org/videos",
        ] {
            assert!(playlist_suitable(url), "{url}");
        }
        assert!(!playlist_suitable(
            "https://framatube.org/w/9c9de5e8-0a1e-484a-b099-e80766180a6d"
        ));
    }

    #[test]
    fn instances_follow_upstream_order() {
        assert_eq!(instances::INSTANCES.first(), Some(&r"0ch\.tv"));
        assert_eq!(instances::INSTANCES.last(), Some(&r"canard\.tube"));
        assert!(instances::INSTANCES.len() > 1000);
    }
}
