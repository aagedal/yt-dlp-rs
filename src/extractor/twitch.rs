//! Upstream `yt_dlp/extractor/twitch.py`: `TwitchVodIE` and `TwitchStreamIE`
//! (GraphQL metadata and playback access tokens, the usher HLS master, a
//! VOD's storyboard), `TwitchClipsIE` (a clip's signed quality URLs) and the
//! channel/collection lists (`TwitchVideosIE`, `TwitchVideosClipsIE`,
//! `TwitchVideosCollectionsIE`, `TwitchCollectionIE`), whose paged GraphQL
//! entries are `url_transparent` results.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use url::Url;

pub struct TwitchVod;
pub struct TwitchClips;
pub struct TwitchStream;
pub struct TwitchVideos;
pub struct TwitchVideosClips;
pub struct TwitchVideosCollections;
pub struct TwitchCollection;

const VOD_NAME: &str = "twitch:vod";
const VOD_KEY: &str = "TwitchVod";
const CLIPS_NAME: &str = "twitch:clips";
const CLIPS_KEY: &str = "TwitchClips";
const STREAM_NAME: &str = "twitch:stream";
const STREAM_KEY: &str = "TwitchStream";
const VIDEOS_NAME: &str = "twitch:videos";
const VIDEOS_KEY: &str = "TwitchVideos";
const VIDEOS_CLIPS_NAME: &str = "twitch:videos:clips";
const VIDEOS_CLIPS_KEY: &str = "TwitchVideosClips";
const VIDEOS_COLLECTIONS_NAME: &str = "twitch:videos:collections";
const VIDEOS_COLLECTIONS_KEY: &str = "TwitchVideosCollections";
const COLLECTION_NAME: &str = "twitch:collection";
const COLLECTION_KEY: &str = "TwitchCollection";
const CLIENT_ID: &str = "ue6666qo983tsx6so1t0vnawi233wa";
/// `_login_hint()` of an extractor supporting login (`_NETRC_MACHINE = 'twitch'`).
const LOGIN_HINT: &str = "Use --cookies, --cookies-from-browser, --username and --password, --netrc-cmd, or --netrc (twitch) to provide account credentials. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";

/// `TwitchBaseIE`'s login hooks.
macro_rules! twitch_login {
    () => {
        fn netrc_machine(&self) -> Option<&'static str> {
            Some("twitch")
        }

        fn performs_login(&self) -> bool {
            true
        }

        fn perform_login(
            &self,
            client: &Client,
            session: &super::login::Session,
            username: &str,
            password: &str,
        ) -> Result<()> {
            perform_login(client, session, username, password)
        }
    };
}

/// Upstream `live_from_start`: VOD formats are marked `is_from_start`, and a
/// live stream resolves to the VOD being recorded.
pub(crate) static LIVE_FROM_START: AtomicBool = AtomicBool::new(false);

static VOD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:(?:www|go|m)\.)?twitch\.tv/(?:[^/]+/v(?:ideo)?|videos)/|player\.twitch\.tv/\?.*?\bvideo=v?|www\.twitch\.tv/[^/]+/schedule\?vodID=)(?P<id>\d+)")
        .unwrap()
});
static CLIPS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:clips\.twitch\.tv/(?:embed\?.*?\bclip=|(?:[^/]+/)*)|(?:(?:www|go|m)\.)?twitch\.tv/(?:[^/]+/)?clip/)(?P<id>[^/?#&]+)")
        .unwrap()
});
static THUMBNAIL_SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+x\d+(\.\w+)($|[?#])").unwrap());
static OLD_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"%7C(\d+)(?:-\d+)?.mp4").unwrap());

/// `TwitchVodIE.suitable`.
pub fn vod_suitable(url: &str) -> bool {
    VOD_URL.is_match(url)
}

/// `TwitchClipsIE.suitable`.
pub fn clips_suitable(url: &str) -> bool {
    CLIPS_URL.is_match(url)
}

static COLLECTION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|go|m)\.)?twitch\.tv/collections/(?P<id>[^/]+)").unwrap()
});
static VIDEOS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|go|m)\.)?twitch\.tv/(?P<id>[^/]+)/(?:videos|profile)").unwrap()
});
static VIDEOS_CLIPS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|go|m)\.)?twitch\.tv/(?P<id>[^/]+)/(?:clips|videos/*?\?.*?\bfilter=clips)")
        .unwrap()
});
static VIDEOS_COLLECTIONS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www|go|m)\.)?twitch\.tv/(?P<id>[^/]+)/videos/*?\?.*?\bfilter=collections",
    )
    .unwrap()
});
static STREAM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:(?:www|go|m)\.)?twitch\.tv/|player\.twitch\.tv/\?.*?\bchannel=)(?P<id>[^/#?]+)")
        .unwrap()
});

/// `TwitchCollectionIE.suitable`.
pub fn collection_suitable(url: &str) -> bool {
    COLLECTION_URL.is_match(url)
}

/// `TwitchVideosClipsIE.suitable`.
pub fn videos_clips_suitable(url: &str) -> bool {
    VIDEOS_CLIPS_URL.is_match(url)
}

/// `TwitchVideosCollectionsIE.suitable`.
pub fn videos_collections_suitable(url: &str) -> bool {
    VIDEOS_COLLECTIONS_URL.is_match(url)
}

/// `TwitchVideosIE.suitable`: not a clips or collections list.
pub fn videos_suitable(url: &str) -> bool {
    !videos_clips_suitable(url) && !videos_collections_suitable(url) && VIDEOS_URL.is_match(url)
}

/// `TwitchStreamIE.suitable`: no other Twitch extractor's URL.
pub fn stream_suitable(url: &str) -> bool {
    !(vod_suitable(url)
        || collection_suitable(url)
        || videos_suitable(url)
        || videos_clips_suitable(url)
        || videos_collections_suitable(url)
        || clips_suitable(url))
        && STREAM_URL.is_match(url)
}

fn operation_hash(name: &str) -> &'static str {
    match name {
        "CollectionSideBar" => "016e1e4ccee0eb4698eb3bf1a04dc1c077fb746c78c82bac9a8f0289658fbd1a",
        "FilterableVideoTower_Videos" => {
            "67004f7881e65c297936f32c75246470629557a393788fb5a69d6d9a25a8fd5f"
        }
        "ClipsCards__User" => "1cd671bfa12cec480499c087319f26d21925e9695d1f80225aae6a4354f23088",
        "ChannelCollectionsContent" => {
            "5247910a19b1cd2b760939bf4cba4dcbd3d13bdf8c266decd16956f6ef814077"
        }
        "StreamMetadata" => "ad022ca32220d5523d03a23cbcb5beaa1e0999889c1f8f78f9f2520dafb5cae6",
        "ComscoreStreamingQuery" => {
            "e1edae8122517d013405f237ffcc124515dc6ded82480a88daef69c83b53ac01"
        }
        "VideoPreviewOverlay" => "9515480dee68a77e667cb19de634739d33f243572b007e98e67184b1a5d8369f",
        "ShareClipRenderStatus" => {
            "2db6a3b20eabf510bd3cf465ae2408834b59eb6b8af89ca73ab1486cacecfb63"
        }
        "VideoMetadata" => "45111672eea2e507f8ba44d101a61862f9c56b11dee09a15634cb75cb9b9084d",
        "VideoPlayer_ChapterSelectButtonVideo" => {
            "71835d5ef425e154bf282453a926d99b328cdc5e32f36d3a209d0f4778b41203"
        }
        "VideoPlayer_VODSeekbarPreviewVideo" => {
            "07e99e4d56c5a7c67117a154777b0baf85a5ffefa393b213f4bc712ccaf85dd6"
        }
        _ => "",
    }
}

fn int(value: Option<&Value>, scale: i64) -> Option<i64> {
    let value = value?;
    let number = value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())?;
    Some((number / scale as f64) as i64)
}

fn fail(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{name}] {id}: {message}"))
}

/// `_CLIENT_ID`: the `twitch:client_id` extractor argument (read with
/// `ie_key='Twitch'` for every Twitch extractor, case-sensitively), else the
/// web client's.
fn client_id() -> String {
    crate::manifest_args::configuration_arg("twitch", "client_id", true)
        .and_then(|values| values.into_iter().next())
        .unwrap_or_else(|| CLIENT_ID.to_owned())
}

/// `_download_base_gql(video_id, ops, note)`.
fn base_gql(
    client: &Client,
    name: &'static str,
    id: &str,
    ops: &Value,
    note: &str,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{name}] {id}: {note}"));
    let mut headers = vec![
        ("Content-Type", "text/plain;charset=UTF-8".to_owned()),
        ("Client-ID", client_id()),
    ];
    if let Some(token) = crate::cookies::get_cookie("https://gql.twitch.tv", "auth-token") {
        headers.push(("Authorization", format!("OAuth {token}")));
    }
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let url = Url::parse("https://gql.twitch.tv/gql")?;
    let body = crate::output::python_json_dumps(ops);
    let mut response =
        super::direct::post(client, &url, &headers, body).map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: name,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            },
            error => error,
        })?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    serde_json::from_slice(&body).map_err(|error| {
        Error::Reported(format!(
            "[{name}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_download_gql(video_id, ops, note)`: persisted operations.
fn gql(
    client: &Client,
    name: &'static str,
    id: &str,
    ops: Vec<Value>,
    note: &str,
) -> Result<Value> {
    let ops: Vec<Value> = ops
        .into_iter()
        .map(|mut op| {
            let hash = operation_hash(op["operationName"].as_str().unwrap_or_default());
            if let Some(object) = op.as_object_mut() {
                object.insert(
                    "extensions".into(),
                    json!({"persistedQuery": {"version": 1, "sha256Hash": hash}}),
                );
            }
            op
        })
        .collect();
    base_gql(client, name, id, &Value::Array(ops), note)
}

/// A format from upstream's dict (`dict:KEYS` key order).
fn format_from(mut format: Map<String, Value>, id: &str, name: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = format.keys().cloned().collect();
    for key in ["height", "width", "fps", "aspect_ratio"] {
        if format.get(key).is_some_and(Value::is_null) {
            format.shift_remove(key);
        }
    }
    if !format.contains_key("ext") {
        let ext = format
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext)
            .unwrap_or_else(|| "unknown_video".into());
        format.insert("ext".into(), ext.into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(name, id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_rates(&mut format);
    Ok(format)
}

/// A video from upstream's dict (`dict;KEYS` order).
fn video(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    name: &str,
    key: &str,
    thumbnail_keys: &str,
) -> VideoInfo {
    let first = formats
        .first()
        .map_or_else(|| url.clone(), |format| format.url.clone());
    let mut info = super::direct::build_at(first.clone(), &first, String::new());
    info.direct = None;
    if formats.is_empty() {
        info.extra
            .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
    }
    info.formats = formats;
    info.webpage_url = url.clone();
    info.extractor = name.into();
    info.extractor_key = key.into();
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
    crate::info_order::mark_info(
        info,
        &format!(
            "dict;{};chapter=start_time,end_time,title{thumbnail_keys}",
            keys.join(",")
        ),
    )
}

/// `_get_thumbnails(thumbnail)`: the full-size image first.
fn thumbnails(thumbnail: Option<&str>) -> Value {
    match thumbnail {
        Some(thumbnail) => json!([
            {"url": THUMBNAIL_SIZE.replace(thumbnail, "0x0$1$2").into_owned(), "preference": 1},
            {"url": thumbnail},
        ]),
        None => Value::Null,
    }
}

/// `_download_access_token(video_id, token_kind, param_name)`.
fn access_token(
    client: &Client,
    name: &'static str,
    id: &str,
    kind: &str,
    param: &str,
) -> Result<Value> {
    let method = format!("{kind}PlaybackAccessToken");
    let query = format!(
        "{{\n              {method}(\n                {param}: \"{id}\",\n                params: {{\n                  platform: \"web\",\n                  playerBackend: \"mediaplayer\",\n                  playerType: \"site\"\n                }}\n              )\n              {{\n                value\n                signature\n              }}\n            }}"
    );
    let token = base_gql(
        client,
        name,
        id,
        &json!({ "query": query }),
        &format!("Downloading {kind} access token GraphQL"),
    )?;
    Ok(token
        .pointer(&format!("/data/{method}"))
        .cloned()
        .unwrap_or(Value::Null))
}

/// `_extract_twitch_m3u8_formats(path, video_id, token, signature,
/// live_from_start)`: the usher master's formats; a `403` JSON error names
/// subscriber-only content or the usher's error.
#[allow(clippy::too_many_arguments)]
fn usher_formats(
    client: &Client,
    name: &'static str,
    temp_id: &str,
    path: &str,
    id: &str,
    access: &Value,
    live_from_start: bool,
    live: bool,
) -> Result<Vec<crate::VideoFormat>> {
    use std::io::Read;
    let random = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1_000_000, |time| {
            1_000_000 + (time.subsec_nanos() as u64 % 9_000_000)
        });
    let text = |key: &str| {
        access
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut usher = Url::parse(&format!("https://usher.ttvnw.net/{path}/{id}.m3u8"))?;
    usher
        .query_pairs_mut()
        .append_pair("allow_source", "true")
        .append_pair("allow_audio_only", "true")
        .append_pair("allow_spectre", "true")
        .append_pair("p", &random.to_string())
        .append_pair("platform", "web")
        .append_pair("player", "twitchweb")
        .append_pair("supported_codecs", "av1,h265,h264")
        .append_pair("playlist_include_framerate", "true")
        .append_pair("sig", &text("signature"))
        .append_pair("token", &text("value"));
    // `_extract_m3u8_formats(..., fatal=True)`, keeping a `403` response.
    let note = format!("[{name}] {id}: Downloading m3u8 information");
    crate::screen::expect_page(id, Some(note.clone()), None);
    let response = super::direct::request_expecting(client, &usher, &[], &[403]);
    if response.is_err() {
        crate::screen::screen(&note);
    }
    let failed = |error: Error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::Reported(
            super::request_error_text(name, id, "m3u8 information", &error).replacen(
                ": Unable to download ",
                ": Failed to download ",
                1,
            ),
        ),
        error => error,
    };
    let parsed = match response {
        Ok(mut response) if response.status().as_u16() == 403 => {
            let json = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .is_some_and(|value| value.as_bytes() == b"application/json");
            if !json {
                Err(crate::retry::check_response(response).map_or_else(failed, |_| {
                    Error::Reported(format!(
                        "[{name}] {id}: Failed to download m3u8 information: HTTP Error 403: Forbidden (caused by <HTTPError 403: Forbidden>)"
                    ))
                }))
            } else {
                let mut body = Vec::new();
                response
                    .read_to_end(&mut body)
                    .map_err(Error::ExtractionRead)?;
                let info = serde_json::from_slice::<Value>(&body)
                    .ok()
                    .and_then(|data| data.get(0).filter(|info| info.is_object()).cloned())
                    .unwrap_or_else(|| json!({}));
                let code = info.get("error_code");
                let restricted = code.and_then(Value::as_str).is_some_and(|code| {
                    matches!(
                        code,
                        "vod_manifest_restricted" | "unauthorized_entitlements"
                    )
                });
                let common = "access to this subscriber-only content";
                let message = if restricted {
                    if crate::cookies::get_cookie("https://gql.twitch.tv", "auth-token").is_some() {
                        format!("Your account does not have {common}")
                    } else {
                        format!(
                            "You must be logged into an account that has {common}. {LOGIN_HINT}"
                        )
                    }
                } else {
                    // `join_nonempty('error_code', 'error', from_dict=..., delim=': ')`.
                    ["error_code", "error"]
                        .iter()
                        .filter_map(|key| info.get(*key).filter(|value| truthy(value)))
                        .map(|value| match value {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(": ")
                };
                Err(if message.is_empty() {
                    Error::Reported(format!(
                        "[{name}] {id}: Failed to download m3u8 information: HTTP Error 403: Forbidden (caused by <HTTPError 403: Forbidden>)"
                    ))
                } else {
                    fail(name, temp_id, &message)
                })
            }
        }
        // A live master's variants are not read (upstream never reads them;
        // their fragments change): live formats download through FFmpeg.
        Ok(response) => {
            super::hls::from_response_with(client, response, &usher, None, true, None, live)
        }
        Err(error) => Err(failed(error)),
    };
    crate::screen::clear_page();
    let stream = parsed?.ok_or_else(|| {
        Error::UnsupportedHtml5(format!("the {name} stream is not an HLS playlist"))
    })?;
    let mut formats = stream.formats;
    for format in &mut formats {
        if format
            .vcodec
            .as_deref()
            .is_some_and(|codec| codec.starts_with("av01"))
        {
            // mpegts does not yet have proper support for av1.
            downloader_option(format, "ffmpeg_args_out", json!(["-f", "mp4"]));
        }
        if live_from_start {
            downloader_option(format, "ffmpeg_args", json!(["-live_start_index", "0"]));
            format.extra.insert("is_from_start".into(), true.into());
        }
    }
    Ok(formats)
}

/// `fmt.setdefault('downloader_options', {}).update({key: value})`.
fn downloader_option(format: &mut crate::VideoFormat, key: &str, value: Value) {
    format
        .downloader_options
        .get_or_insert_with(Default::default)
        .extra
        .insert(key.into(), value);
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// `_prefer_source(formats)`.
fn prefer_source(formats: &mut [crate::VideoFormat]) {
    if let Some(source) = formats
        .iter_mut()
        .find(|format| format.format_id == "Source")
    {
        source.quality = Some(10.0);
        crate::info_order::mark_format_update(source, "quality");
        crate::info_order::mark_int_rates(source);
    } else {
        for format in formats
            .iter_mut()
            .filter(|format| format.url.as_str().contains("/chunked/"))
        {
            format.quality = Some(10.0);
            format.format_note = Some("Source".into());
            crate::info_order::mark_format_update(format, "quality");
            crate::info_order::mark_format_update(format, "format_note");
            crate::info_order::mark_int_rates(format);
        }
    }
}

impl Extractor for TwitchVod {
    twitch_login!();

    fn key(&self) -> &'static str {
        VOD_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VOD_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let vod_id = found["id"].to_owned();
        crate::screen::set_extractor(VOD_NAME);
        self.initialize(client, VOD_NAME, &vod_id)?;
        crate::screen::screen(&format!(
            "[{VOD_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_download_info`.
        let data = gql(
            client,
            VOD_NAME,
            &vod_id,
            vec![
                json!({"operationName": "VideoMetadata", "variables": {"channelLogin": "", "videoID": vod_id}}),
                json!({"operationName": "VideoPlayer_ChapterSelectButtonVideo", "variables": {"includePrivate": false, "videoID": vod_id}}),
                json!({"operationName": "VideoPlayer_VODSeekbarPreviewVideo", "variables": {"includePrivate": false, "videoID": vod_id}}),
            ],
            "Downloading stream metadata GraphQL",
        )?;
        let responses: Vec<&Value> = data
            .as_array()
            .map(|items| items.iter().collect())
            .unwrap_or_default();
        let video = responses
            .iter()
            .find_map(|item| item.pointer("/data/video").filter(|video| !video.is_null()))
            .cloned()
            .ok_or_else(|| fail(VOD_NAME, &vod_id, &format!("Video {vod_id} does not exist")))?;
        let moments: Vec<Value> = responses
            .iter()
            .filter_map(|item| item.pointer("/data/video/moments/edges"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(|edge| edge.get("node").cloned())
            .collect();
        let storyboard = responses
            .iter()
            .filter_map(|item| item.pointer("/data/video/seekPreviewsURL"))
            .filter_map(Value::as_str)
            .find_map(super::json_ld::url_or_none);
        // `_extract_info_gql`.
        let mut id = video
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .unwrap_or(&vod_id)
            .to_owned();
        if !id.starts_with('v') {
            id = format!("v{id}");
        }
        let mut thumbnail = video
            .get("previewThumbnailURL")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let mut is_live = Value::Null;
        if let Some(found) = &thumbnail {
            static PROCESSING: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r"/404_processing_[^.?#]+\.png").unwrap());
            if PROCESSING.is_match(found) {
                is_live =
                    (video.get("broadcastType").and_then(Value::as_str) == Some("ARCHIVE")).into();
                thumbnail = None;
            } else {
                is_live = false.into();
            }
        }
        let duration = int(video.get("lengthSeconds"), 1);
        let mut chapters = Vec::new();
        if moments.is_empty() {
            if let Some(game) = video
                .pointer("/game/displayName")
                .filter(|game| !game.is_null())
            {
                chapters.push(json!({"title": game}));
            }
        } else {
            for moment in &moments {
                let start = int(moment.get("positionMilliseconds"), 1000);
                let length = int(moment.get("durationMilliseconds"), 1000);
                let name = moment.get("description").and_then(Value::as_str);
                let (Some(start), Some(length)) = (start, length) else {
                    crate::screen::warning(&format!(
                        "[{VOD_NAME}] {vod_id}: Important chapter information missing for chapter {}",
                        name.unwrap_or("None")
                    ));
                    continue;
                };
                chapters
                    .push(json!({"start_time": start, "end_time": start + length, "title": name}));
            }
        }
        let mut info = Map::new();
        info.insert("id".into(), id.into());
        info.insert(
            "title".into(),
            video
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| !title.is_empty())
                .unwrap_or("Untitled Broadcast")
                .into(),
        );
        info.insert(
            "description".into(),
            video.get("description").cloned().unwrap_or(Value::Null),
        );
        info.insert("duration".into(), duration.into());
        info.insert("thumbnails".into(), thumbnails(thumbnail.as_deref()));
        info.insert(
            "uploader".into(),
            video
                .pointer("/owner/displayName")
                .filter(|v| v.is_string())
                .cloned()
                .unwrap_or(Value::Null),
        );
        info.insert(
            "uploader_id".into(),
            video
                .pointer("/owner/login")
                .filter(|v| v.is_string())
                .cloned()
                .unwrap_or(Value::Null),
        );
        info.insert(
            "timestamp".into(),
            video
                .get("publishedAt")
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_timestamp(date, false))
                .into(),
        );
        info.insert("view_count".into(), int(video.get("viewCount"), 1).into());
        info.insert("chapters".into(), Value::Array(chapters));
        // A VOD still recording (`is_live`) has a growing playlist.
        let recording = is_live == Value::Bool(true);
        info.insert("is_live".into(), is_live);
        info.insert("was_live".into(), true.into());
        let access = access_token(client, VOD_NAME, &vod_id, "video", "id")?;
        let mut formats = usher_formats(
            client,
            VOD_NAME,
            &vod_id,
            "vod",
            &vod_id,
            &access,
            LIVE_FROM_START.load(Ordering::Relaxed),
            recording,
        )?;
        // `_extract_storyboard`.
        if let (Some(duration), Some(storyboard)) = (duration.filter(|d| *d != 0), &storyboard) {
            let spec = Url::parse(storyboard)
                .ok()
                .and_then(|spec| {
                    super::download_json_note(
                        client,
                        &spec,
                        VOD_NAME,
                        &vod_id,
                        "Downloading storyboard metadata JSON",
                    )
                    .ok()
                })
                .and_then(|spec| spec.as_array().cloned())
                .unwrap_or_default();
            let mut spec = spec;
            spec.sort_by_key(|entry| std::cmp::Reverse(int(entry.get("width"), 1).unwrap_or(0)));
            let base = Url::parse(storyboard)?.join(".")?;
            for (index, entry) in spec.iter().enumerate() {
                let count = int(entry.get("count"), 1).filter(|count| *count != 0);
                let images: Vec<&str> = entry
                    .get("images")
                    .and_then(Value::as_array)
                    .map(|images| images.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                let (Some(count), false) = (count, images.is_empty()) else {
                    continue;
                };
                let fragment_duration = duration as f64 / images.len() as f64;
                let mut format = Map::new();
                format.insert("format_id".into(), format!("sb{index}").into());
                format.insert("format_note".into(), "storyboard".into());
                format.insert("ext".into(), "mhtml".into());
                format.insert("protocol".into(), "mhtml".into());
                format.insert("acodec".into(), "none".into());
                format.insert("vcodec".into(), "none".into());
                format.insert("url".into(), base.join(images[0])?.as_str().into());
                format.insert("width".into(), int(entry.get("width"), 1).into());
                format.insert("height".into(), int(entry.get("height"), 1).into());
                format.insert("fps".into(), json!(count as f64 / duration as f64));
                format.insert("rows".into(), int(entry.get("rows"), 1).into());
                format.insert("columns".into(), int(entry.get("cols"), 1).into());
                let fragments: Vec<Value> = images
                    .iter()
                    .filter_map(|path| base.join(path).ok())
                    .map(|url| json!({"url": url.as_str(), "duration": fragment_duration}))
                    .collect();
                format.insert("fragments".into(), Value::Array(fragments));
                formats.push(format_from(format, &vod_id, VOD_NAME)?);
            }
        }
        prefer_source(&mut formats);
        info.insert("formats".into(), json!([]));
        if let Some(t) = url
            .query_pairs()
            .find(|(key, value)| key == "t" && !value.is_empty())
            .map(|(_, value)| value.into_owned())
            && let Some(start) = crate::parse_duration(&t)
        {
            info.insert(
                "start_time".into(),
                match start {
                    crate::info::PyNumber::Int(value) => value.into(),
                    crate::info::PyNumber::Float(value) => json!(value),
                },
            );
        }
        Ok(Some(self::video(url, info, formats, VOD_NAME, VOD_KEY, "")))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

impl Extractor for TwitchClips {
    twitch_login!();

    fn key(&self) -> &'static str {
        CLIPS_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = CLIPS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let slug = found["id"].to_owned();
        crate::screen::set_extractor(CLIPS_NAME);
        self.initialize(client, CLIPS_NAME, &slug)?;
        crate::screen::screen(&format!(
            "[{CLIPS_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let data = gql(
            client,
            CLIPS_NAME,
            &slug,
            vec![json!({"operationName": "ShareClipRenderStatus", "variables": {"slug": slug}})],
            "Downloading clip GraphQL",
        )?;
        let clip = data
            .pointer("/0/data/clip")
            .filter(|clip| !clip.is_null())
            .cloned()
            .ok_or_else(|| fail(CLIPS_NAME, &slug, "This clip is no longer available"))?;
        let signature = clip
            .pointer("/playbackAccessToken/signature")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let token = clip
            .pointer("/playbackAccessToken/value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let signed = |source: &str| -> Option<String> {
            Url::parse(source).ok().map(|url| {
                super::update_url_query(&url, &[("sig", &signature), ("token", &token)]).to_string()
            })
        };
        let asset = |index: usize| {
            clip.pointer(&format!("/assets/{index}"))
                .filter(|asset| asset.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}))
        };
        let (default, portrait) = (asset(0), asset(1));
        let float = |value: Option<&Value>| {
            value.and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
        };
        let qualities = |asset: &Value| -> Vec<Value> {
            asset
                .get("videoQualities")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|quality| {
                    quality
                        .get("sourceURL")
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                        .is_some()
                })
                .cloned()
                .collect()
        };
        let mut formats = Vec::new();
        let default_ratio = float(default.get("aspectRatio"));
        for quality in qualities(&default) {
            let mut format = Map::new();
            format.insert(
                "url".into(),
                signed(quality["sourceURL"].as_str().unwrap_or_default()).into(),
            );
            if let Some(id) = quality.get("quality").and_then(Value::as_str) {
                format.insert("format_id".into(), id.into());
            }
            if let Some(height) = int(quality.get("quality"), 1) {
                format.insert("height".into(), height.into());
            }
            if let Some(fps) = float(quality.get("frameRate")) {
                format.insert("fps".into(), json!(fps));
            }
            if let Some(ratio) = default_ratio {
                format.insert("aspect_ratio".into(), json!(ratio));
            }
            if !format.contains_key("format_id") {
                format.insert("format_id".into(), "".into());
            }
            formats.push(format_from(format, &slug, CLIPS_NAME)?);
        }
        let portrait_ratio = float(portrait.get("aspectRatio"));
        for quality in qualities(&portrait) {
            let mut format = Map::new();
            format.insert(
                "url".into(),
                signed(quality["sourceURL"].as_str().unwrap_or_default()).into(),
            );
            let label = quality
                .get("quality")
                .and_then(Value::as_str)
                .filter(|q| !q.is_empty());
            format.insert(
                "format_id".into(),
                label
                    .map_or_else(
                        || "portrait".to_owned(),
                        |label| format!("portrait-{label}"),
                    )
                    .into(),
            );
            format.insert("height".into(), int(quality.get("quality"), 1).into());
            format.insert(
                "fps".into(),
                float(quality.get("frameRate")).map_or(Value::Null, |fps| json!(fps)),
            );
            format.insert(
                "aspect_ratio".into(),
                portrait_ratio.map_or(Value::Null, |ratio| json!(ratio)),
            );
            format.insert("quality".into(), (-2).into());
            formats.push(format_from(format, &slug, CLIPS_NAME)?);
        }
        let mut thumbnails = Vec::new();
        let default_thumb = default
            .get("thumbnailURL")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        if let Some(url) = &default_thumb {
            thumbnails.push(json!({"id": "default", "url": url, "preference": 0}));
        }
        if let Some(url) = portrait
            .get("thumbnailURL")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            thumbnails.push(json!({"id": "portrait", "url": url, "preference": -1}));
        }
        if let Some(url) = clip
            .get("thumbnailURL")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .filter(|url| Some(url) != default_thumb.as_ref())
        {
            thumbnails.push(json!({"id": "small", "url": url, "preference": -2}));
        }
        let old_id = formats
            .last()
            .and_then(|format| OLD_ID.captures(format.url.as_str()))
            .map(|found| found[1].to_owned());
        let mut info = Map::new();
        info.insert(
            "id".into(),
            clip.get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .unwrap_or(&slug)
                .into(),
        );
        info.insert(
            "_old_archive_ids".into(),
            old_id.map_or(Value::Null, |old| json!([format!("twitchclips {old}")])),
        );
        info.insert("display_id".into(), slug.clone().into());
        info.insert("formats".into(), json!([]));
        info.insert("thumbnails".into(), Value::Array(thumbnails));
        let text = |path: &str| {
            clip.pointer(path)
                .filter(|value| value.is_string())
                .cloned()
        };
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value.filter(|value| !value.is_null()) {
                info.insert(key.to_owned(), value);
            }
        };
        put("title", text("/title"));
        put(
            "duration",
            int(clip.get("durationSeconds"), 1).map(Value::from),
        );
        put("view_count", int(clip.get("viewCount"), 1).map(Value::from));
        put(
            "timestamp",
            clip.get("createdAt")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map(Value::from),
        );
        let broadcaster = clip
            .pointer("/broadcaster/displayName")
            .and_then(Value::as_str);
        put(
            "creators",
            Some(json!(
                broadcaster
                    .filter(|name| !name.is_empty())
                    .into_iter()
                    .collect::<Vec<_>>()
            )),
        );
        put("channel", text("/broadcaster/displayName"));
        put("channel_id", text("/broadcaster/id"));
        put(
            "channel_follower_count",
            int(clip.pointer("/broadcaster/followers/totalCount"), 1).map(Value::from),
        );
        put(
            "channel_is_verified",
            clip.pointer("/broadcaster/isPartner")
                .filter(|value| value.is_boolean())
                .cloned(),
        );
        put("uploader", text("/curator/displayName"));
        put("uploader_id", text("/curator/id"));
        let game = clip
            .pointer("/game/displayName")
            .and_then(Value::as_str)
            .filter(|game| !game.is_empty());
        if let Some(game) = game {
            put("categories", Some(json!([game])));
        }
        Ok(Some(video(
            url,
            info,
            formats,
            CLIPS_NAME,
            CLIPS_KEY,
            ";thumbnail=id,url,preference",
        )))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

/// `initialize()` (logging in once per extractor) and the `Extracting URL`
/// line.
fn announce(
    extractor: &dyn Extractor,
    client: &Client,
    (name, id): (&'static str, &str),
    url: &Url,
) -> Result<()> {
    crate::screen::set_extractor(name);
    extractor.initialize(client, name, id)?;
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    Ok(())
}

/// `TwitchBaseIE._perform_login`: the login page's hidden inputs with the
/// credentials as JSON to the form's action (Twitch's error, else its
/// redirect page), and a two-factor token when that page asks for it.
fn perform_login(
    client: &Client,
    session: &super::login::Session,
    username: &str,
    password: &str,
) -> Result<()> {
    const LOGIN_FORM_URL: &str = "https://www.twitch.tv/login";
    let fail = |message: &str| session.error(&format!("Unable to login. Twitch said: {message}"));
    let login_step = |page: &super::login::Page,
                      note: &str,
                      data: &[(&str, &str)]|
     -> Result<Option<super::login::Page>> {
        let mut form = super::login::hidden_inputs(&page.text);
        for (key, value) in data {
            super::login::set_field(&mut form, key, value);
        }
        let post_url = POST_URL
            .captures(&page.text)
            .and_then(|found| found.get(1).or_else(|| found.get(2)))
            .map_or("https://passport.twitch.tv/login", |found| found.as_str());
        let post_url = page.url.join(post_url)?;
        let referer = page.url.to_string();
        let headers = [
            ("Referer", referer.as_str()),
            ("Origin", "https://www.twitch.tv"),
            ("Content-Type", "text/plain;charset=UTF-8"),
        ];
        let body = Value::Object(
            form.into_iter()
                .map(|(key, value)| (key, Value::String(value)))
                .collect(),
        );
        let answer = session.request(
            client,
            &post_url,
            (Some(note), "Unable to download JSON metadata"),
            &headers,
            Some(crate::output::python_json_dumps(&body)),
            &[400],
        )?;
        let response: Value = crate::json::from_str(&answer.text).map_err(|error| {
            session.error(&format!("Failed to parse JSON  (caused by {error})"))
        })?;
        // `dict_get(response, ('error', 'error_description', 'error_code'))`.
        let error = ["error", "error_description", "error_code"]
            .into_iter()
            .filter_map(|key| response.get(key))
            .find(|value| super::nuxt::truthy(value));
        if let Some(error) = error {
            return Err(fail(&match error {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            }));
        }
        if response
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("Authenticated successfully"))
        {
            return Ok(None);
        }
        let redirect = response
            .get("redirect")
            .filter(|value| super::nuxt::truthy(value))
            .or_else(|| response.get("redirect_path"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                session
                    .bug("An extractor error has occurred. (caused by KeyError('redirect_path'))")
            })?;
        let redirect = post_url.join(redirect)?;
        session
            .request(
                client,
                &redirect,
                (
                    Some("Downloading login redirect page"),
                    "Unable to download webpage",
                ),
                &headers,
                None,
                &[],
            )
            .map(Some)
    };
    let login_page = session.request(
        client,
        &Url::parse(LOGIN_FORM_URL)?,
        (Some("Downloading login page"), "Unable to download webpage"),
        &[],
        None,
        &[],
    )?;
    // Some TOR nodes and public proxies are blocked completely.
    if login_page.text.contains("blacklist_message") {
        return Err(fail(&crate::html::clean_html(&login_page.text)));
    }
    let Some(redirect_page) = login_step(
        &login_page,
        "Logging in",
        &[
            ("username", username),
            ("password", password),
            ("client_id", &client_id()),
        ],
    )?
    else {
        return Ok(());
    };
    if TWO_FACTOR.is_match(&redirect_page.text) {
        let token = session.tfa_info("two-factor authentication token")?;
        login_step(
            &redirect_page,
            "Submitting TFA token",
            &[("authy_token", &token), ("remember_2fa", "true")],
        )?;
    }
    Ok(())
}

static POST_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<form[^>]+action=(?:"(.+?)"|'(.+?)')"#).unwrap());
static TWO_FACTOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)<form[^>]+id="two-factor-submit""#).unwrap());

/// An unexpected response shape: upstream's uncaught Python error.
fn bug(name: &str, id: &str, cause: &str) -> Error {
    Error::Reported(format!(
        "[{name}] {id}: An extractor error has occurred. (caused by {cause}); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_download_gql(..., fatal=False)`: `None` after a warning.
fn gql_optional(
    client: &Client,
    name: &'static str,
    id: &str,
    ops: Vec<Value>,
    note: &str,
) -> Result<Option<Value>> {
    match gql(client, name, id, ops, note) {
        Ok(data) => Ok(Some(data)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{name}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(Error::Reported(message)) if message.contains("Failed to parse JSON") => {
            crate::screen::warning(&message);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// `float_or_none(value)`.
fn float(value: Option<&Value>) -> Value {
    value
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
        })
        .map_or(Value::Null, |number| json!(number))
}

fn timestamp_of(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_str)
        .and_then(|date| crate::dates::unified_timestamp(date, false))
}

fn raw(node: &Value, key: &str) -> Value {
    node.get(key).cloned().unwrap_or(Value::Null)
}

/// A listing's entry dict (`_extract_entry(node)`) for its `kind`.
fn listing_entry(kind: &str, node: &Value) -> Option<Map<String, Value>> {
    let text_id = node
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let mut entry = Map::new();
    match kind {
        // `_make_video_result(node)`.
        "video" => {
            let id = text_id?;
            entry.insert("_type".into(), "url_transparent".into());
            entry.insert("ie_key".into(), VOD_KEY.into());
            entry.insert("id".into(), format!("v{id}").into());
            entry.insert(
                "url".into(),
                format!("https://www.twitch.tv/videos/{id}").into(),
            );
            entry.insert("title".into(), raw(node, "title"));
            entry.insert("thumbnail".into(), raw(node, "previewThumbnailURL"));
            entry.insert("duration".into(), float(node.get("lengthSeconds")));
            entry.insert("view_count".into(), int(node.get("viewCount"), 1).into());
        }
        // `TwitchVideosClipsIE._extract_entry`.
        "clip" => {
            let url = node
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)?;
            entry.insert("_type".into(), "url_transparent".into());
            entry.insert("ie_key".into(), CLIPS_KEY.into());
            entry.insert("id".into(), raw(node, "id"));
            entry.insert("url".into(), url.into());
            entry.insert("title".into(), raw(node, "title"));
            entry.insert("thumbnail".into(), raw(node, "thumbnailURL"));
            entry.insert("duration".into(), float(node.get("durationSeconds")));
            entry.insert(
                "timestamp".into(),
                timestamp_of(node.get("createdAt")).into(),
            );
            entry.insert("view_count".into(), int(node.get("viewCount"), 1).into());
            entry.insert("language".into(), raw(node, "language"));
        }
        // `TwitchVideosCollectionsIE._extract_entry`.
        "collection" => {
            let id = text_id?;
            entry.insert("_type".into(), "url_transparent".into());
            entry.insert("ie_key".into(), COLLECTION_KEY.into());
            entry.insert("id".into(), id.into());
            entry.insert(
                "url".into(),
                format!("https://www.twitch.tv/collections/{id}").into(),
            );
            entry.insert("title".into(), raw(node, "title"));
            entry.insert("thumbnail".into(), raw(node, "thumbnailURL"));
            entry.insert("duration".into(), float(node.get("lengthSeconds")));
            entry.insert(
                "timestamp".into(),
                timestamp_of(node.get("updatedAt")).into(),
            );
            entry.insert("view_count".into(), int(node.get("viewCount"), 1).into());
        }
        // `TwitchStreamIE._extract_entry`.
        _ => {
            let id = text_id?;
            entry.insert("_type".into(), "url".into());
            entry.insert("ie_key".into(), VOD_KEY.into());
            entry.insert("id".into(), format!("v{id}").into());
            entry.insert(
                "url".into(),
                format!("https://www.twitch.tv/videos/{id}").into(),
            );
            entry.insert("title".into(), raw(node, "title"));
            entry.insert(
                "timestamp".into(),
                timestamp_of(node.get("publishedAt")).unwrap_or(0).into(),
            );
        }
    }
    Some(entry)
}

/// A `url_transparent` entry dict as a playlist entry, extracted (and
/// updated with its non-`None` fields) when processed.
fn transparent_entry(entry: Map<String, Value>) -> Result<VideoInfo> {
    let target = entry
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let url = Url::parse(&target)?;
    let ie_key = entry
        .get("ie_key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let extractor = match ie_key.as_str() {
        CLIPS_KEY => CLIPS_NAME,
        COLLECTION_KEY => COLLECTION_NAME,
        _ => VOD_NAME,
    };
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = match entry.get("id") {
        Some(Value::String(id)) => id.clone(),
        _ => String::new(),
    };
    info.title = entry
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    // `process_ie_result`: every non-`None` field but these is forced.
    let fields: Map<String, Value> = entry
        .iter()
        .filter(|(key, value)| {
            !matches!(key.as_str(), "_type" | "url" | "id" | "ie_key") && !value.is_null()
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": fields,
            "ie_key": ie_key,
            "extractor": extractor,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `TwitchPlaylistBaseIE._entries`' state: the listing and its next page.
#[allow(clippy::too_many_arguments)]
fn listing_state(
    name: &'static str,
    channel: &str,
    operation: &str,
    variables: Value,
    entries_key: &str,
    edge_kind: &str,
    node_kind: &str,
    page_limit: u64,
    kind: &str,
    error_id: Option<&str>,
) -> Value {
    json!({
        "kind": "twitch",
        "extractor": name,
        "channel": channel,
        "operation": operation,
        "variables": variables,
        "entries_key": entries_key,
        "edge_kind": edge_kind,
        "node_kind": node_kind,
        "page_limit": page_limit,
        "entry": kind,
        "error_id": error_id,
        "cursor": null,
        "page": 1,
        "done": false,
    })
}

fn static_name(name: &str) -> &'static str {
    match name {
        STREAM_NAME => STREAM_NAME,
        VIDEOS_CLIPS_NAME => VIDEOS_CLIPS_NAME,
        VIDEOS_COLLECTIONS_NAME => VIDEOS_COLLECTIONS_NAME,
        _ => VIDEOS_NAME,
    }
}

/// `_entries(channel_name, *args)`: GraphQL pages until `want` entries are
/// read, or the listing ends.
fn page_entries(
    client: &Client,
    state: &mut Value,
    want: Option<usize>,
    out: &mut Vec<Map<String, Value>>,
) -> Result<()> {
    let name = static_name(state["extractor"].as_str().unwrap_or_default());
    let channel = state["channel"].as_str().unwrap_or_default().to_owned();
    let text = |key: &str| state[key].as_str().unwrap_or_default().to_owned();
    let (operation, entries_key, edge_kind, node_kind, kind) = (
        text("operation"),
        text("entries_key"),
        text("edge_kind"),
        text("node_kind"),
        text("entry"),
    );
    loop {
        if state["done"] == true || want.is_some_and(|want| out.len() >= want) {
            return Ok(());
        }
        let page_num = state["page"].as_u64().unwrap_or(1);
        let mut variables = state["variables"].as_object().cloned().unwrap_or_default();
        variables.insert("limit".into(), state["page_limit"].clone());
        if let Some(cursor) = state["cursor"].as_str().filter(|cursor| !cursor.is_empty()) {
            variables.insert("cursor".into(), cursor.into());
        }
        let page = gql_optional(
            client,
            name,
            &channel,
            vec![json!({"operationName": operation, "variables": variables})],
            &format!("Downloading {node_kind}s GraphQL page {page_num}"),
        )?;
        // Avoid extracting random/unrelated entries when the channel doesn't exist.
        if page
            .as_ref()
            .and_then(|page| page.pointer("/0/data/user/id"))
            .and_then(Value::as_str)
            == Some("")
        {
            let message = format!("Channel \"{channel}\" not found");
            return Err(match state["error_id"].as_str() {
                Some(id) => fail(name, id, &message),
                None => Error::Reported(message),
            });
        }
        state["page"] = (page_num + 1).into();
        let edges = page
            .filter(truthy)
            .and_then(|page| {
                page.pointer(&format!("/0/data/user/{entries_key}/edges"))
                    .and_then(Value::as_array)
                    .cloned()
            })
            .unwrap_or_default();
        if edges.is_empty() {
            state["done"] = true.into();
            return Ok(());
        }
        let mut yielded = false;
        for edge in &edges {
            if edge.get("__typename").and_then(Value::as_str) != Some(edge_kind.as_str()) {
                continue;
            }
            let Some(node) = edge.get("node").filter(|node| node.is_object()) else {
                continue;
            };
            if node.get("__typename").and_then(Value::as_str) != Some(node_kind.as_str()) {
                continue;
            }
            if let Some(entry) = listing_entry(&kind, node) {
                state["cursor"] = raw(edge, "cursor");
                out.push(entry);
                yielded = true;
            }
        }
        // Upstream requests a page without entries' cursor again, endlessly.
        if !yielded
            || !state["cursor"]
                .as_str()
                .is_some_and(|cursor| !cursor.is_empty())
        {
            state["done"] = true.into();
        }
    }
}

/// A listing's pending pages (see [`crate::YoutubeDl::fetch_pending`]) until
/// `limit` entries: the remaining state unless read to its end.
pub(crate) fn fetch_pages(
    client: &Client,
    mut state: Value,
    limit: Option<usize>,
    out: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let mut found = Vec::new();
    let want = limit.map(|limit| limit.saturating_sub(out.len()));
    let result = page_entries(client, &mut state, want, &mut found);
    for entry in found {
        out.push(transparent_entry(entry)?);
    }
    result?;
    Ok((state["done"] != true).then_some(state))
}

/// `playlist_result(entries, playlist_id, playlist_title)`.
fn playlist(
    page: &Url,
    name: &'static str,
    key: &'static str,
    id: &str,
    title: Option<String>,
    entries: Vec<VideoInfo>,
    pending: Option<Value>,
) -> ExtractResult {
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "playlist-dict;id,title".into(),
    );
    if title.is_none() {
        extra.insert("title".into(), Value::Null);
    }
    ExtractResult::Playlist(PlaylistInfo {
        id: id.to_owned(),
        title: title.unwrap_or_default(),
        webpage_url: page.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: pending.map(|state| crate::PendingEntries { state }),
        entries,
        extra,
    })
}

/// `parse_qs(url)`'s first value of `key` (blank values are dropped).
fn query_value(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, value)| name == key && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// A channel listing whose pages are fetched as its entries are read.
#[allow(clippy::too_many_arguments)]
fn lazy_listing(
    page: &Url,
    name: &'static str,
    key: &'static str,
    channel: &str,
    title: String,
    operation: &str,
    variables: Value,
    entries_key: &str,
    edge_kind: &str,
    node_kind: &str,
    page_limit: u64,
    kind: &str,
) -> ExtractResult {
    let state = listing_state(
        name,
        channel,
        operation,
        variables,
        entries_key,
        edge_kind,
        node_kind,
        page_limit,
        kind,
        None,
    );
    playlist(
        page,
        name,
        key,
        channel,
        Some(title),
        Vec::new(),
        Some(state),
    )
}

impl Extractor for TwitchVideos {
    twitch_login!();

    fn key(&self) -> &'static str {
        VIDEOS_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !videos_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = VIDEOS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let channel = found["id"].to_owned();
        announce(self, client, (VIDEOS_NAME, &channel), url)?;
        let filter = query_value(url, "filter").unwrap_or_else(|| "all".into());
        let sort = query_value(url, "sort").unwrap_or_else(|| "time".into());
        let (broadcast_type, label) = match filter.as_str() {
            "archives" => (Some("ARCHIVE"), "Past Broadcasts"),
            "highlights" => (Some("HIGHLIGHT"), "Highlights"),
            "uploads" => (Some("UPLOAD"), "Uploads"),
            "past_premieres" => (Some("PAST_PREMIERE"), "Past Premieres"),
            _ => (None, "All Videos"),
        };
        let sorted_by = if sort == "views" { "Popular" } else { "Date" };
        Ok(Some(lazy_listing(
            url,
            VIDEOS_NAME,
            VIDEOS_KEY,
            &channel,
            format!("{channel} - {label} sorted by {sorted_by}"),
            "FilterableVideoTower_Videos",
            json!({
                "channelOwnerLogin": channel,
                "broadcastType": broadcast_type,
                "videoSort": sort.to_uppercase(),
            }),
            "videos",
            "VideoEdge",
            "Video",
            100,
            "video",
        )))
    }
}

impl Extractor for TwitchVideosClips {
    twitch_login!();

    fn key(&self) -> &'static str {
        VIDEOS_CLIPS_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VIDEOS_CLIPS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let channel = found["id"].to_owned();
        announce(self, client, (VIDEOS_CLIPS_NAME, &channel), url)?;
        let range = query_value(url, "range").unwrap_or_else(|| "7d".into());
        let (filter, label) = match range.as_str() {
            "24hr" => ("LAST_DAY", "Top 24H"),
            "30d" => ("LAST_MONTH", "Top 30D"),
            "all" => ("ALL_TIME", "Top All"),
            _ => ("LAST_WEEK", "Top 7D"),
        };
        // NB: values other than 20 result in skipped videos.
        Ok(Some(lazy_listing(
            url,
            VIDEOS_CLIPS_NAME,
            VIDEOS_CLIPS_KEY,
            &channel,
            format!("{channel} - Clips {label}"),
            "ClipsCards__User",
            json!({"login": channel, "criteria": {"filter": filter}}),
            "clips",
            "ClipEdge",
            "Clip",
            20,
            "clip",
        )))
    }
}

impl Extractor for TwitchVideosCollections {
    twitch_login!();

    fn key(&self) -> &'static str {
        VIDEOS_COLLECTIONS_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VIDEOS_COLLECTIONS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let channel = found["id"].to_owned();
        announce(self, client, (VIDEOS_COLLECTIONS_NAME, &channel), url)?;
        Ok(Some(lazy_listing(
            url,
            VIDEOS_COLLECTIONS_NAME,
            VIDEOS_COLLECTIONS_KEY,
            &channel,
            format!("{channel} - Collections"),
            "ChannelCollectionsContent",
            json!({ "ownerLogin": channel }),
            "collections",
            "CollectionsItemEdge",
            "Collection",
            100,
            "collection",
        )))
    }
}

impl Extractor for TwitchCollection {
    twitch_login!();

    fn key(&self) -> &'static str {
        COLLECTION_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = COLLECTION_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let collection_id = found["id"].to_owned();
        announce(self, client, (COLLECTION_NAME, &collection_id), url)?;
        let data = gql(
            client,
            COLLECTION_NAME,
            &collection_id,
            vec![
                json!({"operationName": "CollectionSideBar", "variables": {"collectionID": collection_id}}),
            ],
            "Downloading collection GraphQL",
        )?;
        let collection = data
            .pointer("/0/data/collection")
            .ok_or_else(|| bug(COLLECTION_NAME, &collection_id, "KeyError('collection')"))?;
        if !collection.is_object() {
            return Err(bug(
                COLLECTION_NAME,
                &collection_id,
                "AttributeError(\"'NoneType' object has no attribute 'get'\")",
            ));
        }
        let title = collection
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let edges = collection
            .pointer("/items/edges")
            .and_then(Value::as_array)
            .ok_or_else(|| bug(COLLECTION_NAME, &collection_id, "KeyError('items')"))?;
        let entries = edges
            .iter()
            .filter_map(|edge| edge.get("node").filter(|node| node.is_object()))
            .filter_map(|node| listing_entry("video", node))
            .map(transparent_entry)
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(playlist(
            url,
            COLLECTION_NAME,
            COLLECTION_KEY,
            &collection_id,
            title,
            entries,
            None,
        )))
    }
}

impl Extractor for TwitchStream {
    twitch_login!();

    fn key(&self) -> &'static str {
        STREAM_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !stream_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = STREAM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        let channel = temp_id.to_lowercase();
        announce(self, client, (STREAM_NAME, &temp_id), url)?;
        let data = gql(
            client,
            STREAM_NAME,
            &channel,
            vec![
                json!({"operationName": "StreamMetadata", "variables": {"channelLogin": channel, "includeIsDJ": true}}),
                json!({"operationName": "ComscoreStreamingQuery", "variables": {"channel": channel, "clipSlug": "", "isClip": false, "isLive": true, "isVodOrCollection": false, "vodID": ""}}),
                json!({"operationName": "VideoPreviewOverlay", "variables": {"login": channel}}),
            ],
            "Downloading stream GraphQL",
        )?;
        let user = data
            .pointer("/0/data/user")
            .filter(|user| truthy(user))
            .ok_or_else(|| fail(STREAM_NAME, &temp_id, &format!("{channel} does not exist")))?;
        let stream = user
            .get("stream")
            .filter(|stream| truthy(stream))
            .ok_or_else(|| fail(STREAM_NAME, &channel, "The channel is not currently live"))?;
        let timestamp = timestamp_of(stream.get("createdAt"));
        if LIVE_FROM_START.load(Ordering::Relaxed) {
            crate::screen::screen(&format!(
                "[{STREAM_NAME}] {channel}: Extracting VOD to download live from start"
            ));
            let mut state = listing_state(
                STREAM_NAME,
                &channel,
                "FilterableVideoTower_Videos",
                json!({"channelOwnerLogin": channel, "broadcastType": null, "videoSort": "TIME"}),
                "videos",
                "VideoEdge",
                "Video",
                1,
                "stream",
                Some(&temp_id),
            );
            let mut found = Vec::new();
            page_entries(client, &mut state, Some(1), &mut found)?;
            if let Some(mut entry) = found.into_iter().next() {
                let published = entry
                    .shift_remove("timestamp")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0);
                if timestamp.is_some_and(|timestamp| published >= timestamp) {
                    // A `url` result, extracted by `TwitchVodIE`.
                    let target = entry["url"].as_str().unwrap_or_default().to_owned();
                    let mut follow = super::direct::build_at(url.clone(), url, String::new());
                    follow.formats.clear();
                    follow.direct = None;
                    follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
                    return Ok(Some(ExtractResult::Video(follow)));
                }
            }
            crate::screen::warning(&format!(
                "[{STREAM_NAME}] {channel}: Unable to extract the VOD associated with this livestream"
            ));
        }
        let access = access_token(client, STREAM_NAME, &channel, "stream", "channelName")?;
        let stream_id = stream.get("id").filter(|id| truthy(id)).map_or_else(
            || channel.clone(),
            |id| match id {
                Value::String(id) => id.clone(),
                other => other.to_string(),
            },
        );
        let mut formats = usher_formats(
            client,
            STREAM_NAME,
            &temp_id,
            "api/channel/hls",
            &channel,
            &access,
            false,
            true,
        )?;
        prefer_source(&mut formats);
        let sq_user = data
            .pointer("/1/data/user")
            .filter(|user| user.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let uploader = raw(&sq_user, "displayName");
        let description = sq_user
            .pointer("/broadcastSettings/title")
            .filter(|title| title.is_string())
            .cloned()
            .unwrap_or(Value::Null);
        let thumbnail = data
            .pointer("/2/data/user/stream/previewImageURL")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let mut title = uploader
            .as_str()
            .filter(|uploader| !uploader.is_empty())
            .unwrap_or(&channel)
            .to_owned();
        let stream_type = stream.get("type").and_then(Value::as_str);
        if let Some(kind @ ("rerun" | "live")) = stream_type {
            title.push_str(&format!(" ({kind})"));
        }
        let mut info = Map::new();
        info.insert("id".into(), stream_id.into());
        info.insert("display_id".into(), channel.clone().into());
        info.insert("title".into(), title.into());
        info.insert("description".into(), description);
        info.insert("thumbnails".into(), thumbnails(thumbnail.as_deref()));
        info.insert("uploader".into(), uploader);
        info.insert("uploader_id".into(), channel.into());
        info.insert("timestamp".into(), timestamp.into());
        info.insert("view_count".into(), raw(stream, "viewers"));
        info.insert("formats".into(), json!([]));
        info.insert("is_live".into(), (stream_type == Some("live")).into());
        Ok(Some(ExtractResult::Video(video(
            url,
            info,
            formats,
            STREAM_NAME,
            STREAM_KEY,
            "",
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(vod_suitable(
            "http://www.twitch.tv/riotgames/v/6528877?t=5m10s"
        ));
        assert!(vod_suitable("https://www.twitch.tv/videos/6528877"));
        assert!(vod_suitable("https://player.twitch.tv/?video=480452374"));
        assert!(!vod_suitable("https://www.twitch.tv/riotgames"));
        assert!(clips_suitable(
            "https://clips.twitch.tv/FaintLightGullWholeWheat"
        ));
        assert!(clips_suitable(
            "https://www.twitch.tv/sergeynixon/clip/StormyThankfulSproutFutureMan"
        ));
        assert!(clips_suitable(
            "https://clips.twitch.tv/embed?clip=InquisitiveBreakableYogurtJebaited"
        ));
    }

    #[test]
    fn channel_and_stream_urls_follow_upstream_suitable() {
        for url in [
            "https://www.twitch.tv/spamfish/videos?filter=all",
            "https://www.twitch.tv/spamfish/videos/all",
            "https://m.twitch.tv/spamfish/videos/all",
            "https://www.twitch.tv/spamfish/videos",
            "https://www.twitch.tv/spamfish/profile",
        ] {
            assert!(videos_suitable(url), "{url}");
            assert!(!stream_suitable(url), "{url}");
        }
        for url in [
            "https://www.twitch.tv/vanillatv/clips?filter=clips&range=all",
            "https://www.twitch.tv/dota2ruhub/videos?filter=clips&range=7d",
        ] {
            assert!(videos_clips_suitable(url), "{url}");
            assert!(!videos_suitable(url), "{url}");
            assert!(!stream_suitable(url), "{url}");
        }
        let url = "https://www.twitch.tv/spamfish/videos?filter=collections";
        assert!(videos_collections_suitable(url));
        assert!(!videos_suitable(url));
        assert!(collection_suitable(
            "https://www.twitch.tv/collections/o9zZer3IQBhTJw"
        ));
        assert!(!stream_suitable(
            "https://www.twitch.tv/collections/o9zZer3IQBhTJw"
        ));
        for url in [
            "http://www.twitch.tv/shroomztv",
            "http://www.twitch.tv/miracle_doto#profile-0",
            "https://player.twitch.tv/?channel=lotsofs",
            "https://go.twitch.tv/food",
            "https://m.twitch.tv/food",
            "https://www.twitch.tv/monstercat",
        ] {
            assert!(stream_suitable(url), "{url}");
        }
        for url in [
            "https://www.twitch.tv/videos/6528877",
            "https://clips.twitch.tv/FaintLightGullWholeWheat",
            "https://www.twitch.tv/sergeynixon/clip/StormyThankfulSproutFutureMan",
        ] {
            assert!(!stream_suitable(url), "{url}");
        }
    }

    #[test]
    fn listing_entries_follow_upstream_entry_dicts() {
        let node = json!({"id": "123", "title": "T", "lengthSeconds": 60, "viewCount": "5"});
        assert_eq!(
            Value::Object(listing_entry("video", &node).unwrap()),
            json!({"_type": "url_transparent", "ie_key": "TwitchVod", "id": "v123",
                   "url": "https://www.twitch.tv/videos/123", "title": "T", "thumbnail": null,
                   "duration": 60.0, "view_count": 5})
        );
        assert!(listing_entry("video", &json!({"id": ""})).is_none());
        assert!(listing_entry("clip", &json!({"id": "1", "url": "not a url"})).is_none());
        let stream = listing_entry("stream", &json!({"id": "9"})).unwrap();
        assert_eq!(stream["_type"], "url");
        assert_eq!(stream["timestamp"], 0);
    }

    #[test]
    fn thumbnails_add_the_full_size_image() {
        assert_eq!(
            thumbnails(Some("https://x/a-320x180.jpg")),
            json!([{"url": "https://x/a-0x0.jpg", "preference": 1}, {"url": "https://x/a-320x180.jpg"}])
        );
    }
}
