//! Upstream `KickIE`, `KickVODIE`, `KickClipIE`, `KickVideosIE` and
//! `KickClipsIE` (`yt_dlp/extractor/kick.py`, as reworked in yt-dlp
//! fe84bef3969de96ceb39ca988425fdd3895bbfb1): the site APIs (`kick.com/api/v2`
//! and `web.kick.com/api/v1`, requested with upstream's impersonation
//! warning, as impersonation is unavailable here) for live channels, VODs
//! (the playback endpoint, with the channel's video list for metadata),
//! clips, and a channel's video and clip lists, their HLS (or direct)
//! formats and metadata.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct KickLive;
pub struct KickVod;
pub struct KickClip;
pub struct KickVideos;
pub struct KickClips;

const BASE_URL: &str = "https://kick.com";
const LIVE: &str = "kick:live";
const VOD: &str = "kick:vod";
const CLIP: &str = "kick:clip";
const VIDEOS: &str = "kick:videos";
const CLIPS: &str = "kick:clips";

static LIVE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?kick\.com/(?P<id>[\w-]+)").unwrap());
static VOD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?kick\.com/[\w-]+/videos/(?P<id>[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12})")
        .unwrap()
});
static CLIP_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?kick\.com/[\w-]+(?:/clips/|/?\?(?:[^#]+&)?clip=)(?P<id>clip_[\w-]+)",
    )
    .unwrap()
});
static VIDEOS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?kick\.com/(?P<id>[\w-]+)/videos/?(?:[?#]|$)").unwrap()
});
static CLIPS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?kick\.com/(?P<id>[\w-]+)/clips/?(?:[?#]|$)").unwrap()
});

/// `KickVODIE.suitable`.
pub fn vod_suitable(url: &str) -> bool {
    VOD_URL.is_match(url)
}

/// `KickClipIE.suitable`.
pub fn clip_suitable(url: &str) -> bool {
    CLIP_URL.is_match(url)
}

/// `KickVideosIE.suitable`.
pub fn videos_suitable(url: &str) -> bool {
    VIDEOS_URL.is_match(url)
}

/// `KickClipsIE.suitable`.
pub fn clips_suitable(url: &str) -> bool {
    CLIPS_URL.is_match(url)
}

/// `KickIE.suitable`: not a VOD, clip, video or clip list, nor a reserved
/// path.
pub fn live_suitable(url: &str) -> bool {
    if vod_suitable(url) || clip_suitable(url) || videos_suitable(url) || clips_suitable(url) {
        return false;
    }
    let Some(found) = LIVE_URL.captures(url) else {
        return false;
    };
    let id = &found["id"];
    let after = &url[found.get(0).map_or(0, |whole| whole.end())..];
    let reserved = matches!(id, "video" | "categories" | "search" | "auth")
        && (after.is_empty() || after.starts_with(['/', '?', '#']));
    !reserved
}

fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current).filter(|value| !value.is_null())
}

/// Python `str(value)` of a JSON scalar (`str_or_none`).
fn py_str(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(true) => Some("True".into()),
        Value::Bool(false) => Some("False".into()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `({int}, {str_or_none})`: an int (Python's `bool` included) as text.
fn int_text(value: Option<&Value>) -> Option<Value> {
    value
        .filter(|value| value.is_i64() || value.is_u64() || value.is_boolean())
        .and_then(py_str)
        .map(Value::from)
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// `{str}, filter`: a nonempty string.
fn text(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(Value::from)
}

/// `{clean_html}, filter`.
fn clean(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(crate::html::clean_html)
        .filter(|text| !text.is_empty())
}

/// `int_or_none`.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|number| number.is_finite())
                .map(|number| number.trunc() as i64)
        }),
        Value::String(text) => {
            let text = text.trim();
            text.strip_prefix('+').unwrap_or(text).parse().ok()
        }
        _ => None,
    }
}

fn int(value: Option<&Value>) -> Option<Value> {
    int_or_none(value).map(Value::from)
}

fn url(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map(Value::from)
}

/// `('is_mature', {bool}, {lambda x: 18 if x else None})`.
fn age(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_bool)
        .filter(|mature| *mature)
        .map(|_| Value::from(18))
}

fn boolean(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| value.is_boolean()).cloned()
}

/// `(..., {clean_html}, filter, all, filter)`: the cleaned nonempty texts,
/// omitted when there are none.
fn clean_list<'a>(values: impl Iterator<Item = &'a Value>) -> Option<Value> {
    let found: Vec<Value> = values
        .filter_map(|value| clean(Some(value)))
        .map(Value::from)
        .collect();
    (!found.is_empty()).then_some(Value::Array(found))
}

fn iso8601(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map(Value::from)
}

fn unified(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(|date| crate::dates::unified_timestamp(date, false))
        .map(Value::from)
}

fn insert_all(dict: &mut Map<String, Value>, fields: Vec<(&str, Option<Value>)>) {
    for (key, value) in fields {
        if let Some(value) = value {
            dict.insert(key.to_owned(), value);
        }
    }
}

/// An `ExtractorError` with `expected=True`.
fn expected(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{name}] {id}: {message}"))
}

/// An unexpected `ExtractorError` (`require(...)` and the like).
fn bug(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{name}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// One `_call_api` request.
struct ApiCall<'a> {
    path: &'a str,
    id: &'a str,
    note: &'a str,
    query: &'a [(&'a str, &'a str)],
    /// A JSON POST body (`data=`).
    body: Option<&'a str>,
    /// `expected_status=`.
    expected: &'a [u16],
}

impl<'a> ApiCall<'a> {
    fn new(path: &'a str, id: &'a str) -> Self {
        Self {
            path,
            id,
            note: "Downloading API JSON",
            query: &[],
            body: None,
            expected: &[],
        }
    }
}

/// `_call_api(path, display_id, ...)`: `v1/...` on `web.kick.com`, `v2/...`
/// on `kick.com`, with the `session_token` cookie as a bearer token.
fn call_api(client: &Client, name: &'static str, call: ApiCall<'_>) -> Result<Value> {
    use std::io::Read;
    let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
        crate::screen::extractor_warning_once(&format!("[{name}] {message}"));
    });
    let domain = match call.path.split('/').next() {
        Some("v1") => "web.kick.com",
        _ => "kick.com",
    };
    let mut api = Url::parse(&format!("https://{domain}/api/{}", call.path))?;
    if !call.query.is_empty() {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(call.query)
            .finish();
        api.set_query(Some(&query));
    }
    let token = crate::cookies::get_cookie(&format!("{BASE_URL}/"), "session_token").map(|token| {
        format!(
            "Bearer {}",
            percent_encoding::percent_decode_str(&token).decode_utf8_lossy()
        )
    });
    let mut headers: Vec<(&str, &str)> = Vec::new();
    if let Some(token) = token.as_deref() {
        headers.push(("Authorization", token));
    }
    crate::screen::screen(&format!("[{name}] {}: {}", call.id, call.note));
    let response = match call.body {
        Some(body) => {
            headers.push(("Content-Type", "application/json"));
            super::direct::post_expecting(client, &api, &headers, body.to_owned(), call.expected)
        }
        None => super::direct::request_expecting(client, &api, &headers, call.expected),
    };
    let mut response = response.map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: name,
            id: call.id.to_owned(),
            what: "JSON metadata",
            source: Box::new(error),
        },
        error => error,
    })?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(call.id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    serde_json::from_slice(&body).map_err(|error| {
        Error::Reported(format!(
            "[{name}] {}: Failed to parse JSON  (caused by {error})",
            call.id
        ))
    })
}

/// `_get_creator_info(channel_slug)`: the channel's creator ID and username.
fn creator_info(
    client: &Client,
    name: &'static str,
    slug: &str,
    temp_id: &str,
) -> Result<(String, Option<String>)> {
    let path = format!("v2/channels/{slug}");
    let mut call = ApiCall::new(&path, slug);
    call.note = "Downloading creator info";
    let channel = call_api(client, name, call)?;
    let creator_id = int_text(channel.get("id"))
        .and_then(|id| id.as_str().map(str::to_owned))
        .ok_or_else(|| bug(name, temp_id, "Unable to extract creator ID"))?;
    let username =
        text(at(&channel, &["user", "username"])).and_then(|name| name.as_str().map(str::to_owned));
    Ok((creator_id, username))
}

/// `parse_resolution(text, lenient=True)['width']`.
fn lenient_width(text: &str) -> Option<i64> {
    static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\s*[xX×,]\s*(\d+)").unwrap());
    static WIDTH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:^|\D)(\d{2,5})w(?:$|[^a-zA-Z0-9])").unwrap());
    if let Some(found) = SIZE.captures(text) {
        return found[1].parse().ok();
    }
    if super::html5::parse_resolution(text) != (None, None) {
        return None;
    }
    WIDTH.captures(text)?[1].parse().ok()
}

/// Upstream `url_basename` of any text.
fn url_basename(text: &str) -> String {
    let path = match Url::parse(text) {
        Ok(url) => url.path().to_owned(),
        Err(_) => text.split(['?', '#']).next().unwrap_or_default().to_owned(),
    };
    path.trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// The `thumbnail.srcSet` candidates: `{url, width, height}` (the height
/// from the file name).
fn thumbnails(video: &Value) -> Option<Value> {
    let srcset = at(video, &["thumbnail", "srcSet"])?.as_str()?;
    let found: Vec<Value> = srcset
        .split(',')
        .filter_map(|candidate| {
            let candidate = candidate.trim();
            let parts: Vec<&str> = match candidate.rsplit_once(char::is_whitespace) {
                Some((url, descriptor)) => vec![url.trim_end(), descriptor],
                None if candidate.is_empty() => vec![],
                None => vec![candidate],
            };
            let mut thumbnail = Map::new();
            let first = parts.first().copied();
            insert_all(
                &mut thumbnail,
                vec![
                    ("url", url(first.map(Value::from).as_ref())),
                    (
                        "width",
                        parts.get(1).and_then(|w| lenient_width(w)).map(Value::from),
                    ),
                    (
                        "height",
                        first
                            .map(url_basename)
                            .and_then(|name| {
                                let stem = name.split('.').next().unwrap_or_default().to_owned();
                                int_or_none(Some(&Value::from(stem)))
                            })
                            .map(Value::from),
                    ),
                ],
            );
            (!thumbnail.is_empty()).then_some(Value::Object(thumbnail))
        })
        .collect();
    (!found.is_empty()).then_some(Value::Array(found))
}

/// `_extract_vod_metadata(video_data)`.
fn vod_metadata(video: &Value) -> Map<String, Value> {
    let availability = match video.get("status").and_then(Value::as_str) {
        Some("sub_only") => Value::from("subscriber_only"),
        Some("private") => Value::from("private"),
        Some("public") => Value::from("public"),
        _ => Value::Null,
    };
    let mut dict = Map::new();
    dict.insert("availability".into(), availability);
    insert_all(
        &mut dict,
        vec![
            ("title", clean(video.get("title")).map(Value::from)),
            ("age_limit", age(video.get("is_mature"))),
            (
                "categories",
                clean_list(at(video, &["category", "name"]).into_iter()),
            ),
            ("duration", int(video.get("duration"))),
            ("is_live", boolean(video.get("is_live"))),
            (
                "tags",
                clean_list(
                    video
                        .get("tags")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten(),
                ),
            ),
            ("thumbnails", thumbnails(video)),
            ("timestamp", iso8601(video.get("start_time"))),
            ("view_count", int(video.get("viewer_count"))),
            ("channel", text(at(video, &["channel", "username"]))),
            ("uploader", text(at(video, &["channel", "username"]))),
            ("uploader_id", text(at(video, &["channel", "slug"]))),
        ],
    );
    dict
}

/// `_extract_clip_metadata(clip_data)`.
fn clip_metadata(clip: &Value) -> Map<String, Value> {
    let first_int = |keys: &[&str]| keys.iter().find_map(|key| int(clip.get(*key)));
    let mut dict = Map::new();
    insert_all(
        &mut dict,
        vec![
            ("title", clean(clip.get("title")).map(Value::from)),
            ("age_limit", age(clip.get("is_mature"))),
            (
                "categories",
                clean_list(at(clip, &["category", "name"]).into_iter()),
            ),
            ("duration", int(clip.get("duration"))),
            ("like_count", first_int(&["likes_count", "likes"])),
            ("thumbnail", url(clip.get("thumbnail_url"))),
            ("timestamp", iso8601(clip.get("created_at"))),
            (
                "view_count",
                first_int(&["views_count", "view_count", "views"]),
            ),
            ("channel", text(at(clip, &["channel", "username"]))),
            ("channel_id", int_text(at(clip, &["channel", "id"]))),
            ("uploader", text(at(clip, &["channel", "username"]))),
            ("uploader_id", text(at(clip, &["channel", "slug"]))),
        ],
    );
    dict
}

/// A video from upstream's dict (`dict;KEYS` order).
fn video(
    page: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    name: &str,
    key: &str,
) -> VideoInfo {
    let first = formats
        .first()
        .map_or_else(|| page.clone(), |format| format.url.clone());
    let mut info = super::direct::build_at(first.clone(), &first, String::new());
    info.direct = None;
    if formats.is_empty() {
        info.extra
            .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
    }
    info.formats = formats;
    info.webpage_url = page.clone();
    info.extractor = name.into();
    info.extractor_key = key.into();
    let titled = dict.get("title").is_some_and(|title| !title.is_null());
    if !titled {
        info.extra
            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
    }
    let mut keys = Vec::new();
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "formats" => {}
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    // `process_video_result`: `live_status` from `is_live`.
    if info.extra.get("is_live") == Some(&Value::Bool(true)) {
        info.metadata.live_status = Some("is_live".into());
    }
    crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{};thumbnail=url,width,height",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    )
}

fn announce(name: &'static str, url: &Url) {
    // `InfoExtractor.initialize()`: `only_once` warnings are per extraction.
    crate::screen::reset_extractor_warnings();
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `_extract_m3u8_formats(m3u8_url, video_id, 'mp4')` (a missing URL is
/// `Failed to obtain m3u8 URL`).
fn hls(
    client: &Client,
    name: &'static str,
    source: Option<&str>,
    id: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let source = source.ok_or_else(|| bug(name, id, "Failed to obtain m3u8 URL"))?;
    let manifest = Url::parse(source)?;
    Ok(super::extract_m3u8_fatal(client, &manifest, name, id, None, false)?.formats)
}

/// `unsmuggle_url(url)`'s data.
fn unsmuggle(page: &Url) -> Option<Map<String, Value>> {
    let fragment = page.fragment()?;
    if !page.as_str().contains("#__youtubedl_smuggle") {
        return None;
    }
    let data = url::form_urlencoded::parse(fragment.as_bytes())
        .find(|(key, _)| key == "__youtubedl_smuggle")?
        .1;
    serde_json::from_str::<Value>(&data)
        .ok()?
        .as_object()
        .cloned()
}

/// `smuggle_url(url, data)`.
fn smuggle(url: &str, data: &Value) -> String {
    format!(
        "{url}#__youtubedl_smuggle={}",
        super::html5::quote_plus(&crate::output::python_json_dumps(data))
    )
}

/// The query's last nonempty `name` value (`parse_qs`).
fn query_param(page: &Url, name: &str) -> Option<String> {
    page.query_pairs()
        .filter(|(key, value)| key == name && !value.is_empty())
        .last()
        .map(|(_, value)| value.into_owned())
}

thread_local! {
    /// `KickVODIE._subscription_cache`.
    static SUBSCRIPTIONS: std::cell::RefCell<std::collections::HashMap<String, bool>> =
        std::cell::RefCell::default();
}

/// `_is_subscribed(creator_id)`.
fn is_subscribed(client: &Client, creator_id: &str) -> Result<bool> {
    if let Some(known) = SUBSCRIPTIONS.with(|cache| cache.borrow().get(creator_id).copied()) {
        return Ok(known);
    }
    let path = format!("v1/channels/{creator_id}/me");
    let mut call = ApiCall::new(&path, creator_id);
    call.note = "Checking subscription status";
    let me = call_api(client, VOD, call)?;
    let subscribed = at(&me, &["data", "subscription"]).is_some_and(Value::is_object);
    SUBSCRIPTIONS.with(|cache| cache.borrow_mut().insert(creator_id.to_owned(), subscribed));
    Ok(subscribed)
}

/// A playlist entry: upstream's `url_result` dict (`entry`), with the
/// fields a `url_transparent` entry overlays on the extracted video.
fn entry(raw: Map<String, Value>, ie_key: &str, extractor: &str) -> Result<VideoInfo> {
    let target = raw
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let transparent = raw.get("_type").and_then(Value::as_str) == Some("url_transparent");
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = raw.get("id").and_then(py_str).unwrap_or_default();
    info.title = raw
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let fields: Map<String, Value> = if transparent {
        raw.iter()
            .filter(|(key, _)| !matches!(key.as_str(), "_type" | "url" | "ie_key" | "id"))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    } else {
        Map::new()
    };
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": fields,
            "ie_key": ie_key,
            "extractor": extractor,
            "entry": Value::Object(raw),
        }),
    );
    Ok(info)
}

fn playlist(
    page: &Url,
    name: &str,
    key: &str,
    id: String,
    title: String,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "playlist-dict;id,title".into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title,
        webpage_url: page.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

impl Extractor for KickVod {
    fn key(&self) -> &'static str {
        VOD
    }

    fn extract(&self, client: &Client, page: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VOD_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(VOD, page);
        let smuggled = unsmuggle(page).unwrap_or_default();
        let path = format!("v1/stream/{id}/playback");
        let mut call = ApiCall::new(&path, &id);
        call.body = Some(
            r#"{"video_player": {"player": {}}, "video_session": {}, "user_session": {"non_personalised_ads": true}}"#,
        );
        call.expected = &[404];
        let playback = call_api(client, VOD, call)?;
        if let Some(data) = playback
            .get("data")
            .and_then(Value::as_object)
            .filter(|data| !data.is_empty())
        {
            let message: Vec<String> = ["type", "details"]
                .iter()
                .filter_map(|key| data.get(*key))
                .filter(|value| truthy(value))
                .filter_map(py_str)
                .collect();
            let message = if message.is_empty() {
                "API returned an error response".to_owned()
            } else {
                message.join(": ")
            };
            return Err(expected(VOD, &id, &message));
        }
        let creator_id = at(&playback, &["video_session", "creator_id"])
            .and_then(Value::as_str)
            .ok_or_else(|| bug(VOD, &id, "Unable to extract creator ID"))?
            .to_owned();
        let mut metadata = Map::new();
        let availability = if let Some(availability) = smuggled.get("availability") {
            availability.clone()
        } else {
            let path = format!("v1/channels/{creator_id}/videos");
            let videos = call_api(client, VOD, ApiCall::new(&path, &creator_id))?;
            let video_data = videos
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|video| video.get("id").and_then(Value::as_str) == Some(id.as_str()))
                .filter(|video| video.is_object())
                .cloned()
                .unwrap_or(Value::Null);
            metadata = vod_metadata(&video_data);
            metadata["availability"].clone()
        };
        let m3u8 = at(&playback, &["playback_url", "vod"])
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        if m3u8.is_none()
            && availability.as_str() == Some("subscriber_only")
            && !is_subscribed(client, &creator_id)?
        {
            return Err(expected(
                VOD,
                &id,
                "This video requires a channel subscription. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies",
            ));
        }
        let formats = hls(client, VOD, m3u8.as_deref(), &id)?;
        let mut dict = Map::new();
        dict.insert("id".into(), id.clone().into());
        dict.insert("channel_id".into(), creator_id.into());
        dict.insert("formats".into(), json!([]));
        dict.extend(metadata);
        Ok(Some(video(page, dict, formats, VOD, "KickVOD")))
    }
}

impl Extractor for KickClip {
    fn key(&self) -> &'static str {
        CLIP
    }

    fn extract(&self, client: &Client, page: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = CLIP_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(CLIP, page);
        let path = format!("v2/clips/{id}");
        let mut call = ApiCall::new(&path, &id);
        call.expected = &[404];
        let response = call_api(client, CLIP, call)?;
        if let Some(message) = clean(response.get("message")) {
            return Err(expected(CLIP, &id, &message));
        }
        let clip = response
            .get("clip")
            .filter(|clip| clip.is_object())
            .cloned()
            .unwrap_or(Value::Null);
        let clip_url = ["playback_url", "clip_url", "video_url"]
            .iter()
            .find_map(|key| {
                clip.get(*key)
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
            })
            .ok_or_else(|| bug(CLIP, &id, "Unable to extract clip source URL"))?;
        let formats = if super::direct::determine_ext(&clip_url).as_deref() == Some("m3u8") {
            hls(client, CLIP, Some(&clip_url), &id)?
        } else {
            let mut format: crate::VideoFormat = serde_json::from_value(json!({
                "url": clip_url,
                "format_id": "",
                "ext": super::direct::determine_ext(&clip_url).unwrap_or_else(|| "unknown_video".into()),
            }))
            .map_err(|error| Error::Reported(error.to_string()))?;
            crate::info_order::mark_format(&mut format, "url-only");
            vec![format]
        };
        let mut dict = Map::new();
        dict.insert("id".into(), id.clone().into());
        dict.insert("formats".into(), json!([]));
        dict.extend(clip_metadata(&clip));
        Ok(Some(video(page, dict, formats, CLIP, "KickClip")))
    }
}

impl Extractor for KickLive {
    fn key(&self) -> &'static str {
        LIVE
    }

    fn extract(&self, client: &Client, page: &Url) -> Result<Option<VideoInfo>> {
        if !live_suitable(page.as_str()) {
            return Ok(None);
        }
        let Some(found) = LIVE_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        let slug = temp_id.to_lowercase();
        announce(LIVE, page);
        let path = format!("v2/channels/{slug}");
        let response = call_api(client, LIVE, ApiCall::new(&path, &slug))?;
        let Some(live) = response
            .get("livestream")
            .filter(|live| live.is_object())
            .cloned()
        else {
            return Err(expected(LIVE, &slug, "The channel is not currently live"));
        };
        let playback = url(response.get("playback_url"))
            .and_then(|url| url.as_str().map(str::to_owned))
            .ok_or_else(|| bug(LIVE, &temp_id, "Unable to extract m3u8 URL"))?;
        let formats =
            super::extract_m3u8_fatal(client, &Url::parse(&playback)?, LIVE, &slug, None, false)?
                .formats;
        let mut dict = Map::new();
        dict.insert("formats".into(), json!([]));
        dict.insert("is_live".into(), true.into());
        dict.insert("uploader_id".into(), slug.clone().into());
        insert_all(
            &mut dict,
            vec![
                (
                    "channel_follower_count",
                    int(response.get("followers_count")),
                ),
                ("channel_id", int_text(response.get("id"))),
                ("channel_is_verified", boolean(response.get("verified"))),
                (
                    "id",
                    live.get("id")
                        .filter(|id| !id.is_null())
                        .and_then(py_str)
                        .map(Value::from),
                ),
                ("title", clean(live.get("session_title")).map(Value::from)),
                ("age_limit", age(live.get("is_mature"))),
                (
                    "categories",
                    clean_list(
                        live.get("categories")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|category| category.get("name")),
                    ),
                ),
                ("concurrent_view_count", int(live.get("viewer_count"))),
                ("release_timestamp", unified(live.get("start_time"))),
                (
                    "tags",
                    clean_list(
                        live.get("tags")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten(),
                    ),
                ),
                ("thumbnail", url(at(&live, &["thumbnail", "url"]))),
                ("timestamp", unified(live.get("created_at"))),
                ("channel", text(at(&response, &["user", "username"]))),
                (
                    "description",
                    clean(at(&response, &["user", "bio"])).map(Value::from),
                ),
                ("uploader", text(at(&response, &["user", "username"]))),
            ],
        );
        Ok(Some(video(page, dict, formats, LIVE, "Kick")))
    }
}

impl Extractor for KickVideos {
    fn key(&self) -> &'static str {
        VIDEOS
    }

    fn extract(&self, _client: &Client, _page: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VIDEOS_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        let slug = temp_id.to_lowercase();
        announce(VIDEOS, page);
        let (creator_id, username) = creator_info(client, VIDEOS, &slug, &temp_id)?;
        let by_views = query_param(page, "sort").as_deref() == Some("views");
        let title = format!(
            "{} - Videos sorted by {}",
            username.as_deref().unwrap_or(&slug),
            if by_views { "Views" } else { "Date" }
        );
        let path = format!("v1/channels/{creator_id}/videos");
        let videos = call_api(client, VIDEOS, ApiCall::new(&path, &slug))?;
        let mut data: Vec<Value> = videos
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if by_views {
            // `sort(key=lambda v: v['viewer_count'], reverse=True)`.
            let mut keyed = Vec::new();
            for video in data {
                let count = video
                    .get("viewer_count")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        bug(
                            VIDEOS,
                            &temp_id,
                            "An extractor error has occurred. (caused by KeyError('viewer_count'))",
                        )
                    })?;
                keyed.push((count, video));
            }
            keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
            data = keyed.into_iter().map(|(_, video)| video).collect();
        }
        let mut entries = Vec::new();
        for video_data in &data {
            let Some(video_id) = video_data
                .get("id")
                .filter(|id| !id.is_null())
                .and_then(py_str)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let metadata = vod_metadata(video_data);
            let target = smuggle(
                &format!("{BASE_URL}/{slug}/videos/{video_id}"),
                &json!({"availability": metadata["availability"]}),
            );
            let mut raw = metadata;
            raw.insert("ie_key".into(), "KickVOD".into());
            raw.insert("id".into(), video_data["id"].clone());
            raw.insert("_type".into(), "url_transparent".into());
            raw.insert("url".into(), target.into());
            entries.push(entry(raw, "KickVOD", VOD)?);
        }
        Ok(Some(playlist(
            page,
            VIDEOS,
            "KickVideos",
            slug,
            title,
            entries,
        )))
    }
}

impl Extractor for KickClips {
    fn key(&self) -> &'static str {
        CLIPS
    }

    fn extract(&self, _client: &Client, _page: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CLIPS_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        let slug = temp_id.to_lowercase();
        announce(CLIPS, page);
        let (creator_id, username) = creator_info(client, CLIPS, &slug, &temp_id)?;
        let by_views = query_param(page, "sort").as_deref() != Some("date");
        let range = query_param(page, "range")
            .filter(|range| matches!(range.as_str(), "day" | "week" | "month" | "all"))
            .unwrap_or_else(|| "week".into());
        let title = format!(
            "{} - Clips sorted by {} ({})",
            username.as_deref().unwrap_or(&slug),
            if by_views { "Views" } else { "Date" },
            match range.as_str() {
                "day" => "Last Day",
                "month" => "Last Month",
                "all" => "All Time",
                _ => "Last Week",
            }
        );
        let path = format!("v1/channels/{creator_id}/clips");
        let sort = if by_views { "views" } else { "date" };
        let mut cursor: Option<String> = None;
        let mut entries = Vec::new();
        for page_number in 1.. {
            let note = format!("Downloading page {page_number}");
            let mut query = vec![("sort", sort), ("time", range.as_str())];
            if let Some(cursor) = cursor.as_deref() {
                query.push(("cursor", cursor));
            }
            let mut call = ApiCall::new(&path, &slug);
            call.note = &note;
            call.query = &query;
            let clips = call_api(client, CLIPS, call)?;
            for clip_data in at(&clips, &["data", "clips"])
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(clip_id) = clip_data
                    .get("id")
                    .filter(|id| !id.is_null())
                    .and_then(py_str)
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                let mut raw = clip_metadata(clip_data);
                raw.insert("ie_key".into(), "KickClip".into());
                raw.insert("id".into(), clip_data["id"].clone());
                raw.insert("_type".into(), "url".into());
                raw.insert(
                    "url".into(),
                    format!("{BASE_URL}/{slug}/clips/{clip_id}").into(),
                );
                entries.push(entry(raw, "KickClip", CLIP)?);
            }
            cursor = at(&clips, &["data", "cursor"])
                .and_then(Value::as_str)
                .filter(|cursor| !cursor.is_empty())
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok(Some(playlist(
            page,
            CLIPS,
            "KickClips",
            slug,
            title,
            entries,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(vod_suitable(
            "https://kick.com/xqc/videos/8dd97a8d-e17f-48fb-8bc3-565f88dbc9ea"
        ));
        assert!(clip_suitable(
            "https://kick.com/mxddy?clip=clip_01GYXVB5Y8PWAPWCWMSBCFB05X"
        ));
        assert!(clip_suitable(
            "https://kick.com/destiny/clips/clip_01H9SKET879NE7N9RJRRDS98J3"
        ));
        assert!(videos_suitable("https://kick.com/xqc/videos"));
        assert!(videos_suitable("https://kick.com/xqc/videos/?sort=views"));
        assert!(!videos_suitable(
            "https://kick.com/xqc/videos/8dd97a8d-e17f-48fb-8bc3-565f88dbc9ea"
        ));
        assert!(clips_suitable(
            "https://kick.com/xqc/clips?sort=date&range=day"
        ));
        assert!(!clips_suitable(
            "https://kick.com/destiny/clips/clip_01H9SKET879NE7N9RJRRDS98J3"
        ));
        assert!(live_suitable("https://kick.com/buddha"));
        assert!(!live_suitable("https://kick.com/video/abc"));
        assert!(!live_suitable(
            "https://kick.com/destiny/clips/clip_01H9SKET879NE7N9RJRRDS98J3"
        ));
        assert!(!live_suitable("https://kick.com/xqc/videos"));
        assert!(!live_suitable("https://kick.com/xqc/clips"));
        assert!(!live_suitable("https://kick.com/categories"));
    }

    #[test]
    fn srcset_thumbnails_follow_upstream() {
        let video = json!({"thumbnail": {"srcSet": "https://a/x/1080.webp 1920w, https://a/x/thumb.webp, , x"}});
        assert_eq!(
            thumbnails(&video),
            Some(json!([
                {"url": "https://a/x/1080.webp", "width": 1920, "height": 1080},
                {"url": "https://a/x/thumb.webp"},
            ]))
        );
        assert_eq!(lenient_width("640w"), Some(640));
        assert_eq!(lenient_width("720p"), None);
        assert_eq!(lenient_width("1280x720"), Some(1280));
    }
}
