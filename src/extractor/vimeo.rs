//! Upstream `VimeoIE` (`yt_dlp/extractor/vimeo.py`) without browser
//! impersonation: player pages' `playerConfig` and vimeo.com pages' clip page
//! config through `_parse_config` (progressive, per-CDN HLS/DASH formats, text
//! tracks, thumbnails, chapters and live status), and
//! `VimeoBaseInfoExtractor`'s login (the viewer's XSRF token posted to
//! `/log_in`, or a `vimeo` cookie) with the logged-in `web` API client
//! (`_call_videos_api` with the viewer's JWT, `_extract_from_api`,
//! `_parse_api_response`, video passwords) and original formats. The
//! `android` client (cached OAuth tokens only) is not ported.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct Vimeo;

pub(crate) const NAME: &str = "vimeo";
pub(crate) const KEY: &str = "Vimeo";
const QUALITIES: [&str; 3] = ["low", "medium", "high"];

static HOST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:(?:www|player)\.)?vimeo\.com/").unwrap());
static USER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^user(?P<id>[0-9]+)(?P<rest>[^#]*)(?:#.*)?$").unwrap());
/// `_VALID_URL` after its optional path prefix (`(?:(?!event/).*?/)??`).
static TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<q>(?:play_redirect_hls|moogaloop\.swf)\?clip_id=)?(?:videos?/)?(?P<id>[0-9]+)(?P<rest>[^#]*)(?:#.*)?$",
    )
    .unwrap()
});
static EXCLUDED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|player)\.)?vimeo\.com/(?:(?:channels|album|showcase)/[^/?#]+/?(?:$|[?#])|[^/]+/review/|ondemand/|(?:.*?/)??event/)")
        .unwrap()
});
static UNLISTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/(?P<hash>[\da-f]{10})/?(?:\?.*)?$").unwrap());
static PLAYER_CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:playerC|c)onfig\s*=").unwrap());
static PAGE_CONFIG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"vimeo\.(?:clip|vod_title)_page_config\s*=\s*(\{.+?\});").unwrap()
});
static VIMEO_CONFIG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"vimeo\.config\s*=\s*(?:(\{.+?\})|_extend\([^,]+,\s+(\{.+?\})\));").unwrap()
});
static SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/sep/video/").unwrap());
/// `_EMBED_REGEX` (each `(["\'])...\\1` as one alternative per quote).
static EMBED: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        r#"<iframe[^>]+?src=(?:"(?P<url>(?:https?:)?//player\.vimeo\.com/video/\d+.*?)"|'(?P<url2>(?:https?:)?//player\.vimeo\.com/video/\d+.*?)')"#,
        r#"<embed[^>]+?src=(?:"(?P<url>(?:https?:)?//(?:www\.)?vimeo\.com/moogaloop\.swf.+?)"|'(?P<url2>(?:https?:)?//(?:www\.)?vimeo\.com/moogaloop\.swf.+?)')"#,
        r#"<video[^>]+src=(?:"(?P<url>(?:https?:)?//(?:www\.)?vimeo\.com/[0-9]+)"|'(?P<url2>(?:https?:)?//(?:www\.)?vimeo\.com/[0-9]+)')"#,
    ]
    .map(|pattern| Regex::new(pattern).unwrap())
});
static AUDIO_QUALITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"audio-(low|medium|high)").unwrap());

/// The `(?(u)...)` branch: a `user` URL's trailing name.
fn user_rest(rest: &str) -> bool {
    let Some(name) = rest.strip_prefix('/') else {
        return false;
    };
    let end = name.find(['/', '?', '#']).unwrap_or(name.len());
    let (segment, after) = name.split_at(end);
    !segment.is_empty()
        && !segment.starts_with("videos")
        && !segment.starts_with("likes")
        && (after.is_empty() || after == "/" || after.starts_with('?') || after.starts_with("/?"))
}

/// `VimeoIE._match_valid_url`: the ID and any unlisted hash. The lazy
/// path prefix is tried at each `/` (shortest first) like Python's
/// backtracking, so `album/ID/video/ID` takes the video's ID.
fn match_url(url: &str) -> Option<(String, Option<String>)> {
    if EXCLUDED.is_match(url) {
        return None;
    }
    let path = &url[HOST.find(url)?.end()..];
    if let Some(found) = USER.captures(path)
        && user_rest(found.name("rest").map_or("", |rest| rest.as_str()))
    {
        return Some((found["id"].to_owned(), None));
    }
    let starts = std::iter::once(0).chain(path.match_indices('/').map(|(index, _)| index + 1));
    for start in starts {
        let Some(found) = TAIL.captures(&path[start..]) else {
            continue;
        };
        let rest = found.name("rest").map_or("", |rest| rest.as_str());
        let id = found["id"].to_owned();
        if found.name("q").is_some() {
            if rest.is_empty() || rest.starts_with('&') {
                return Some((id, None));
            }
            continue;
        }
        if let Some(hash) = UNLISTED.captures(rest) {
            return Some((id, Some(hash["hash"].to_owned())));
        }
        let tail = rest.strip_prefix('/').unwrap_or(rest);
        if tail.is_empty() || tail.starts_with('?') {
            return Some((id, None));
        }
    }
    None
}

/// `VimeoIE.suitable`.
pub fn suitable(url: &str) -> bool {
    match_url(url).is_some()
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    match_url(url).map(|(id, _)| id)
}

/// `VimeoIE._extract_embed_urls`: the `_EMBED_REGEX` embeds, each
/// smuggled with the page as its `referer` (`_smuggle_referrer`).
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    super::embed_regex_urls(url, webpage, &*EMBED, suitable)
        .into_iter()
        .filter_map(|embed| {
            let smuggled =
                super::brightcove::smuggle(embed.as_str(), &json!({"referer": url.as_str()}));
            Url::parse(&smuggled).ok()
        })
        .collect()
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// `_parse_config(config, video_id)`: the info dict and its formats.
fn parse_config(
    client: &Client,
    config: &Value,
    video_id: &str,
) -> Result<(
    Map<String, Value>,
    Vec<crate::VideoFormat>,
    crate::Subtitles,
)> {
    let video = config.get("video").cloned().unwrap_or_else(|| json!({}));
    let live_event = video
        .get("live_event")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let live_status = match live_event.get("status").and_then(Value::as_str) {
        Some("pending" | "active") => Some("is_upcoming"),
        Some("started") => Some("is_live"),
        Some("ended") => Some("post_live"),
        _ => None,
    };
    let is_live = live_status == Some("is_live");
    let request = config.get("request").cloned().unwrap_or_else(|| json!({}));
    let files = video
        .get("files")
        .filter(|files| files.as_object().is_some_and(|files| !files.is_empty()))
        .or_else(|| request.get("files"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for file in files
        .get("progressive")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(url) = file
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let quality = match file.get("quality") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Null) | None => "None".into(),
            Some(other) => other.to_string(),
        };
        let mut format = Map::new();
        format.insert("url".into(), url.into());
        format.insert("format_id".into(), format!("http-{quality}").into());
        format.insert("source_preference".into(), 10.into());
        for key in ["width", "height", "fps"] {
            format.insert(key.into(), int(file.get(key)).into());
        }
        format.insert("tbr".into(), int(file.get("bitrate")).into());
        let keys: Vec<String> = format.keys().cloned().collect();
        for key in ["width", "height", "fps", "tbr"] {
            if format.get(key).is_some_and(Value::is_null) {
                format.shift_remove(key);
            }
        }
        format.insert(
            "ext".into(),
            super::direct::determine_ext(url)
                .unwrap_or_else(|| "unknown_video".into())
                .into(),
        );
        let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
            .map_err(|error| fail(video_id, &error.to_string()))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    for files_type in ["hls", "dash"] {
        let Some(cdns) = files
            .get(files_type)
            .and_then(|kind| kind.get("cdns"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for (cdn_name, cdn) in cdns {
            let Some(manifest_url) = cdn
                .get("url")
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty())
            else {
                continue;
            };
            let format_id = format!("{files_type}-{cdn_name}");
            let urls: Vec<(String, String)> = if SEP.is_match(manifest_url) {
                [("", "video"), ("_sep", "sep/video")]
                    .iter()
                    .map(|(suffix, replacement)| {
                        (
                            format!("{format_id}{suffix}"),
                            SEP.replace_all(manifest_url, format!("/{replacement}/").as_str())
                                .into_owned(),
                        )
                    })
                    .collect()
            } else {
                vec![(format_id.clone(), manifest_url.to_owned())]
            };
            for (f_id, m_url) in urls {
                if files_type == "hls" {
                    let Some(stream) = Url::parse(&m_url).ok().and_then(|manifest| {
                        super::extract_m3u8_optional_note(
                            client,
                            &manifest,
                            NAME,
                            video_id,
                            Some(&f_id),
                            true,
                            is_live,
                            &format!("Downloading {cdn_name} m3u8 information"),
                        )
                    }) else {
                        continue;
                    };
                    for mut format in stream.formats {
                        if let Some(found) = AUDIO_QUALITY.captures(&format.format_id) {
                            let quality = QUALITIES
                                .iter()
                                .position(|quality| *quality == &found[1])
                                .map_or(-1, |index| index as i64);
                            format.quality = Some(quality as f64);
                            crate::info_order::mark_format_update(&mut format, "quality");
                            crate::info_order::mark_int_rates(&mut format);
                        }
                        formats.push(format);
                    }
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                } else {
                    let mut m_url = m_url;
                    if m_url.contains("json=1")
                        && let Some(real) = Url::parse(&m_url)
                            .ok()
                            .and_then(|json_url| {
                                super::download_json(client, &json_url, NAME, video_id).ok()
                            })
                            .and_then(|data| {
                                data.get("url").and_then(Value::as_str).map(str::to_owned)
                            })
                    {
                        m_url = real;
                    }
                    let Some(stream) = Url::parse(&m_url.replace("/master.json", "/master.mpd"))
                        .ok()
                        .and_then(|manifest| {
                            super::extract_mpd_optional_note(
                                client,
                                &manifest,
                                NAME,
                                video_id,
                                Some(&f_id),
                                &format!("Downloading {cdn_name} MPD information"),
                            )
                        })
                    else {
                        continue;
                    };
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            }
        }
    }
    if let Some(source) = live_event
        .pointer("/archive/source_url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        && live_event
            .pointer("/archive/status")
            .and_then(Value::as_str)
            == Some("done")
    {
        let mut format = Map::new();
        format.insert("format_id".into(), "live-archive-source".into());
        format.insert("url".into(), source.into());
        format.insert("quality".into(), 10.into());
        let keys: Vec<String> = format.keys().cloned().collect();
        format.insert(
            "ext".into(),
            super::direct::determine_ext(source)
                .unwrap_or_else(|| "unknown_video".into())
                .into(),
        );
        let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
            .map_err(|error| fail(video_id, &error.to_string()))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    for track in request
        .get("text_tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(lang), Some(path)) = (
            track.get("lang").and_then(Value::as_str),
            track.get("url").and_then(Value::as_str),
        ) else {
            continue;
        };
        let url = Url::parse("https://player.vimeo.com/")?.join(path)?;
        let track: crate::SubtitleFormat =
            serde_json::from_value(json!({"ext": "vtt", "url": url.as_str()}))
                .map_err(|error| fail(video_id, &error.to_string()))?;
        let language = Some(lang.to_owned());
        match subtitles
            .entries
            .iter_mut()
            .find(|(key, _)| *key == language)
        {
            Some((_, tracks)) => tracks.push(track),
            None => subtitles.entries.push((language, vec![track])),
        }
    }
    let mut thumbnails = Vec::new();
    if !is_live {
        for (key, thumb) in video
            .get("thumbs")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            thumbnails.push(json!({
                "id": key,
                "width": key.parse::<i64>().ok(),
                "url": thumb,
            }));
        }
        for key in ["thumbnail", "thumbnail_url"] {
            if let Some(url) = video
                .get(key)
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            {
                thumbnails.push(json!({"url": url}));
            }
        }
    }
    let owner = video.get("owner").cloned().unwrap_or_else(|| json!({}));
    let uploader_url = owner
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty());
    let mut chapters: Vec<Value> = config
        .pointer("/embed/chapters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|chapter| int(chapter.get("timecode")).is_some())
        .map(|chapter| {
            let mut found = Map::new();
            if let Some(title) = chapter.get("title").and_then(Value::as_str) {
                found.insert("title".into(), title.into());
            }
            found.insert("start_time".into(), int(chapter.get("timecode")).into());
            Value::Object(found)
        })
        .collect();
    chapters.sort_by_key(|chapter| int(chapter.get("start_time")).unwrap_or(0));
    let release_timestamp = live_event
        .pointer("/ingest/scheduled_start_time")
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .or_else(|| int(live_event.pointer("/ingest/start_time")));
    let id = match video.get("id") {
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => video_id.to_owned(),
    };
    let mut info = Map::new();
    info.insert("id".into(), id.into());
    info.insert(
        "title".into(),
        video.get("title").cloned().unwrap_or(Value::Null),
    );
    info.insert(
        "uploader".into(),
        owner.get("name").cloned().unwrap_or(Value::Null),
    );
    info.insert(
        "uploader_id".into(),
        uploader_url
            .map(|url| url.rsplit('/').next().unwrap_or_default().to_owned())
            .into(),
    );
    info.insert("uploader_url".into(), uploader_url.into());
    info.insert("thumbnails".into(), Value::Array(thumbnails));
    info.insert("duration".into(), int(video.get("duration")).into());
    info.insert(
        "chapters".into(),
        if chapters.is_empty() {
            Value::Null
        } else {
            Value::Array(chapters)
        },
    );
    info.insert("formats".into(), json!([]));
    info.insert("subtitles".into(), json!({}));
    info.insert("live_status".into(), live_status.into());
    info.insert("release_timestamp".into(), release_timestamp.into());
    info.insert(
        "_format_sort_fields".into(),
        json!(["quality", "res", "fps", "hdr:12", "source"]),
    );
    Ok((info, formats, subtitles))
}

/// `merge_dicts(a, b, ...)`.
fn merge_dicts(dicts: Vec<Map<String, Value>>) -> Map<String, Value> {
    let mut merged = Map::new();
    for dict in dicts {
        for (key, value) in dict {
            let replace = match merged.get(&key) {
                None => !value.is_null(),
                Some(Value::String(existing)) => existing.is_empty() && value.is_string(),
                Some(_) => false,
            };
            if replace {
                merged.insert(key, value);
            }
        }
    }
    merged
}

/// A video from upstream's dict (`dict;KEYS` order); `nested` is the info
/// shape's subtitle (and thumbnail) key orders.
fn video_keyed(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
    nested: &str,
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
    info.subtitles = subtitles;
    info.webpage_url = url.clone();
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
    let mut keys = Vec::new();
    let mut titled = false;
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => {
                titled = !value.is_null();
                info.title = value.as_str().unwrap_or_default().to_owned();
            }
            "formats" | "subtitles" | "webpage_url" => {}
            "_format_sort_fields" => {
                info.format_sort_fields = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
            }
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    if !titled {
        info.extra
            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
    }
    crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{};{nested};chapter=title,start_time",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    )
}

/// [`video_keyed`] for `_parse_config`'s subtitle tracks (`ext`, `url`).
fn video_info(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
) -> VideoInfo {
    video_keyed(url, dict, formats, subtitles, "subtitle=ext,url")
}

const LOGIN_URL: &str = "https://vimeo.com/log_in";
/// `_CLIENT_HEADERS`.
const CLIENT_HEADERS: [(&str, &str); 2] = [
    ("Accept", "application/vnd.vimeo.*+json; version=3.4.10"),
    ("Accept-Language", "en"),
];
/// `_CLIENT_CONFIGS`' clients, in order.
const CLIENTS: [&str; 2] = ["android", "web"];
/// `_CLIENT_CONFIGS['web']['VIDEOS_FIELDS']`.
const WEB_VIDEOS_FIELDS: &str = "config_url,created_time,description,license,metadata.connections.comments.total,metadata.connections.likes.total,release_time,stats.plays";
/// `_CLIENT_CONFIGS['android']['CACHE_KEY']`.
const ANDROID_CACHE_KEY: &str = "oauth-token-android";
const API_ERRNOTE: &str = "Unable to download web API JSON";

/// `_viewer_info`: the `/_next/viewer` response (web JWT and XSRF token).
static VIEWER: Mutex<Option<Value>> = Mutex::new(None);
/// `_DEFAULT_CLIENT`, which `_real_initialize` switches to a client with a
/// cached OAuth token.
static DEFAULT_CLIENT: Mutex<&str> = Mutex::new("web");

/// `_is_logged_in`: a `vimeo` cookie for vimeo.com.
fn is_logged_in() -> bool {
    crate::cookies::get_cookie("https://vimeo.com", "vimeo").is_some()
}

fn key_error(id: &str, key: &str) -> Error {
    fail(
        id,
        &format!(
            "An extractor error has occurred. (caused by KeyError('{key}')); {}",
            crate::BUG_REPORTS_MESSAGE
        ),
    )
}

/// An upstream request's response text, or its HTTP error with the error
/// body (`e.cause.response`).
enum Fetched {
    Page(String),
    Status {
        error: Error,
        status: u16,
        body: String,
    },
}

/// An upstream `_request_webpage` (a GET, or a POST of `data`): `[vimeo]
/// ID: NOTE` (`[vimeo] NOTE` without an ID), keeping HTTP error responses.
fn fetch(
    client: &Client,
    url: &Url,
    (id, note): (Option<&str>, &str),
    headers: &[(&str, &str)],
    data: Option<String>,
) -> Result<Fetched> {
    use std::io::Read;
    match id {
        Some(id) => crate::screen::screen(&format!("[{NAME}] {id}: {note}")),
        None => crate::screen::screen(&format!("[{NAME}] {note}")),
    }
    let errors: Vec<u16> = (400..600).collect();
    let mut response = match data {
        Some(body) => super::direct::post_expecting(client, url, headers, body, &errors)?,
        None => super::direct::request_expecting(client, url, headers, &errors)?,
    };
    let error = response.error_for_status_ref().err().map(Error::Http);
    let status = response.status().as_u16();
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    if let Some(error) = error {
        return Ok(Fetched::Status {
            error,
            status,
            body: text,
        });
    }
    crate::screen::expect_page(id.unwrap_or_default(), None, None);
    crate::screen::page_read(&page_url, &body)?;
    Ok(Fetched::Page(text))
}

/// `[vimeo] ID: ERRNOTE: HTTP Error ... (caused by ...)`.
fn request_failure(id: &str, errnote: &str, error: &Error) -> Error {
    let text = super::request_errnote_text(NAME, id, errnote, error);
    Error::Reported(if id.is_empty() {
        text.replacen("] : ", "] ", 1)
    } else {
        text
    })
}

fn parse_json(id: &str, text: &str) -> Result<Value> {
    crate::json::from_str(text).map_err(|error| {
        Error::Reported(
            format!("[{NAME}] {id}: Failed to parse JSON  (caused by {error})")
                .replacen("] : ", "] ", 1),
        )
    })
}

/// Upstream `_download_json(url, video_id, note, errnote, headers=...)`:
/// `id` is the note's (`None` for `video_id=None`), `error_id` the ID its
/// errors carry (the URL's temporary ID while logging in).
fn download_json(
    client: &Client,
    url: &Url,
    (id, error_id): (Option<&str>, &str),
    (note, errnote): (&str, &str),
    headers: &[(&str, &str)],
) -> Result<Value> {
    match fetch(client, url, (id, note), headers, None)? {
        Fetched::Page(text) => parse_json(error_id, &text),
        Fetched::Status { error, .. } => Err(request_failure(error_id, errnote, &error)),
    }
}

/// `_jwt_is_expired(token)`: less than two minutes left.
fn jwt_is_expired(id: &str, token: &str) -> Result<bool> {
    let exp = super::podbayfm::jwt_payload(token)
        .and_then(|payload| payload.get("exp").and_then(Value::as_f64))
        .ok_or_else(|| key_error(id, "exp"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    Ok(exp - now < 120.0)
}

/// `_fetch_viewer_info(display_id)`: the cached viewer unless its JWT
/// expires, else `https://vimeo.com/_next/viewer`.
fn fetch_viewer_info(client: &Client, display_id: Option<&str>, error_id: &str) -> Result<Value> {
    let cached = VIEWER.lock().unwrap().clone();
    if let Some(viewer) = cached {
        let jwt = viewer
            .get("jwt")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(error_id, "jwt"))?;
        if !jwt_is_expired(error_id, jwt)? {
            return Ok(viewer);
        }
    }
    let viewer = download_json(
        client,
        &Url::parse("https://vimeo.com/_next/viewer")?,
        (display_id, display_id.unwrap_or(error_id)),
        (
            "Downloading web token info",
            "Failed to download web token info",
        ),
        &[("Accept", "application/json")],
    )?;
    *VIEWER.lock().unwrap() = Some(viewer.clone());
    Ok(viewer)
}

/// `viewer[KEY]` (a missing key is upstream's `KeyError`).
fn viewer_field(viewer: &Value, key: &str, id: &str) -> Result<String> {
    match viewer.get(key) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Number(number)) => Ok(number.to_string()),
        _ => Err(key_error(id, key)),
    }
}

/// `_configuration_arg('client', [None], ie_key=VimeoIE)[0]`.
fn client_arg() -> Option<String> {
    crate::manifest_args::configuration_arg(KEY, "client", false)
        .and_then(|values| values.into_iter().next())
}

/// `_get_requested_client()`.
fn requested_client(id: &str) -> Result<String> {
    if let Some(client) = client_arg() {
        if !CLIENTS.contains(&client.as_str()) {
            return Err(fail(
                id,
                &format!(
                    "Unsupported API client \"{client}\" requested. Supported clients are: {}",
                    CLIENTS.join(", ")
                ),
            ));
        }
        crate::screen::extractor_debug_once(&format!(
            "Using {client} API client as specified by extractor argument"
        ));
        return Ok(client);
    }
    if is_logged_in() {
        return Ok("web".into());
    }
    Ok((*DEFAULT_CLIENT.lock().unwrap()).to_owned())
}

/// An API request's HTTP error: the error, its status and body.
type ApiFailure = (Error, u16, String);

/// `_call_videos_api(video_id, unlisted_hash, path, force_client=,
/// query=)`: the web client's request (`Authorization: jwt VIEWER_JWT`)
/// and its JSON, or its HTTP error. The android client is not ported:
/// without a cached token it is upstream's error, else an unsupported one.
fn call_videos_api(
    client: &Client,
    (video_id, unlisted_hash): (&str, Option<&str>),
    path: Option<&str>,
    force_client: Option<&str>,
    query: &[(&str, &str)],
) -> Result<std::result::Result<Value, ApiFailure>> {
    let api_client = match force_client {
        Some(name) => name.to_owned(),
        None => requested_client(video_id)?,
    };
    if api_client == "web" && !is_logged_in() {
        return Err(fail(
            video_id,
            &super::login::login_required_message(
                "The web client only works when logged-in",
                Some(NAME),
                None,
            ),
        ));
    }
    if api_client != "web" {
        let cached = crate::cache::load(NAME, ANDROID_CACHE_KEY, None)
            .is_some_and(|token| !token.is_null() && token != "" && token != false);
        if !cached {
            return Err(fail(
                video_id,
                &format!(
                    "The {api_client} client is unable to fetch new OAuth tokens and is only intended for use with previously cached tokens"
                ),
            ));
        }
        return Err(Error::UnsupportedHtml5(format!(
            "Vimeo's {api_client} API client is not supported natively"
        )));
    }
    let jwt = viewer_field(&fetch_viewer_info(client, None, video_id)?, "jwt", video_id)?;
    let authorization = format!("jwt {jwt}");
    let mut target = String::from("https://api.vimeo.com/videos/");
    target.push_str(video_id);
    if let Some(hash) = unlisted_hash.filter(|hash| !hash.is_empty()) {
        target.push(':');
        target.push_str(hash);
    }
    if let Some(path) = path {
        target.push('/');
        target.push_str(path);
    }
    let mut pairs: Vec<(&str, &str)> = vec![("fields", WEB_VIDEOS_FIELDS)];
    for (key, value) in query {
        match pairs.iter_mut().find(|(name, _)| name == key) {
            Some(pair) => pair.1 = value,
            None => pairs.push((key, value)),
        }
    }
    let url = Url::parse(&format!("{target}?{}", super::login::urlencode(&pairs)))?;
    let mut headers = vec![("Authorization", authorization.as_str())];
    headers.extend(CLIENT_HEADERS);
    match fetch(
        client,
        &url,
        (Some(video_id), "Downloading web API JSON"),
        &headers,
        None,
    )? {
        Fetched::Page(text) => Ok(Ok(parse_json(video_id, &text)?)),
        Fetched::Status {
            error,
            status,
            body,
        } => Ok(Err((error, status, body))),
    }
}

/// `_verify_video_password(video_id)`.
fn verify_video_password(client: &Client, video_id: &str) -> Result<()> {
    let Some(password) = super::video_password() else {
        return Err(fail(
            video_id,
            "This video is protected by a password, use the --video-password option",
        ));
    };
    let token = viewer_field(
        &fetch_viewer_info(client, Some(video_id), video_id)?,
        "xsrft",
        video_id,
    )?;
    let page = format!("https://vimeo.com/{video_id}");
    let body = serde_json::to_string(&json!({"password": password, "token": token}))
        .map_err(|error| fail(video_id, &error.to_string()))?;
    let impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
        crate::screen::warning_once(&format!("[{NAME}] {message}"));
    });
    let fetched = fetch(
        client,
        &Url::parse(&format!("{page}/password"))?,
        (Some(video_id), "Submitting video password"),
        &[
            ("Accept", "*/*"),
            ("Content-Type", "application/json"),
            ("Referer", &page),
        ],
        Some(body),
    )?;
    drop(impersonate);
    match fetched {
        Fetched::Page(_) => Ok(()),
        Fetched::Status { status: 418, .. } => Err(fail(video_id, "Wrong password")),
        Fetched::Status { error, .. } => Err(Error::ExtractorRequest {
            extractor: NAME,
            id: video_id.to_owned(),
            what: "webpage",
            source: Box::new(error),
        }),
    }
}

/// `_get_album_data_and_hashed_pass(album_id, False, None)` for a video in
/// an album or showcase: the album's privacy (through the viewer's JWT),
/// and its password posted (which sets cookies) when it has one.
fn album_data(client: &Client, album_id: &str, video_id: &str) -> Result<()> {
    let viewer = fetch_viewer_info(client, Some(album_id), album_id)?;
    let jwt = viewer_field(&viewer, "jwt", video_id)?;
    let authorization = format!("jwt {jwt}");
    let url = Url::parse(&format!(
        "https://api.vimeo.com/albums/{album_id}?{}",
        super::login::urlencode(&[
            ("is_embed", "false"),
            ("referrer", ""),
            ("fields", "description,name,privacy"),
        ])
    ))?;
    let album = download_json(
        client,
        &url,
        (Some(album_id), album_id),
        (
            "Downloading JSON metadata",
            "Unable to download JSON metadata",
        ),
        &[
            ("Authorization", &authorization),
            ("Accept", "application/json"),
        ],
    )?;
    if album.pointer("/privacy/view").and_then(Value::as_str) != Some("password") {
        return Ok(());
    }
    let Some(password) = super::video_password().filter(|password| !password.is_empty()) else {
        return Err(fail(
            video_id,
            "This album is protected by a password, use the --video-password option",
        ));
    };
    let token = viewer_field(&viewer, "xsrft", video_id)?;
    match fetch(
        client,
        &Url::parse(&format!("https://vimeo.com/showcase/{album_id}/auth"))?,
        (Some(album_id), "Verifying the password"),
        &[
            ("X-Requested-With", "XMLHttpRequest"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        Some(super::login::urlencode(&[
            ("password", &password),
            ("token", &token),
        ])),
    )? {
        Fetched::Page(text) => {
            if parse_json(album_id, &text)?.get("hashed_pass").is_none() {
                return Err(key_error(video_id, "hashed_pass"));
            }
            Ok(())
        }
        Fetched::Status { status: 401, .. } => Err(fail(video_id, "Wrong password")),
        Fetched::Status { error, .. } => Err(request_failure(
            album_id,
            "Unable to download JSON metadata",
            &error,
        )),
    }
}

/// `int_or_none` on a JSON value.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(flag) => Some(i64::from(*flag)),
        _ => None,
    }
}

/// `str(value)` of a `dict.get(key, default)` result.
fn py_str_or(value: Option<&Value>, default: &str) -> String {
    match value {
        None => default.to_owned(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) => "None".to_owned(),
        Some(other) => other.to_string(),
    }
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
        Some(_) => true,
    }
}

/// `_extract_original_format(url, video_id, unlisted_hash)`: a logged-in
/// account's source file (`vimeo:original_format_policy`): the download
/// config's `source_file`, else the web API's `source` download when the
/// privacy allows downloads.
fn original_format(
    client: &Client,
    video_id: &str,
    unlisted_hash: Option<&str>,
) -> Result<Option<crate::VideoFormat>> {
    if !is_logged_in() {
        return Ok(None);
    }
    let policy = crate::manifest_args::configuration_arg(KEY, "original_format_policy", false)
        .and_then(|values| values.into_iter().next())
        .unwrap_or_else(|| "auto".into());
    if policy == "never" {
        return Ok(None);
    }
    let mut query = vec![("action", "load_download_config")];
    if let Some(hash) = unlisted_hash {
        query.push(("unlisted_hash", hash));
    }
    let url = Url::parse(&format!(
        "https://vimeo.com/{video_id}?{}",
        super::login::urlencode(&query)
    ))?;
    let download_data = match fetch(
        client,
        &url,
        (Some(video_id), "Loading download config JSON"),
        &[
            ("Accept", "application/json"),
            ("X-Requested-With", "XMLHttpRequest"),
        ],
        None,
    )? {
        Fetched::Page(text) => match crate::json::from_str::<Value>(&text) {
            Ok(value) => Some(value),
            Err(error) => {
                crate::screen::extractor_debug(&format!(
                    "Unable to load download config JSON: {error}"
                ));
                None
            }
        },
        Fetched::Status { error, .. } => {
            crate::screen::extractor_debug(&format!(
                "Unable to load download config JSON: {}",
                crate::upstream_error_text(&error)
            ));
            None
        }
    };
    let source_file = download_data
        .as_ref()
        .and_then(|data| data.get("source_file"))
        .filter(|file| file.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Some(download_url) = source_file
        .get("download_url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        && !truthy(source_file.get("is_cold"))
        && !truthy(source_file.get("is_defrosting"))
    {
        let source_name = py_str_or(source_file.get("public_name"), "Original");
        if super::is_valid_url(
            client,
            &download_url,
            NAME,
            video_id,
            &format!("{source_name} video"),
        )? {
            let ext = source_file
                .get("extension")
                .and_then(Value::as_str)
                .filter(|ext| !ext.is_empty())
                .map(str::to_owned)
                .or_else(|| super::direct::determine_ext(&download_url))
                .unwrap_or_else(|| "mp4".into())
                .to_lowercase();
            let filesize = source_file
                .get("size")
                .and_then(Value::as_str)
                .and_then(super::xboxclips::parse_filesize);
            return super::dict_format(
                vec![
                    ("url", download_url.into()),
                    ("ext", ext.into()),
                    ("width", int_or_none(source_file.get("width")).into()),
                    ("height", int_or_none(source_file.get("height")).into()),
                    ("filesize", filesize.into()),
                    ("format_id", source_name.into()),
                    ("quality", 1.into()),
                ],
                NAME,
                video_id,
            )
            .map(Some);
        }
    }
    // Requesting only the `privacy` field is not rate-limited.
    let privacy = match call_videos_api(
        client,
        (video_id, unlisted_hash),
        None,
        Some("web"),
        &[("fields", "privacy")],
    )? {
        Ok(privacy) => privacy,
        Err((error, _, _)) => {
            crate::screen::extractor_debug(&format!(
                "Unable to download privacy info: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(None);
        }
    };
    if privacy
        .pointer("/privacy/download")
        .and_then(Value::as_bool)
        != Some(true)
    {
        let message = format!("{video_id}: Vimeo says this video is not downloadable");
        if policy != "always" {
            crate::screen::extractor_debug(&format!(
                "{message}, so yt-dlp is not attempting to extract the original/source format. To try anyways, use --extractor-args \"vimeo:original_format_policy=always\""
            ));
            return Ok(None);
        }
        crate::screen::extractor_debug(&format!(
            "{message}; attempting to extract original/source format anyways"
        ));
    }
    let original = match call_videos_api(
        client,
        (video_id, unlisted_hash),
        None,
        Some("web"),
        &[("fields", "download")],
    )? {
        Ok(value) => value,
        Err((error, _, _)) => {
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: {API_ERRNOTE}: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(None);
        }
    };
    for download in original
        .get("download")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|download| download.is_object())
    {
        let Some(link) = download
            .get("link")
            .and_then(Value::as_str)
            .filter(|link| !link.is_empty())
        else {
            continue;
        };
        if download.get("quality").and_then(Value::as_str) != Some("source") {
            continue;
        }
        let filename = Url::parse(link)
            .ok()
            .and_then(|url| {
                url.query_pairs()
                    .find(|(key, value)| key == "filename" && !value.is_empty())
                    .map(|(_, value)| value.to_lowercase())
            })
            .unwrap_or_default();
        let mut ext = super::direct::determine_ext(&filename);
        if ext.is_none() {
            crate::screen::screen(&format!(
                "[{NAME}] {video_id}: Determining source extension"
            ));
            ext = match super::direct::head_with_headers(client, &Url::parse(link)?, &[]) {
                Ok(response) => Some(super::direct::urlhandle_detect_ext(&response, None)),
                Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
                    crate::screen::warning(&format!(
                        "[{NAME}] {video_id}: Unable to download webpage: {}",
                        crate::upstream_error_text(&error)
                    ));
                    None
                }
                Err(error) => return Err(error),
            }
            .filter(|ext| !ext.is_empty());
        }
        return super::dict_format(
            vec![
                ("url", link.into()),
                ("ext", ext.unwrap_or_else(|| "unknown_video".into()).into()),
                (
                    "format_id",
                    py_str_or(download.get("public_name"), "Original").into(),
                ),
                ("width", int_or_none(download.get("width")).into()),
                ("height", int_or_none(download.get("height")).into()),
                ("fps", int_or_none(download.get("fps")).into()),
                ("filesize", int_or_none(download.get("size")).into()),
                ("quality", 1.into()),
            ],
            NAME,
            video_id,
        )
        .map(Some);
    }
    Ok(None)
}

/// `_get_subtitles(video_id, unlisted_hash)`: the API's text tracks.
fn api_subtitles(
    client: &Client,
    video_id: &str,
    unlisted_hash: Option<&str>,
) -> Result<crate::Subtitles> {
    let mut subtitles = crate::Subtitles::default();
    let tracks = match call_videos_api(
        client,
        (video_id, unlisted_hash),
        Some("texttracks"),
        None,
        &[
            ("include_transcript", "true"),
            (
                "fields",
                "active,display_language,id,language,link,name,type,uri",
            ),
        ],
    )? {
        Ok(tracks) => tracks,
        Err((error, _, _)) => {
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: {API_ERRNOTE}: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(subtitles);
        }
    };
    for track in tracks
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(link) = track
            .get("link")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let mut entry = json!({"url": link, "ext": "vtt"});
        if let Some(name) = track.get("display_language").and_then(Value::as_str) {
            entry["name"] = name.into();
        }
        let track_format: crate::SubtitleFormat =
            serde_json::from_value(entry).map_err(|error| fail(video_id, &error.to_string()))?;
        let language = track
            .get("language")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match subtitles
            .entries
            .iter_mut()
            .find(|(key, _)| *key == language)
        {
            Some((_, list)) => list.push(track_format),
            None => subtitles.entries.push((language, vec![track_format])),
        }
    }
    Ok(subtitles)
}

/// `_parse_api_response(video, video_id, unlisted_hash)`: an API response
/// without a `config_url` (its `play`, `files` and `download` links).
fn parse_api_response(
    client: &Client,
    video: &Value,
    video_id: &str,
    unlisted_hash: Option<&str>,
) -> Result<(
    Map<String, Value>,
    Vec<crate::VideoFormat>,
    crate::Subtitles,
)> {
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    let mut seen: Vec<String> = Vec::new();
    let duration = int_or_none(video.get("duration"));
    let mut files: Vec<&Value> = Vec::new();
    for pointer in [
        "/play/files",
        "/play/progressive/files",
        "/files",
        "/download",
    ] {
        if let Some(list) = video.pointer(pointer).and_then(Value::as_array) {
            files.extend(list.iter().filter(|file| {
                file.get("link")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .is_some()
            }));
        }
    }
    for file in files {
        let format_url = file["link"].as_str().unwrap_or_default().to_owned();
        if seen.contains(&format_url) {
            continue;
        }
        seen.push(format_url.clone());
        let quality = file.get("quality");
        let ext = super::direct::determine_ext(&format_url);
        let quality_text = quality.and_then(Value::as_str);
        if quality_text == Some("hls") || ext.as_deref() == Some("m3u8") {
            if let Some(stream) = Url::parse(&format_url).ok().and_then(|manifest| {
                super::extract_m3u8_optional(client, &manifest, NAME, video_id, Some("hls"), true)
            }) {
                formats.extend(stream.formats);
                super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else if quality_text == Some("dash") || ext.as_deref() == Some("mpd") {
            if let Some(stream) = Url::parse(&format_url).ok().and_then(|manifest| {
                super::extract_mpd_optional_note(
                    client,
                    &manifest,
                    NAME,
                    video_id,
                    Some("dash"),
                    "Downloading MPD manifest",
                )
            }) {
                for mut format in stream.formats {
                    let mut parts: Vec<String> = format
                        .format_id
                        .splitn(3, '-')
                        .take(2)
                        .filter(|part| !part.is_empty())
                        .map(str::to_owned)
                        .collect();
                    if let Some(tbr) = format.tbr {
                        parts.push((tbr as i64).to_string());
                    }
                    format.format_id = parts.join("-");
                    crate::info_order::mark_format_update(&mut format, "format_id");
                    formats.push(format);
                }
                super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else {
            let mut fields: Vec<(&str, Value)> = Vec::new();
            if let Some(mime) = file.get("type").and_then(Value::as_str) {
                fields.push((
                    "ext",
                    super::direct::mimetype2ext_or(mime, Some("mp4")).into(),
                ));
            }
            if let Some(codec) = file.get("codec").and_then(Value::as_str) {
                fields.push(("vcodec", codec.to_lowercase().into()));
            }
            let filesize = int_or_none(file.get("size"));
            for (key, value) in [
                ("width", int_or_none(file.get("width"))),
                ("height", int_or_none(file.get("height"))),
                ("filesize", filesize),
                ("fps", int_or_none(file.get("fps"))),
            ] {
                if let Some(value) = value {
                    fields.push((key, value.into()));
                }
            }
            let rendition = file
                .get("public_name")
                .filter(|value| !value.is_null())
                .or_else(|| file.get("rendition").filter(|value| !value.is_null()));
            let mut format_id = vec!["http".to_owned()];
            for value in [rendition, quality] {
                if truthy(value) || value.is_some_and(|value| value == &json!(0)) {
                    format_id.push(py_str_or(value, ""));
                }
            }
            let tbr = match (filesize, duration) {
                (Some(size), Some(duration)) if duration != 0 => {
                    Some(size as f64 * 8.0 / duration as f64 / 1024.0)
                }
                _ => None,
            };
            fields.push(("url", format_url.clone().into()));
            fields.push(("format_id", format_id.join("-").into()));
            fields.push(("tbr", tbr.into()));
            formats.push(super::dict_format(fields, NAME, video_id)?);
        }
    }
    if video
        .pointer("/metadata/connections/texttracks/total")
        .and_then(Value::as_i64)
        .is_some_and(|total| total != 0)
        && super::subtitles_requested()
    {
        let tracks = api_subtitles(client, video_id, unlisted_hash)?;
        super::html5::merge_subtitles(&mut subtitles, tracks);
    }
    let mut info = Map::new();
    if let Some(title) = video.get("name").and_then(Value::as_str) {
        info.insert("title".into(), title.into());
    }
    if let Some(name) = video.pointer("/user/name").and_then(Value::as_str) {
        info.insert("uploader".into(), name.into());
    }
    if let Some(link) = video.pointer("/user/link").and_then(Value::as_str) {
        if let Ok(parsed) = Url::parse(link) {
            info.insert(
                "uploader_id".into(),
                crate::info::url_basename(&parsed).into(),
            );
        }
        if let Some(url) = super::json_ld::url_or_none(link) {
            info.insert("uploader_url".into(), url.into());
        }
    }
    if let Some(timestamp) = int_or_none(video.pointer("/live/scheduled_start_time")) {
        info.insert("release_timestamp".into(), timestamp.into());
    }
    if let Some(sizes) = video.pointer("/pictures/sizes").and_then(Value::as_array) {
        let thumbnails: Vec<Value> = sizes
            .iter()
            .filter_map(|size| {
                let link = size
                    .get("link")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)?;
                let mut thumbnail = Map::new();
                thumbnail.insert("url".into(), link.into());
                if let Some(width) = int_or_none(size.get("width")) {
                    thumbnail.insert("width".into(), width.into());
                }
                if let Some(height) = int_or_none(size.get("height")) {
                    thumbnail.insert("height".into(), height.into());
                }
                Some(Value::Object(thumbnail))
            })
            .collect();
        info.insert("thumbnails".into(), Value::Array(thumbnails));
    }
    info.insert("id".into(), video_id.into());
    info.insert("duration".into(), duration.into());
    info.insert("formats".into(), json!([]));
    info.insert("subtitles".into(), json!({}));
    let live_status = match video.pointer("/live/status").and_then(Value::as_str) {
        Some("streaming") => Some("is_live"),
        Some("done") => Some("was_live"),
        _ => None,
    };
    info.insert("live_status".into(), live_status.into());
    Ok((info, formats, subtitles))
}

/// `_extract_from_api(video_id, unlisted_hash)`: the API client's video
/// (its `config_url` through `_parse_config`, else `_parse_api_response`),
/// the original format and the API's metadata.
fn extract_from_api(
    client: &Client,
    url: &Url,
    video_id: &str,
    unlisted_hash: Option<&str>,
) -> Result<VideoInfo> {
    let mut video = Value::Null;
    for retry in [false, true] {
        match call_videos_api(client, (video_id, unlisted_hash), None, None, &[])? {
            Ok(value) => {
                video = value;
                break;
            }
            Err((error, status, body)) => {
                let response = serde_json::from_str::<Value>(&body)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({}));
                let password = response
                    .get("invalid_parameters")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|parameter| {
                        parameter.get("field").and_then(Value::as_str) == Some("password")
                    });
                if !retry && status == 400 && password {
                    verify_video_password(client, video_id)?;
                } else if status == 404 && response.get("error_code") == Some(&json!(5460)) {
                    let mut parts = Vec::new();
                    if let Some(text) = response
                        .get("error")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                    {
                        parts.push(text.to_owned());
                    }
                    parts.push("Authentication may be needed due to your location.".into());
                    parts.push(
                        "If your IP address is located in Europe you could try using a VPN/proxy,"
                            .into(),
                    );
                    let hint = super::login::login_hint(Some(NAME), None);
                    parts.push(format!("or else u{}", &hint[1..]));
                    return Err(fail(video_id, &parts.join(" ")));
                } else {
                    return Err(request_failure(video_id, API_ERRNOTE, &error));
                }
            }
        }
    }
    let config_url = video
        .get("config_url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none);
    let api_shape = config_url.is_none();
    let (mut info, mut formats, subtitles) = match config_url {
        Some(config_url) => {
            let config = super::download_json(client, &Url::parse(&config_url)?, NAME, video_id)?;
            parse_config(client, &config, video_id)?
        }
        None => parse_api_response(client, &video, video_id, unlisted_hash)?,
    };
    if let Some(source) = original_format(client, video_id, unlisted_hash)? {
        formats.push(source);
    }
    let timestamp = |key: &str| {
        video
            .get(key)
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
    };
    info.insert(
        "description".into(),
        video.get("description").cloned().unwrap_or(Value::Null),
    );
    info.insert(
        "license".into(),
        video.get("license").cloned().unwrap_or(Value::Null),
    );
    info.insert("release_timestamp".into(), timestamp("release_time").into());
    info.insert("timestamp".into(), timestamp("created_time").into());
    info.insert(
        "view_count".into(),
        int_or_none(video.pointer("/stats/plays")).into(),
    );
    for key in ["comment", "like"] {
        info.insert(
            format!("{key}_count"),
            int_or_none(video.pointer(&format!("/metadata/connections/{key}s/total"))).into(),
        );
    }
    let nested = if api_shape {
        "subtitle=url,ext,name;thumbnail=url,width,height"
    } else {
        "subtitle=ext,url"
    };
    Ok(video_keyed(url, info, formats, subtitles, nested))
}

impl Extractor for Vimeo {
    fn key(&self) -> &'static str {
        KEY
    }

    fn netrc_machine(&self) -> Option<&'static str> {
        Some("vimeo")
    }

    fn performs_login(&self) -> bool {
        true
    }

    /// `VimeoBaseInfoExtractor._perform_login`: skipped with a `vimeo`
    /// cookie, else the viewer's XSRF token posted with the credentials to
    /// `/log_in` (404/405/418 meaning bad credentials), then the logged-out
    /// viewer info dropped.
    fn perform_login(
        &self,
        client: &Client,
        session: &super::login::Session,
        username: &str,
        password: &str,
    ) -> Result<()> {
        if is_logged_in() {
            return Ok(());
        }
        let viewer = fetch_viewer_info(client, None, &session.id)?;
        let token = viewer_field(&viewer, "xsrft", &session.id)?;
        let data = super::login::urlencode(&[
            ("action", "login"),
            ("email", username),
            ("password", password),
            ("service", "vimeo"),
            ("token", &token),
        ]);
        match fetch(
            client,
            &Url::parse(LOGIN_URL)?,
            (None, "Logging in"),
            &[
                ("Content-Type", "application/x-www-form-urlencoded"),
                ("Referer", LOGIN_URL),
            ],
            Some(data),
        ) {
            Ok(Fetched::Page(_)) => {}
            Ok(Fetched::Status {
                status: 404 | 405 | 418,
                ..
            }) => return Err(session.error("Unable to log in: bad username or password")),
            Ok(Fetched::Status { .. }) | Err(_) => return Err(session.bug("Unable to log in")),
        }
        *VIEWER.lock().unwrap() = None;
        Ok(())
    }

    /// `_real_initialize`: without a login, a cached android OAuth token
    /// makes android the default client (unless `vimeo:client` is given).
    fn real_initialize(&self, _client: &Client, _session: &super::login::Session) -> Result<()> {
        if is_logged_in() || client_arg().is_some() {
            return Ok(());
        }
        if crate::cache::load(NAME, ANDROID_CACHE_KEY, None)
            .is_some_and(|token| !token.is_null() && token != "" && token != false)
        {
            *DEFAULT_CLIENT.lock().unwrap() = "android";
            crate::screen::extractor_debug(
                "Found cached android token; using android as default API client",
            );
        }
        Ok(())
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some((video_id, unlisted_hash)) = match_url(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        self.initialize(client, NAME, &video_id)?;
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        if let Some(hash) = unlisted_hash.as_deref() {
            return extract_from_api(client, url, &video_id, Some(hash)).map(Some);
        }
        // `_unsmuggle_headers`: an embed's smuggled `referer`, else the URL.
        let (bare, smuggled) = super::brightcove::unsmuggle(url.as_str());
        let bare = if smuggled.is_empty() {
            url.clone()
        } else {
            Url::parse(&bare)?
        };
        let mut page = bare.clone();
        if ["play_redirect_hls", "moogaloop.swf"]
            .iter()
            .any(|marker| bare.as_str().contains(marker))
        {
            page = Url::parse(&format!("https://vimeo.com/{video_id}"))?;
        }
        static ALBUM: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"vimeo\.com/(?:album|showcase)/([0-9]+)/").unwrap());
        if let Some(found) = ALBUM.captures(page.as_str()) {
            // Detect a password-protected showcase video: post its password.
            album_data(client, &found[1], &video_id)?;
        }
        // `impersonate=is_secure`.
        let impersonate = (page.scheme() == "https").then(|| {
            crate::impersonate::attempt(crate::impersonate::ANY, |message| {
                crate::screen::warning_once(&format!("[{NAME}] {message}"));
            })
        });
        let referer = match smuggled.get("referer") {
            Some(Value::String(referer)) => referer.clone(),
            _ => bare.as_str().to_owned(),
        };
        crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading webpage"));
        let response =
            super::direct::request_with_headers(client, &page, None, &[("Referer", &referer)])
                .map_err(|error| match error {
                    Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                        extractor: NAME,
                        id: video_id.clone(),
                        what: "webpage",
                        source: Box::new(error),
                    },
                    error => error,
                })?;
        let webpage = {
            use std::io::Read;
            let mut response = response;
            let mut body = Vec::new();
            response
                .read_to_end(&mut body)
                .map_err(Error::ExtractionRead)?;
            drop(impersonate);
            String::from_utf8_lossy(&body).into_owned()
        };
        if page.host_str() == Some("player.vimeo.com") {
            let config = match super::search_json(&PLAYER_CONFIG, &webpage, false) {
                Some(Some(config)) => config,
                _ => {
                    return Err(fail(
                        &video_id,
                        &format!(
                            "Unable to extract info section; {}",
                            crate::BUG_REPORTS_MESSAGE
                        ),
                    ));
                }
            };
            if config.get("view").and_then(Value::as_i64) == Some(4) {
                if super::video_password().is_some() {
                    return Err(Error::UnsupportedHtml5(
                        "Vimeo player video passwords (check-password) are not supported natively"
                            .into(),
                    ));
                }
                return Err(fail(
                    &video_id,
                    "This video is protected by a password, use the --video-password option",
                ));
            }
            let (info, mut formats, subtitles) = parse_config(client, &config, &video_id)?;
            if let Some(source) = original_format(client, &video_id, None)? {
                formats.push(source);
            }
            return Ok(Some(video_info(url, info, formats, subtitles)));
        }
        if let Some(found) = VIMEO_CONFIG.captures(&webpage)
            && let Some(config) = found
                .get(1)
                .or(found.get(2))
                .and_then(|json| serde_json::from_str::<Value>(json.as_str()).ok())
            && config.pointer("/seed_status/state").and_then(Value::as_str) == Some("failed")
        {
            let title = config
                .pointer("/seed_status/title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(fail(&video_id, &format!("{NAME} said: {title}")));
        }
        let mut info_dict = Map::new();
        let mut description: Option<String> = None;
        let mut config_url: Option<String> = None;
        static CHANNEL: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"vimeo\.com/channels/([^/?#]+)").unwrap());
        if let Some(found) = CHANNEL.captures(bare.as_str()) {
            static CONFIG_URL: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r#"\bdata-config-url="([^"]+)""#).unwrap());
            config_url = CONFIG_URL
                .captures(&webpage)
                .map(|found| crate::html::unescape_html(&found[1]));
            let channel = found[1].to_owned();
            info_dict.insert("channel_id".into(), channel.clone().into());
            info_dict.insert(
                "channel_url".into(),
                format!("https://vimeo.com/channels/{channel}").into(),
            );
        }
        let mut timestamp: Option<String> = None;
        let mut license: Option<Value> = None;
        if config_url.is_none() {
            let page_config = PAGE_CONFIG
                .captures(&webpage)
                .and_then(|found| serde_json::from_str::<Value>(&found[1]).ok())
                .filter(|config| config.as_object().is_some_and(|config| !config.is_empty()));
            let Some(page_config) = page_config else {
                return extract_from_api(client, url, &video_id, None).map(Some);
            };
            config_url = page_config
                .pointer("/player/config_url")
                .and_then(Value::as_str)
                .map(str::to_owned);
            license = page_config
                .get("cc_license")
                .filter(|v| !v.is_null())
                .cloned();
            timestamp = page_config
                .pointer("/clip/uploaded_on")
                .and_then(Value::as_str)
                .map(str::to_owned);
            description = page_config
                .pointer("/clip/description")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    page_config
                        .get("description_html_escaped")
                        .and_then(Value::as_str)
                })
                .map(crate::html::clean_html);
        }
        let config_url = Url::parse(config_url.as_deref().unwrap_or_default())?;
        let config = super::download_json(client, &config_url, NAME, &video_id)?;
        if description.as_deref().is_none_or(str::is_empty) {
            static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r#"(?s)<div\s+class="[^"]*description[^"]*"[^>]*>(.*?)</div>"#).unwrap()
            });
            description = DESCRIPTION
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]))
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    ["description", "og:description", "twitter:description"]
                        .iter()
                        .find_map(|name| crate::html::html_search_meta(&webpage, name))
                });
        }
        if description.as_deref().is_none_or(str::is_empty) {
            crate::screen::warning(&format!("[{NAME}] Cannot find video description"));
        }
        if timestamp.is_none() {
            static TIME: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r#"<time[^>]+datetime="([^"]+)""#).unwrap());
            timestamp = TIME.captures(&webpage).map(|found| found[1].to_owned());
        }
        let count = |pattern: &str| {
            Regex::new(pattern)
                .ok()?
                .captures(&webpage)
                .and_then(|found| found[1].parse::<i64>().ok())
        };
        if license.is_none() {
            static LICENSE: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r#"<link[^>]+rel=["']license["'][^>]+href="([^"]+)""#).unwrap()
            });
            license = LICENSE
                .captures(&webpage)
                .map(|found| Value::from(found[1].to_owned()));
        }
        let unlisted = config
            .pointer("/video/unlisted_hash")
            .and_then(Value::as_str);
        let mut formats: Vec<crate::VideoFormat> = original_format(client, &video_id, unlisted)?
            .into_iter()
            .collect();
        let (config_info, config_formats, subtitles) = parse_config(client, &config, &video_id)?;
        formats.extend(config_formats);
        info_dict.insert(
            "_format_sort_fields".into(),
            config_info["_format_sort_fields"].clone(),
        );
        info_dict.insert("formats".into(), json!([]));
        info_dict.insert(
            "timestamp".into(),
            timestamp
                .as_deref()
                .and_then(|text| crate::dates::unified_timestamp(text, false))
                .into(),
        );
        info_dict.insert("description".into(), description.into());
        info_dict.insert("webpage_url".into(), bare.as_str().into());
        info_dict.insert("view_count".into(), count(r"UserPlays:(\d+)").into());
        info_dict.insert("like_count".into(), count(r"UserLikes:(\d+)").into());
        info_dict.insert("comment_count".into(), count(r"UserComments:(\d+)").into());
        info_dict.insert("license".into(), license.unwrap_or(Value::Null));
        let dict = merge_dicts(vec![info_dict, config_info]);
        Ok(Some(video_info(&bare, dict, formats, subtitles)))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert_eq!(
            match_url("https://vimeo.com/76979871"),
            Some(("76979871".into(), None))
        );
        assert_eq!(
            match_url("https://player.vimeo.com/video/54469442"),
            Some(("54469442".into(), None))
        );
        assert_eq!(
            match_url("https://vimeo.com/123456789/abcdef0123"),
            Some(("123456789".into(), Some("abcdef0123".into())))
        );
        assert!(suitable("https://vimeo.com/channels/staffpicks/77810431"));
        assert!(suitable("https://vimeo.com/user7108434/my-clip"));
        assert!(!suitable("https://vimeo.com/channels/staffpicks"));
        assert!(!suitable(
            "https://vimeo.com/ondemand/thelastcolony/150274832"
        ));
        assert!(!suitable("https://vimeo.com/user7108434/videos"));
        assert_eq!(
            match_url("https://vimeo.com/album/2632481/video/79010983"),
            Some(("79010983".into(), None))
        );
        assert_eq!(
            match_url("https://vimeo.com/showcase/3253534/video/119195465"),
            Some(("119195465".into(), None))
        );
    }
}
