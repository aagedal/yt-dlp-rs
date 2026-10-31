//! Upstream `BiliBiliIE` (`yt_dlp/extractor/bilibili.py`): the video page's
//! `__INITIAL_STATE__`, anthologies (playlists of their parts, or one part
//! with `?p=N`), the WBI-signed `playurl` API's DASH and legacy `durl`
//! formats (other `accept_quality` values requested one by one), chapters,
//! the danmaku subtitle track, multi-part FLV `multi_video` results and
//! upstream's errors. Interactive videos, inline SRT subtitles and bangumi
//! redirects are reported as unsupported.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct BiliBili;

const NAME: &str = "BiliBili";
const LOGIN_HINT: &str = "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";
/// `getMixinKey()`'s table from the vendor JS.
const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?bilibili\.com/(?:video/|festival/[^/?#]+\?(?:[^#]*&)?bvid=)(?P<prefix>[aAbB][vV])(?P<id>[^/?#&]+)")
        .unwrap()
});
static BANGUMI_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?bilibili\.com/bangumi/play/ep\d+").unwrap());
static INITIAL_STATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__INITIAL_STATE__\s*=").unwrap());
static PLAY_INFO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__playinfo__\s*=").unwrap());
static RISK_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bwindow\._riskdata_\s*=").unwrap());
static FORMAT_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-(\d+)\.m4s\?").unwrap());
/// `_wbi_key_cache`: the key and when it was fetched (`_WBI_KEY_CACHE_TIMEOUT`).
static WBI_KEY: Mutex<Option<(std::time::Instant, String)>> = Mutex::new(None);

/// `BiliBiliIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
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

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn float(value: Option<&Value>, scale: f64) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .map(|number| number / scale)
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.into()),
        _ => None,
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(items)) => !items.is_empty(),
    }
}

/// A clock-seeded xorshift64 for `random`'s fingerprint values.
fn random_below(bound: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static STATE: Mutex<u64> = Mutex::new(0);
    let mut state = STATE.lock().unwrap_or_else(|poison| poison.into_inner());
    if *state == 0 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos()),
        );
        *state = hasher.finish() | 1;
    }
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state % bound.max(1)
}

/// `base64.b64encode(random printable text)[:-2]`.
fn random_b64(minimum: u64, maximum: u64) -> String {
    use base64::Engine;
    const PRINTABLE: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~ \t\n\r\x0b\x0c";
    let length = minimum + random_below(maximum - minimum + 1);
    let text: Vec<u8> = (0..length)
        .map(|_| PRINTABLE[random_below(PRINTABLE.len() as u64) as usize])
        .collect();
    let mut encoded = base64::engine::general_purpose::STANDARD.encode(text);
    encoded.truncate(encoded.len().saturating_sub(2));
    encoded
}

/// `_dm_params`: the user-fingerprint query values.
fn dm_params() -> Vec<(String, String)> {
    const DIMENSIONS: [((i64, i64), u64); 7] = [
        ((1920, 1080), 18),
        ((1366, 768), 18),
        ((1536, 864), 17),
        ((1280, 720), 8),
        ((2560, 1440), 7),
        ((1440, 900), 5),
        ((1600, 900), 5),
    ];
    let mut pick = random_below(DIMENSIONS.iter().map(|(_, weight)| weight).sum());
    let (width, height) = DIMENSIONS
        .iter()
        .find_map(|(size, weight)| {
            if pick < *weight {
                Some(*size)
            } else {
                pick -= weight;
                None
            }
        })
        .unwrap_or((1920, 1080));
    let rnd = random_below(114) as i64;
    let wh = [
        2 * width + 2 * height + 3 * rnd,
        4 * width - height + rnd,
        rnd,
    ];
    let top = random_below(101) as i64;
    let rnd = random_below(514) as i64;
    let of = [3 * top + rnd, 4 * top + 2 * rnd, rnd];
    vec![
        ("dm_img_list".into(), "[]".into()),
        ("dm_img_str".into(), random_b64(16, 64)),
        ("dm_cover_img_str".into(), random_b64(32, 128)),
        (
            "dm_img_inter".into(),
            serde_json::to_string(&json!({"ds": [], "wh": wh, "of": of})).unwrap_or_default(),
        ),
    ]
}

/// `urllib.parse.urlencode(params)`.
fn urlencode(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                super::html5::quote_plus(key),
                super::html5::quote_plus(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// `url` with `query` (`update_url_query`): this API's URLs have none.
fn with_query(url: &str, params: &[(String, String)]) -> Result<Url> {
    let mut url = Url::parse(url)?;
    let query = urlencode(params);
    url.set_query((!query.is_empty()).then_some(query.as_str()));
    Ok(url)
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// `_get_wbi_key(video_id)`: the nav API's image names, mixed.
fn wbi_key(client: &Client, video_id: &str) -> Result<String> {
    if let Ok(cache) = WBI_KEY.lock()
        && let Some((fetched, key)) = cache.as_ref()
        && fetched.elapsed() < std::time::Duration::from_secs(30)
    {
        return Ok(key.clone());
    }
    let nav = Url::parse("https://api.bilibili.com/x/web-interface/nav")?;
    let session = super::download_json_note(client, &nav, NAME, video_id, "Downloading wbi sign")?;
    let lookup: String = ["img_url", "sub_url"]
        .iter()
        .filter_map(|key| {
            session
                .pointer(&format!("/data/wbi_img/{key}"))
                .and_then(Value::as_str)
        })
        .map(|url| {
            let name = url.rsplit_once('/').map_or(url, |(_, name)| name);
            name.split_once('.')
                .map_or(name, |(stem, _)| stem)
                .to_owned()
        })
        .collect();
    let chars: Vec<char> = lookup.chars().collect();
    let mut key = String::new();
    for index in MIXIN_KEY_ENC_TAB {
        let Some(c) = chars.get(index) else {
            return Err(fail(
                video_id,
                &format!(
                    "An extractor error has occurred. (caused by IndexError('string index out of range')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        };
        key.push(*c);
    }
    let key: String = key.chars().take(32).collect();
    if let Ok(mut cache) = WBI_KEY.lock() {
        *cache = Some((std::time::Instant::now(), key.clone()));
    }
    Ok(key)
}

/// `_sign_wbi(params, video_id)`: `wts`, sorted and filtered values, and the
/// `w_rid` MD5 of their query with the WBI key.
fn sign_wbi(
    client: &Client,
    mut params: Vec<(String, String)>,
    video_id: &str,
) -> Result<Vec<(String, String)>> {
    let wts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs_f64().round() as u64);
    match params.iter_mut().find(|(key, _)| key == "wts") {
        Some((_, value)) => *value = wts.to_string(),
        None => params.push(("wts".into(), wts.to_string())),
    }
    params.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (_, value) in &mut params {
        value.retain(|c| !"!'()*".contains(c));
    }
    let query = urlencode(&params);
    let key = wbi_key(client, video_id)?;
    params.push((
        "w_rid".into(),
        crate::md5::hex(format!("{query}{key}").as_bytes()),
    ));
    Ok(params)
}

fn logged_in() -> bool {
    crate::cookies::get_cookie("https://api.bilibili.com", "SESSDATA")
        .is_some_and(|value| !value.is_empty())
}

/// `_download_playinfo(bvid, cid, headers, query, fatal)`.
fn download_playinfo(
    client: &Client,
    bvid: &str,
    cid: &str,
    headers: &[(&str, &str)],
    query: &[(&str, &str)],
    fatal: bool,
) -> Result<Option<Value>> {
    let mut params = pairs(&[("bvid", bvid), ("cid", cid), ("fnval", "4048")]);
    params.extend(dm_params());
    params.extend(pairs(query));
    if logged_in() {
        params.retain(|(key, _)| key != "try_look");
    }
    let note = match params.iter().find(|(key, _)| key == "qn") {
        Some((_, qn)) if !qn.is_empty() => format!("Downloading video format {qn} for cid {cid}"),
        _ => format!("Downloading video formats for cid {cid}"),
    };
    let signed = sign_wbi(client, params, bvid)?;
    let url = with_query("https://api.bilibili.com/x/player/wbi/playurl", &signed)?;
    let raw = super::download_json_note_headers(client, &url, NAME, bvid, &note, headers)?;
    let code = int(raw.get("code")).map(|code| -code);
    if code == Some(0) {
        return match raw.get("data") {
            Some(data) => Ok(Some(data.clone())),
            None => Err(key_error(bvid, "data")),
        };
    }
    let mut parts = vec!["Unable to download video info".to_owned()];
    if let Some(code) = code
        && code != 0
    {
        parts.push(code.to_string());
    }
    if let Some(message) = raw
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
    {
        parts.push(message.to_owned());
    }
    let mut message = parts.join(": ");
    if matches!(code, Some(401 | 352)) {
        message.push_str(", please wait and try later");
    }
    if fatal {
        if !matches!(code, Some(401 | 352)) {
            message = format!("{message}; {}", crate::BUG_REPORTS_MESSAGE);
        }
        return Err(fail(bvid, &message));
    }
    crate::screen::warning(&format!("[{NAME}] {message}"));
    Ok(None)
}

/// `{r['quality']: new_description or display_desc}` of `support_formats`.
fn format_names(play_info: &Value) -> Vec<(Value, Value)> {
    play_info
        .get("support_formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|format| truthy(format.get("quality")))
        .map(|format| {
            let name = ["new_description", "display_desc"]
                .iter()
                .find_map(|key| format.get(*key).filter(|value| !value.is_null()))
                .cloned()
                .unwrap_or(Value::Null);
            (format["quality"].clone(), name)
        })
        .collect()
}

fn format_name(names: &[(Value, Value)], quality: Option<&Value>) -> Value {
    let Some(quality) = quality else {
        return Value::Null;
    };
    names
        .iter()
        .rev()
        .find(|(key, _)| key == quality)
        .map_or(Value::Null, |(_, name)| name.clone())
}

/// `_check_missing_formats(play_info, formats)`.
fn check_missing_formats(play_info: &Value, formats: &[Map<String, Value>]) {
    let parsed: Vec<&Value> = formats
        .iter()
        .map(|format| format.get("quality").unwrap_or(&Value::Null))
        .collect();
    let missing: Vec<String> = play_info
        .get("support_formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|format| {
            format
                .get("quality")
                .is_some_and(|quality| !parsed.contains(&quality))
        })
        .filter_map(|format| {
            ["new_description", "display_desc", "quality"]
                .iter()
                .find_map(|key| format.get(*key).filter(|value| !value.is_null()))
                .and_then(|value| match value {
                    Value::String(text) => Some(text.clone()),
                    Value::Number(number) => Some(number.to_string()),
                    _ => None,
                })
        })
        .filter(|text| !text.is_empty())
        .collect();
    if !missing.is_empty() {
        crate::screen::screen(&format!(
            "[{NAME}] Format(s) {} are missing; you have to become a premium member to download them. {LOGIN_HINT}",
            missing.join(", ")
        ));
    }
}

fn first_of<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| value.get(*key).filter(|value| !value.is_null()))
}

fn mime_ext(value: Option<&Value>) -> Value {
    value.and_then(Value::as_str).map_or(Value::Null, |mime| {
        Value::from(super::direct::mimetype2ext_or(mime, None))
    })
}

/// `extract_formats(play_info)`: DASH audio (with Dolby and FLAC) and video,
/// then the legacy `durl` parts as one format.
fn extract_formats(play_info: Option<&Value>) -> Vec<Map<String, Value>> {
    let Some(play_info) = play_info else {
        return Vec::new();
    };
    let names = format_names(play_info);
    let mut audios: Vec<&Value> = Vec::new();
    for path in ["/dash/audio", "/dash/dolby/audio"] {
        audios.extend(
            play_info
                .pointer(path)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|audio| audio.is_object()),
        );
    }
    if let Some(flac) = play_info
        .pointer("/dash/flac/audio")
        .filter(|flac| truthy(Some(flac)))
    {
        audios.push(flac);
    }
    let mut formats: Vec<Map<String, Value>> = audios
        .iter()
        .map(|audio| {
            let mut format = Map::new();
            format.insert(
                "url".into(),
                first_of(audio, &["baseUrl", "base_url", "url"])
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            format.insert(
                "ext".into(),
                mime_ext(first_of(audio, &["mimeType", "mime_type"])),
            );
            format.insert(
                "acodec".into(),
                audio
                    .get("codecs")
                    .and_then(Value::as_str)
                    .map_or(Value::Null, |codecs| codecs.to_lowercase().into()),
            );
            format.insert("vcodec".into(), "none".into());
            format.insert(
                "tbr".into(),
                float(audio.get("bandwidth"), 1000.0).map_or(Value::Null, |tbr| json!(tbr)),
            );
            format.insert("filesize".into(), int(audio.get("size")).into());
            format.insert(
                "format_id".into(),
                str_or_none(audio.get("id")).map_or(Value::Null, Value::from),
            );
            format
        })
        .collect();
    for video in play_info
        .pointer("/dash/video")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let mut format = Map::new();
        format.insert(
            "url".into(),
            first_of(video, &["baseUrl", "base_url", "url"])
                .cloned()
                .unwrap_or(Value::Null),
        );
        format.insert(
            "ext".into(),
            mime_ext(first_of(video, &["mimeType", "mime_type"])),
        );
        format.insert(
            "fps".into(),
            float(first_of(video, &["frameRate", "frame_rate"]), 1.0)
                .map_or(Value::Null, |fps| json!(fps)),
        );
        format.insert("width".into(), int(video.get("width")).into());
        format.insert("height".into(), int(video.get("height")).into());
        format.insert(
            "vcodec".into(),
            video.get("codecs").cloned().unwrap_or(Value::Null),
        );
        format.insert(
            "acodec".into(),
            if audios.is_empty() {
                Value::Null
            } else {
                "none".into()
            },
        );
        let id = int(video.get("id"));
        format.insert(
            "dynamic_range".into(),
            match id {
                Some(126) => "DV".into(),
                Some(125) => "HDR10".into(),
                _ => Value::Null,
            },
        );
        format.insert(
            "tbr".into(),
            float(video.get("bandwidth"), 1000.0).map_or(Value::Null, |tbr| json!(tbr)),
        );
        format.insert("filesize".into(), int(video.get("size")).into());
        format.insert("quality".into(), id.into());
        let format_id = ["baseUrl", "base_url"]
            .iter()
            .filter_map(|key| video.get(*key).and_then(Value::as_str))
            .find_map(|url| FORMAT_ID.captures(url).map(|found| found[1].to_owned()))
            .or_else(|| str_or_none(video.get("id")));
        format.insert(
            "format_id".into(),
            format_id.map_or(Value::Null, Value::from),
        );
        format.insert("format".into(), format_name(&names, video.get("id")));
        formats.push(format);
    }
    if !formats.is_empty() {
        check_missing_formats(play_info, &formats);
    }
    let fragments: Vec<Map<String, Value>> = play_info
        .get("durl")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|part| {
            part.get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .is_some()
        })
        .map(|part| {
            let mut fragment = Map::new();
            fragment.insert("url".into(), part["url"].clone());
            if let Some(duration) = float(part.get("length"), 1000.0) {
                fragment.insert("duration".into(), json!(duration));
            }
            if let Some(size) = int(part.get("size")) {
                fragment.insert("filesize".into(), size.into());
            }
            fragment
        })
        .collect();
    if let Some(first) = fragments.first() {
        let mut format = Map::new();
        format.insert("url".into(), first["url"].clone());
        format.insert(
            "filesize".into(),
            fragments
                .iter()
                .filter_map(|fragment| fragment.get("filesize").and_then(Value::as_i64))
                .sum::<i64>()
                .into(),
        );
        if fragments.len() > 1 {
            format.insert(
                "fragments".into(),
                Value::Array(fragments.iter().cloned().map(Value::Object).collect()),
            );
            format.insert("protocol".into(), "http_dash_segments".into());
        }
        let quality = play_info.get("quality");
        if let Some(quality) = int(quality) {
            format.insert("quality".into(), quality.into());
        }
        if let Some(id) = str_or_none(quality) {
            format.insert("format_id".into(), id.into());
        }
        let note = format_name(&names, quality);
        if !note.is_null() {
            format.insert("format_note".into(), note.clone());
        }
        if let Some(duration) = float(play_info.get("timelength"), 1000.0) {
            format.insert("duration".into(), json!(duration));
        }
        if let Some(note) = note.as_str() {
            let (width, height) = super::html5::parse_resolution(note);
            if let Some(width) = width {
                format.insert("width".into(), width.into());
            }
            if let Some(height) = height {
                format.insert("height".into(), height.into());
            }
        }
        formats.push(format);
    }
    formats
}

fn format_from(mut format: Map<String, Value>, id: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = format.keys().cloned().collect();
    format.retain(|_, value| !value.is_null());
    if !format.contains_key("ext") {
        let ext = format
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext)
            .unwrap_or_else(|| "unknown_video".into());
        format.insert("ext".into(), ext.into());
    }
    if !format.contains_key("format_id") {
        format.insert("format_id".into(), "".into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_quality(&mut format);
    Ok(format)
}

/// `_download_json(..., fatal=False)`: a warning and `None` on failure.
fn optional_json(
    client: &Client,
    url: &Url,
    id: &str,
    note: &str,
    headers: &[(&str, &str)],
) -> Result<Option<Value>> {
    match super::download_json_note_headers(client, url, NAME, id, note, headers) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
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

/// `url_result(url, BiliBiliIE)` for an anthology part.
fn part_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": "bilibili",
            "entry": {"ie_key": NAME, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

fn follow(page: &Url, target: &Url) -> ExtractResult {
    let mut follow = super::direct::build_at(page.clone(), page, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow
        .extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    ExtractResult::Video(follow)
}

impl Extractor for BiliBili {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        let prefix = found["prefix"].to_uppercase();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let (webpage, final_url) = super::download_webpage_handle(client, page, NAME, &temp_id)?;
        if !VALID_URL.is_match(final_url.as_str()) {
            return Ok(Some(follow(page, &final_url)));
        }
        let headers = [
            ("Referer", "https://www.bilibili.com/"),
            ("Origin", "https://www.bilibili.com"),
        ];
        let initial_state = super::search_json(&INITIAL_STATE, &webpage, false)
            .flatten()
            .filter(|state| truthy(Some(state)));
        let Some(initial_state) = initial_state else {
            let risk = super::search_json(&RISK_DATA, &webpage, false)
                .flatten()
                .unwrap_or_else(|| json!({}));
            if truthy(risk.get("v_voucher")) {
                return Err(fail(
                    &temp_id,
                    "You have exceeded the rate limit. Try again later",
                ));
            }
            let mut query = pairs(&[("platform", "web")]);
            if prefix == "BV" {
                query.push(("bvid".into(), format!("{prefix}{temp_id}")));
            } else {
                query.push(("aid".into(), temp_id.clone()));
            }
            let signed = sign_wbi(client, query, &temp_id)?;
            let url = with_query(
                "https://api.bilibili.com/x/web-interface/wbi/view/detail",
                &signed,
            )?;
            let detail = super::download_json_note_headers(
                client,
                &url,
                NAME,
                &temp_id,
                "Downloading redirection URL",
                &headers,
            )?;
            if let Some(new_url) = detail
                .pointer("/data/View/redirect_url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .filter(|url| BANGUMI_URL.is_match(url))
            {
                return Err(Error::UnsupportedHtml5(format!(
                    "Bilibili bangumi pages ({new_url}) are not supported natively"
                )));
            }
            return Err(fail(
                &temp_id,
                &format!(
                    "Unable to extract initial state; {}",
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        };
        match int(initial_state.pointer("/error/trueCode")) {
            Some(-403) => {
                return Err(fail(
                    &temp_id,
                    &format!("This video is only available for registered users. {LOGIN_HINT}"),
                ));
            }
            Some(-404) => {
                return Err(fail(
                    &temp_id,
                    "This video may be deleted or geo-restricted. You might want to try a VPN or a proxy server (with --proxy)",
                ));
            }
            _ => {}
        }
        let is_festival = initial_state.get("videoData").is_none();
        let data_key = if is_festival {
            "videoInfo"
        } else {
            "videoData"
        };
        let video_data = initial_state
            .get(data_key)
            .cloned()
            .ok_or_else(|| key_error(&temp_id, data_key))?;
        let video_id = str_or_none(video_data.get("bvid"))
            .filter(|_| video_data.get("bvid").is_some())
            .ok_or_else(|| key_error(&temp_id, "bvid"))?;
        let mut title = video_data.get("title").cloned().unwrap_or(Value::Null);

        let page_list: Vec<Value> = if is_festival {
            Vec::new()
        } else {
            let url = with_query(
                "https://api.bilibili.com/x/player/pagelist",
                &pairs(&[("bvid", &video_id), ("jsonp", "jsonp")]),
            )?;
            optional_json(
                client,
                &url,
                &video_id,
                "Extracting videos in anthology",
                &headers,
            )?
            .and_then(|list| list.get("data").and_then(Value::as_array).cloned())
            .unwrap_or_default()
        };
        let is_anthology = page_list.len() > 1;
        let mut part_id = page
            .query_pairs()
            .filter(|(key, value)| key == "p" && !value.is_empty())
            .last()
            .and_then(|(_, value)| value.trim().parse::<i64>().ok());
        if is_anthology && part_id.is_none_or(|part| part == 0) {
            let yes = if super::no_playlist() {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading just the video {video_id} because of --no-playlist"
                ));
                false
            } else {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading playlist {video_id} - add --no-playlist to download just the video {video_id}"
                ));
                true
            };
            if yes {
                let mut urls: Vec<String> = Vec::new();
                for entry in &page_list {
                    let Some(number) = str_or_none(entry.get("page")) else {
                        return Err(key_error(&temp_id, "page"));
                    };
                    let url = format!("https://www.bilibili.com/video/{video_id}?p={number}");
                    if !urls.contains(&url) {
                        urls.push(url);
                    }
                }
                let entries = urls
                    .iter()
                    .map(|url| part_entry(url))
                    .collect::<Result<Vec<_>>>()?;
                let mut extra = Map::new();
                let mut keys = vec!["id"];
                if !title.is_null() {
                    keys.push("title");
                }
                extra.insert(
                    crate::info_order::INFO_SHAPE.into(),
                    format!("playlist-dict;{}", keys.join(",")).into(),
                );
                return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                    id: video_id,
                    title: title.as_str().unwrap_or_default().to_owned(),
                    webpage_url: page.clone(),
                    extractor: NAME.into(),
                    extractor_key: NAME.into(),
                    nested: Vec::new(),
                    pending: None,
                    entries,
                    extra,
                })));
            }
        }
        if is_anthology {
            let part = part_id.filter(|part| *part != 0).unwrap_or(1);
            part_id = Some(part);
            let name = usize::try_from(part - 1)
                .ok()
                .and_then(|index| page_list.get(index))
                .and_then(|entry| entry.get("part"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            title = format!("{} p{part:02} {name}", title.as_str().unwrap_or_default()).into();
        }
        let part_id = part_id.filter(|part| *part != 0);
        let aid = video_data.get("aid").filter(|aid| !aid.is_null());
        let aid_text = str_or_none(aid);
        let old_video_id = aid_text
            .as_ref()
            .map(|aid| format!("{aid}_part{}", part_id.unwrap_or(1)));
        let cid = match part_id {
            Some(part) => usize::try_from(part - 1)
                .ok()
                .and_then(|index| video_data.pointer(&format!("/pages/{index}/cid"))),
            None => video_data.get("cid"),
        }
        .and_then(|cid| str_or_none(Some(cid)));

        let mut dict = Map::new();
        let set = |dict: &mut Map<String, Value>, key: &str, value: Option<Value>| {
            if let Some(value) = value.filter(|value| !value.is_null()) {
                dict.insert(key.into(), value);
            }
        };
        set(
            &mut dict,
            "uploader",
            initial_state.pointer("/upData/name").cloned(),
        );
        set(
            &mut dict,
            "uploader_id",
            str_or_none(initial_state.pointer("/upData/mid")).map(Value::from),
        );
        set(
            &mut dict,
            "like_count",
            int(initial_state.pointer("/videoData/stat/like")).map(Value::from),
        );
        let tags: Vec<Value> = initial_state
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| tag.get("tag_name").filter(|name| !name.is_null()).cloned())
            .collect();
        if !tags.is_empty() {
            dict.insert("tags".into(), Value::Array(tags));
        }
        set(
            &mut dict,
            "thumbnail",
            initial_state
                .pointer("/videoData/pic")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(Value::from),
        );
        if is_festival {
            set(
                &mut dict,
                "uploader",
                initial_state.pointer("/videoInfo/upName").cloned(),
            );
            set(
                &mut dict,
                "uploader_id",
                str_or_none(initial_state.pointer("/videoInfo/upMid")).map(Value::from),
            );
            set(
                &mut dict,
                "like_count",
                int(initial_state.pointer("/videoStatus/like")).map(Value::from),
            );
            set(
                &mut dict,
                "thumbnail",
                initial_state
                    .get("sectionEpisodes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|episode| {
                        episode.get("bvid").and_then(Value::as_str) == Some(video_id.as_str())
                    })
                    .find_map(|episode| episode.get("cover").filter(|cover| !cover.is_null()))
                    .cloned(),
            );
        }
        set(&mut dict, "description", video_data.get("desc").cloned());
        set(
            &mut dict,
            "timestamp",
            int(video_data.get("pubdate")).map(Value::from),
        );
        set(
            &mut dict,
            "view_count",
            int(video_data.get("viewCount"))
                .or_else(|| int(video_data.pointer("/stat/view")))
                .map(Value::from),
        );
        set(
            &mut dict,
            "comment_count",
            int(video_data.pointer("/stat/reply")).map(Value::from),
        );
        let id = match part_id {
            Some(part) => format!("{video_id}_p{part}"),
            None => video_id.clone(),
        };
        dict.insert("id".into(), id.clone().into());
        dict.insert(
            "_old_archive_ids".into(),
            old_video_id.map_or(Value::Null, |old| json!([format!("bilibili {old}")])),
        );
        dict.insert("title".into(), title.clone());
        dict.insert("http_headers".into(), json!({"Referer": page.as_str()}));

        if truthy(video_data.pointer("/rights/is_stein_gate")) {
            return Err(Error::UnsupportedHtml5(
                "Bilibili interactive videos are not supported natively".into(),
            ));
        }
        let cid_text = cid.clone().unwrap_or_else(|| "None".into());
        let mut play_info = super::search_json(&PLAY_INFO, &webpage, false)
            .flatten()
            .and_then(|info| info.get("data").filter(|data| data.is_object()).cloned());
        if (!logged_in() || play_info.is_none())
            && let Some(downloaded) = download_playinfo(
                client,
                &video_id,
                &cid_text,
                &headers,
                &[("try_look", "1")],
                false,
            )?
            && truthy(Some(&downloaded))
        {
            play_info = Some(downloaded);
        }
        let mut formats = extract_formats(play_info.as_ref());
        if truthy(video_data.get("is_upower_exclusive")) {
            let high_level = initial_state
                .pointer("/elecFullInfo/show_info/high_level")
                .filter(|level| level.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            let joined: Vec<String> = ["title", "sub_title"]
                .iter()
                .filter_map(|key| str_or_none(high_level.get(*key)))
                .filter(|text| !text.is_empty())
                .collect();
            let message = format!("{}. {LOGIN_HINT}", joined.join("，"));
            if formats.is_empty() {
                return Err(fail(
                    &temp_id,
                    &format!("This is a supporter-only video: {message}"),
                ));
            }
            let preview = play_info
                .as_ref()
                .and_then(|info| info.get("accept_description"))
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("试看")));
            if preview {
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: This is a supporter-only video, only the preview will be extracted: {message}"
                ));
            }
        }
        if !truthy(play_info.as_ref().and_then(|info| info.get("dash"))) {
            // Legacy formats: each missing `accept_quality` on its own.
            let qualities: Vec<i64> = play_info
                .as_ref()
                .and_then(|info| info.get("accept_quality"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_i64)
                .collect();
            let has_qn = |formats: &[Map<String, Value>], qn: &Value| {
                formats
                    .iter()
                    .any(|format| format.get("quality").is_some_and(|quality| quality == qn))
            };
            for qn in qualities {
                if has_qn(&formats, &json!(qn)) {
                    continue;
                }
                let info = download_playinfo(
                    client,
                    &video_id,
                    &cid_text,
                    &headers,
                    &[("qn", &qn.to_string())],
                    true,
                )?;
                let extra: Vec<Map<String, Value>> = extract_formats(info.as_ref())
                    .into_iter()
                    .filter(|format| {
                        !has_qn(&formats, format.get("quality").unwrap_or(&Value::Null))
                    })
                    .collect();
                formats.extend(extra);
            }
            if let Some(info) = &play_info {
                check_missing_formats(info, &formats);
            }
            let flv: Vec<&Map<String, Value>> = formats
                .iter()
                .filter(|format| truthy(format.get("fragments")))
                .collect();
            // FLV and MP4 formats cannot both be kept (FLV becomes a
            // `multi_video`): the `bilibili:prefer_multi_flv` extractor
            // argument keeps FLV, else MP4 is kept.
            let prefer_flv =
                crate::manifest_args::configuration_arg(NAME, "prefer_multi_flv", false)
                    .and_then(|values| values.into_iter().next());
            let mixed = !flv.is_empty() && flv.len() < formats.len();
            if let Some(prefer) = prefer_flv.filter(|_| mixed) {
                // `prefer_multi_flv=QUALITY` keeps that quality's formats
                // (`int()` failing like a mismatch), else the best FLV one.
                let wanted = prefer.parse::<i64>().ok();
                let chosen: Vec<Map<String, Value>> = formats
                    .iter()
                    .filter(|format| {
                        wanted.is_some() && format.get("quality").and_then(Value::as_i64) == wanted
                    })
                    .cloned()
                    .collect();
                formats = if chosen.is_empty() {
                    // `max(...)` keeps the first of equal qualities.
                    let best = flv
                        .iter()
                        .rev()
                        .max_by_key(|format| format.get("quality").and_then(Value::as_i64))
                        .map(|format| (*format).clone());
                    best.into_iter().collect()
                } else {
                    chosen
                };
            } else if mixed {
                let dropped: Vec<String> = flv
                    .iter()
                    .map(|format| {
                        let text = |key: &str| {
                            str_or_none(format.get(key)).unwrap_or_else(|| "None".into())
                        };
                        format!("{} ({})", text("format_note"), text("format_id"))
                    })
                    .collect();
                formats.retain(|format| !truthy(format.get("fragments")));
                crate::screen::screen(&format!(
                    "[{NAME}] Dropping incompatible flv format(s) {} since mp4 is available. To extract flv, pass --extractor-args \"bilibili:prefer_multi_flv\"",
                    dropped.join(", ")
                ));
            }
        }
        if formats
            .first()
            .is_some_and(|format| truthy(format.get("fragments")))
        {
            // FLV parts carry their own timestamps, which binary
            // concatenation corrupts: upstream's `multi_video` workaround.
            let duration = float(
                play_info.as_ref().and_then(|info| info.get("timelength")),
                1000.0,
            );
            return multi_part_flv(
                client,
                page,
                dict,
                &formats[0],
                duration,
                (&video_id, &cid_text),
            )
            .map(Some);
        }
        dict.insert("formats".into(), json!([]));
        dict.insert(
            "duration".into(),
            float(
                play_info.as_ref().and_then(|info| info.get("timelength")),
                1000.0,
            )
            .map_or(Value::Null, |duration| json!(duration)),
        );
        // `_get_chapters(aid, cid)`.
        let mut chapters = Value::Null;
        if let (Some(aid), Some(cid)) = (
            aid_text.as_ref().filter(|_| truthy(aid)),
            cid.as_ref().filter(|cid| cid.as_str() != "0"),
        ) {
            let url = with_query(
                "https://api.bilibili.com/x/player/wbi/v2",
                &pairs(&[("aid", aid), ("cid", cid)]),
            )?;
            let found = optional_json(
                client,
                &url,
                aid,
                "Extracting chapters",
                &[("Referer", "https://www.bilibili.com/")],
            )?;
            let points: Vec<Value> = found
                .as_ref()
                .and_then(|found| found.pointer("/data/view_points"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|point| {
                    let mut chapter = Map::new();
                    for (key, source) in [
                        ("title", "content"),
                        ("start_time", "from"),
                        ("end_time", "to"),
                    ] {
                        if let Some(value) = point.get(source).filter(|value| !value.is_null()) {
                            chapter.insert(key.into(), value.clone());
                        }
                    }
                    Value::Object(chapter)
                })
                .collect();
            if !points.is_empty() {
                chapters = Value::Array(points);
            }
        }
        dict.insert("chapters".into(), chapters);
        dict.insert(
            "subtitles".into(),
            if super::subtitles_requested() {
                subtitles(client, &video_id, &cid_text)?
            } else {
                json!({})
            },
        );

        let formats = formats
            .into_iter()
            .map(|format| format_from(format, &temp_id))
            .collect::<Result<Vec<_>>>()?;
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
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if title.is_null() {
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
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!(
                "dict;{};subtitle=ext,url;chapter=title,start_time,end_time",
                keys.join(",")
            ),
        ))))
    }
}

/// `{**metainfo, '_type': 'multi_video', 'entries': [...], 'duration'}`:
/// one entry per FLV part of the first format (its subtitles on the first
/// entry only).
fn multi_part_flv(
    client: &Client,
    page: &Url,
    mut metainfo: Map<String, Value>,
    format: &Map<String, Value>,
    duration: Option<f64>,
    (video_id, cid): (&str, &str),
) -> Result<ExtractResult> {
    let id = metainfo
        .shift_remove("id")
        .and_then(|id| id.as_str().map(str::to_owned))
        .unwrap_or_default();
    let title = metainfo.get("title").cloned().unwrap_or(Value::Null);
    let headers = metainfo.get("http_headers").cloned().unwrap_or(Value::Null);
    let format_id = format.get("format_id").cloned().unwrap_or(Value::Null);
    let parts = format
        .get("fragments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut entries = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let entry_id = format!("{id}_{index}");
        let mut fields: Vec<(&str, Value)> = part
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        fields.push(("format_id", format_id.clone()));
        let part_format = super::dict_format(fields, NAME, &entry_id)?;
        let subtitles = if index == 0 {
            if super::subtitles_requested() {
                subtitles(client, video_id, cid)?
            } else {
                json!({})
            }
        } else {
            Value::Null
        };
        let none = subtitles.is_null();
        let mut entry = super::dict_info(
            page,
            (NAME, NAME),
            vec![
                ("id", Value::from(entry_id.as_str())),
                ("title", title.clone()),
                ("http_headers", headers.clone()),
                ("formats", Value::Null),
                ("subtitles", subtitles),
            ],
            vec![part_format],
        );
        if none {
            // `'subtitles': None` stays in the dict.
            entry.extra.insert("subtitles".into(), Value::Null);
        }
        entries.push(entry);
    }
    let mut keys: Vec<String> = metainfo.keys().cloned().collect();
    keys.insert(
        keys.iter()
            .position(|key| key == "_old_archive_ids")
            .unwrap_or(keys.len()),
        "id".into(),
    );
    let title = metainfo
        .shift_remove("title")
        .and_then(|title| title.as_str().map(str::to_owned))
        .unwrap_or_default();
    metainfo.insert(
        "duration".into(),
        duration.map_or(Value::Null, |d| json!(d)),
    );
    metainfo.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{};after=duration", keys.join(",")).into(),
    );
    metainfo.insert(crate::info::MULTI_VIDEO.into(), Value::Bool(true));
    Ok(ExtractResult::Playlist(PlaylistInfo {
        id,
        title,
        webpage_url: page.clone(),
        extractor: NAME.into(),
        extractor_key: NAME.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra: metainfo,
    }))
}

/// `_get_subtitles(video_id, cid)`: the danmaku XML and the player's tracks.
fn subtitles(client: &Client, video_id: &str, cid: &str) -> Result<Value> {
    subtitles_with(
        client,
        NAME,
        video_id,
        &pairs(&[("bvid", video_id), ("cid", cid)]),
        cid,
    )
}

/// [`subtitles`] for a query of `bvid`/`cid` or `aid`/`cid` (bangumi episodes).
fn subtitles_with(
    client: &Client,
    name: &'static str,
    video_id: &str,
    query: &[(String, String)],
    cid: &str,
) -> Result<Value> {
    let url = with_query("https://api.bilibili.com/x/player/wbi/v2", query)?;
    let info = super::download_json_note_headers(
        client,
        &url,
        name,
        video_id,
        &format!("Extracting subtitle info {cid}"),
        &[("Referer", "https://www.bilibili.com/")],
    )?;
    if truthy(info.pointer("/data/need_login_subtitle")) {
        crate::screen::warning_once(&format!(
            "[{name}] Subtitles are only available when logged in. {LOGIN_HINT}"
        ));
    }
    let tracks = info
        .pointer("/data/subtitle/subtitles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|track| truthy(track.get("subtitle_url")) && truthy(track.get("lan")))
        .count();
    if tracks > 0 {
        return Err(Error::UnsupportedHtml5(
            "Bilibili SRT subtitles (inline data) are not supported natively".into(),
        ));
    }
    Ok(
        json!({"danmaku": [{"ext": "xml", "url": format!("https://comment.bilibili.com/{cid}.xml")}]}),
    )
}

// ---------------------------------------------------------------------------
// Bangumi (`BiliBiliBangumiIE`, `BiliBiliBangumiMediaIE`,
// `BiliBiliBangumiSeasonIE`): episodes from the embedded `playurlSSRData` (or
// the `pgc/player/web/v2/playurl` API), season and media pages as playlists of
// episode `url` results.
// ---------------------------------------------------------------------------

const BANGUMI_NAME: &str = "BiliBiliBangumi";
const BANGUMI_MEDIA_NAME: &str = "BiliBiliBangumiMedia";
const BANGUMI_SEASON_NAME: &str = "BiliBiliBangumiSeason";
static BANGUMI_EP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?bilibili\.com/bangumi/play/ep(\d+)").unwrap()
});
static BANGUMI_SS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?bilibili\.com/bangumi/play/ss(\d+)").unwrap()
});
static BANGUMI_MD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?bilibili\.com/bangumi/media/md(\d+)").unwrap()
});
static PLAYURL_SSR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"playurlSSRData\s*=").unwrap());
static LD_JSON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script[^>]+type="application/ld\+json"[^>]*>"#).unwrap());

/// `BiliBiliBangumiIE.suitable`.
pub fn bangumi_episode_suitable(url: &str) -> bool {
    BANGUMI_EP.is_match(url)
}

/// `BiliBiliBangumiMediaIE.suitable`.
pub fn bangumi_media_suitable(url: &str) -> bool {
    BANGUMI_MD.is_match(url)
}

/// `BiliBiliBangumiSeasonIE.suitable`.
pub fn bangumi_season_suitable(url: &str) -> bool {
    BANGUMI_SS.is_match(url)
}

fn named_fail(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{name}] {id}: {message}"))
}

fn named_key_error(name: &str, id: &str, key: &str) -> Error {
    named_fail(
        name,
        id,
        &format!(
            "An extractor error has occurred. (caused by KeyError('{key}')); {}",
            crate::BUG_REPORTS_MESSAGE
        ),
    )
}

fn captured_id(regex: &Regex, url: &str) -> Option<String> {
    regex.captures(url).map(|found| found[1].to_owned())
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `url_result(target, BiliBiliBangumiIE, id)` as a playlist entry.
fn bangumi_url_entry(target: &str, id: Option<String>, extractor: &str) -> Result<VideoInfo> {
    url_result_entry(target, BANGUMI_NAME, id, extractor, Vec::new())
}

/// `url_result(target, ie_key, id)` as a playlist entry, with `fields` set on
/// the entry dict (upstream's `entry['album'] = ...`).
fn url_result_entry(
    target: &str,
    ie_key: &str,
    id: Option<String>,
    extractor: &str,
    fields: Vec<(&str, Value)>,
) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ie_key.into());
    if let Some(id) = id {
        entry.insert("id".into(), id.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    for (key, value) in fields {
        entry.insert(key.into(), value);
    }
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": extractor,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `_get_episodes_from_season(ss_id, url)`.
fn bangumi_season_entries(
    client: &Client,
    name: &'static str,
    page: &Url,
    ss_id: &str,
) -> Result<Vec<VideoInfo>> {
    let url = with_query(
        "https://api.bilibili.com/pgc/web/season/section",
        &pairs(&[("season_id", ss_id)]),
    )?;
    let season_info = super::download_json_note_headers(
        client,
        &url,
        name,
        ss_id,
        "Downloading season info",
        &[("Referer", page.as_str())],
    )?;
    let mut entries = Vec::new();
    for entry in season_info
        .pointer("/result/main_section/episodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(share_url) = entry
            .get("share_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        if !truthy(entry.get("id")) {
            continue;
        }
        entries.push(bangumi_url_entry(
            &share_url,
            str_or_none(entry.get("id")),
            name,
        )?);
    }
    Ok(entries)
}

/// `playlist_result(...)`; `id_first` for the positional-argument form
/// (`playlist_result(entries, id, title, description)`), otherwise the
/// keyword form (`title`/`description` first, then the id).
fn bangumi_playlist(
    name: &'static str,
    page: &Url,
    id: &str,
    title: Option<String>,
    description: Option<String>,
    entries: Vec<VideoInfo>,
    id_first: bool,
) -> ExtractResult {
    let mut keys = Vec::new();
    let mut extra = Map::new();
    if id_first {
        keys.push("id");
    }
    if let Some(title) = &title {
        keys.push("title");
        extra.insert("title".into(), title.clone().into());
    }
    if let Some(description) = description {
        keys.push("description");
        extra.insert("description".into(), description.into());
    }
    if !id_first {
        keys.push("id");
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id: id.to_owned(),
        title: title.unwrap_or_default(),
        webpage_url: page.clone(),
        extractor: name.into(),
        extractor_key: name.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

/// `_search_json` on an embedded JSON-LD block (`ld+json`), fatal as upstream.
fn bangumi_json_ld(webpage: &str, name: &str, id: &str) -> Result<Value> {
    super::search_json(&LD_JSON, webpage, false)
        .flatten()
        .ok_or_else(|| {
            named_fail(
                name,
                id,
                &format!("Unable to extract info; {}", crate::BUG_REPORTS_MESSAGE),
            )
        })
}

/// Upstream's dict result for an episode: the keys are kept in upstream's
/// order and `formats` is filled from `formats`.
fn bangumi_video(
    name: &'static str,
    page: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
) -> ExtractResult {
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
    info.extractor_key = name.into();
    let mut keys = Vec::new();
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "formats" => {}
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    ExtractResult::Video(crate::info_order::mark_info(
        info,
        &format!("dict;{};subtitle=url,ext", keys.join(",")),
    ))
}

/// `BiliBiliBangumiIE._real_extract`.
fn bangumi_episode(client: &Client, page: &Url, episode_id: &str) -> Result<ExtractResult> {
    let name = BANGUMI_NAME;
    let referer = page.as_str().to_owned();
    let headers = [("Referer", referer.as_str())];
    let webpage = super::download_webpage(client, page, name, episode_id)?;
    if webpage.contains("您所在的地区无法观看本片") {
        return Err(named_fail(name, episode_id, "This video is restricted"));
    }
    if webpage.contains("正在观看预览，大会员免费看全片") {
        return Err(named_fail(
            name,
            episode_id,
            &format!("This video is for premium members only. {LOGIN_HINT}"),
        ));
    }

    let mut play_info = match super::search_json(&PLAYURL_SSR, &webpage, false).flatten() {
        Some(value) => value,
        None => {
            let url = with_query(
                "https://api.bilibili.com/pgc/player/web/v2/playurl",
                &pairs(&[("fnval", "12240"), ("ep_id", episode_id)]),
            )?;
            super::download_json_note_headers(
                client,
                &url,
                name,
                episode_id,
                "Extracting episode",
                &headers,
            )?
        }
    };
    // Three shapes: `result`/`video_info` at the top, under `raw`, or under `data`.
    let mut status_code = play_info.get("code").cloned();
    if let Some(raw) = play_info.get("raw").cloned() {
        play_info = raw;
    }
    if let Some(data) = play_info.get("data").cloned() {
        play_info = data;
    }
    if status_code.is_none() {
        status_code = play_info.get("code").cloned();
    }
    if let Some(result) = play_info.get("result").cloned() {
        play_info = result;
    }

    let geo_blocked = play_info
        .get("plugins")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|plugin| {
            plugin.get("name").and_then(Value::as_str) == Some("AreaLimitPanel")
                && truthy(plugin.pointer("/config/is_block"))
        });
    let premium_only = int(status_code.as_ref()) == Some(-10403);

    let video_info = play_info
        .get("video_info")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let formats = extract_formats(Some(&video_info))
        .into_iter()
        .map(|format| format_from(format, episode_id))
        .collect::<Result<Vec<_>>>()?;

    if formats.is_empty() {
        if geo_blocked {
            return Err(named_fail(
                name,
                episode_id,
                "This video is not available from your location due to geo restriction. You might want to use a VPN or a proxy server (with --proxy)",
            ));
        }
        if premium_only
            || webpage.contains("成为大会员抢先看")
            || webpage.contains("开通大会员观看")
        {
            return Err(named_fail(
                name,
                episode_id,
                &format!("This video is for premium members only. {LOGIN_HINT}"),
            ));
        }
    }
    let preview = ["play_check", "play_detail", "play_video_type"]
        .iter()
        .any(|key| {
            matches!(
                play_info.get(*key).and_then(Value::as_str),
                Some("PLAY_PREVIEW" | "preview")
            )
        });
    if preview {
        crate::screen::warning(&format!(
            "[{name}] Only preview format is available, you have to become a premium member to access full video. {LOGIN_HINT}"
        ));
    }

    let url = with_query(
        "https://api.bilibili.com/pgc/view/web/season",
        &pairs(&[("ep_id", episode_id)]),
    )?;
    let season = super::download_json_note_headers(
        client,
        &url,
        name,
        episode_id,
        "Get episode details",
        &headers,
    )?;
    let bangumi_info = season
        .get("result")
        .cloned()
        .ok_or_else(|| named_key_error(name, episode_id, "result"))?;

    // `episodes` of the main section, then those of every other section.
    let mut candidates: Vec<Value> = Vec::new();
    if let Some(list) = bangumi_info.get("episodes").and_then(Value::as_array) {
        candidates.extend(list.iter().cloned());
    }
    for section in bangumi_info
        .get("section")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(list) = section.get("episodes").and_then(Value::as_array) {
            candidates.extend(list.iter().cloned());
        }
    }
    let mut episode_number = 1;
    let mut episode_info = json!({});
    for (index, candidate) in candidates
        .iter()
        .filter(|value| value.is_object())
        .enumerate()
    {
        if str_or_none(candidate.get("id")).as_deref() == Some(episode_id) {
            episode_number = index as i64 + 1;
            episode_info = candidate.clone();
            break;
        }
    }

    let season_id = bangumi_info
        .get("season_id")
        .filter(|value| truthy(Some(value)))
        .cloned();
    let (season_number, season_title) = season_id
        .as_ref()
        .and_then(|id| {
            bangumi_info
                .get("seasons")
                .and_then(Value::as_array)
                .and_then(|seasons| {
                    seasons
                        .iter()
                        .enumerate()
                        .find(|(_, season)| season.get("season_id") == Some(id))
                        .map(|(index, season)| {
                            (
                                Some(index as i64 + 1),
                                str_or_none(season.get("season_title")),
                            )
                        })
                })
        })
        .unwrap_or((None, None));

    let mut dict = Map::new();
    dict.insert("id".into(), episode_id.into());
    dict.insert("formats".into(), Value::Null);
    let series = bangumi_info.get("series");
    for (key, value) in [
        (
            "series",
            series
                .and_then(|series| series.get("series_title"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
        (
            "series_id",
            str_or_none(series.and_then(|series| series.get("series_id"))),
        ),
        (
            "thumbnail",
            bangumi_info
                .get("square_cover")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none),
        ),
    ] {
        if let Some(value) = value {
            dict.insert(key.into(), value.into());
        }
    }
    if let Some(long_title) = str_or_none(episode_info.get("long_title")) {
        dict.insert("episode".into(), long_title.into());
    }
    let episode_index = int(episode_info.get("title"))
        .filter(|number| *number != 0)
        .unwrap_or(episode_number);
    dict.insert("episode_number".into(), episode_index.into());
    if let Some(timestamp) = int(episode_info.get("pub_time")) {
        dict.insert("timestamp".into(), timestamp.into());
    }
    if episode_info
        .as_object()
        .is_some_and(|object| !object.is_empty())
    {
        let title = [
            str_or_none(episode_info.get("title")),
            str_or_none(episode_info.get("long_title")),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        dict.insert("title".into(), title.into());
    }
    dict.insert("episode_id".into(), episode_id.into());
    dict.insert(
        "season".into(),
        season_title.map_or(Value::Null, Value::from),
    );
    dict.insert(
        "season_id".into(),
        str_or_none(season_id.as_ref()).map_or(Value::Null, Value::from),
    );
    dict.insert(
        "season_number".into(),
        season_number.map_or(Value::Null, Value::from),
    );
    dict.insert(
        "duration".into(),
        float(video_info.get("timelength"), 1000.0).map_or(Value::Null, Value::from),
    );
    let cid = str_or_none(episode_info.get("cid")).unwrap_or_else(|| "None".into());
    let subtitles = if super::subtitles_requested() {
        let query = match episode_info.get("aid").filter(|value| truthy(Some(value))) {
            Some(aid) => pairs(&[
                ("aid", str_or_none(Some(aid)).unwrap_or_default().as_str()),
                ("cid", cid.as_str()),
            ]),
            None => pairs(&[("bvid", episode_id), ("cid", cid.as_str())]),
        };
        subtitles_with(client, name, episode_id, &query, &cid)?
    } else {
        json!({})
    };
    dict.insert("subtitles".into(), subtitles);
    dict.insert("http_headers".into(), json!({"Referer": referer}));

    Ok(bangumi_video(name, page, dict, formats))
}

pub struct BiliBiliBangumi;

impl Extractor for BiliBiliBangumi {
    fn key(&self) -> &'static str {
        BANGUMI_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(episode_id) = captured_id(&BANGUMI_EP, page.as_str()) else {
            return Ok(None);
        };
        announce(BANGUMI_NAME, page);
        bangumi_episode(client, page, &episode_id).map(Some)
    }
}

pub struct BiliBiliBangumiSeason;

impl Extractor for BiliBiliBangumiSeason {
    fn key(&self) -> &'static str {
        BANGUMI_SEASON_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(ss_id) = captured_id(&BANGUMI_SS, page.as_str()) else {
            return Ok(None);
        };
        announce(BANGUMI_SEASON_NAME, page);
        let webpage = super::download_webpage(client, page, BANGUMI_SEASON_NAME, &ss_id)?;
        let ld = bangumi_json_ld(&webpage, BANGUMI_SEASON_NAME, &ss_id)?;
        let metainfo = ld
            .get("itemListElement")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|item| item.is_object()));
        let title = metainfo.and_then(|item| str_or_none(item.get("name")));
        let description = metainfo.and_then(|item| str_or_none(item.get("description")));
        let entries = bangumi_season_entries(client, BANGUMI_SEASON_NAME, page, &ss_id)?;
        Ok(Some(bangumi_playlist(
            BANGUMI_SEASON_NAME,
            page,
            &ss_id,
            title,
            description,
            entries,
            false,
        )))
    }
}

pub struct BiliBiliBangumiMedia;

impl Extractor for BiliBiliBangumiMedia {
    fn key(&self) -> &'static str {
        BANGUMI_MEDIA_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(media_id) = captured_id(&BANGUMI_MD, page.as_str()) else {
            return Ok(None);
        };
        announce(BANGUMI_MEDIA_NAME, page);
        let webpage = super::download_webpage(client, page, BANGUMI_MEDIA_NAME, &media_id)?;
        let state = super::search_json(&INITIAL_STATE, &webpage, false)
            .flatten()
            .ok_or_else(|| {
                named_fail(
                    BANGUMI_MEDIA_NAME,
                    &media_id,
                    &format!(
                        "Unable to extract initial_state; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ),
                )
            })?;
        let media = state
            .get("mediaInfo")
            .ok_or_else(|| named_key_error(BANGUMI_MEDIA_NAME, &media_id, "mediaInfo"))?;
        let ss_id = str_or_none(media.get("season_id"))
            .ok_or_else(|| named_key_error(BANGUMI_MEDIA_NAME, &media_id, "season_id"))?;
        let entries = bangumi_season_entries(client, BANGUMI_MEDIA_NAME, page, &ss_id)?;
        Ok(Some(bangumi_playlist(
            BANGUMI_MEDIA_NAME,
            page,
            &media_id,
            str_or_none(media.get("title")),
            str_or_none(media.get("evaluate")),
            entries,
            false,
        )))
    }
}

// ---------------------------------------------------------------------------
// Audio (`BilibiliAudioIE`, `BilibiliAudioAlbumIE`): the music-service API.
// ---------------------------------------------------------------------------

const AUDIO_NAME: &str = "BilibiliAudio";
const AUDIO_ALBUM_NAME: &str = "BilibiliAudioAlbum";
static AUDIO_AU: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?bilibili\.com/audio/au(\d+)").unwrap());
static AUDIO_AM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?bilibili\.com/audio/am(\d+)").unwrap());

/// `BilibiliAudioIE.suitable`.
pub fn audio_suitable(url: &str) -> bool {
    AUDIO_AU.is_match(url)
}

/// `BilibiliAudioAlbumIE.suitable`.
pub fn audio_album_suitable(url: &str) -> bool {
    AUDIO_AM.is_match(url)
}

/// `BilibiliAudioBaseIE._call_api`: the response's `data`.
fn audio_call(
    client: &Client,
    name: &'static str,
    path: &str,
    sid: &str,
    query: Option<Vec<(String, String)>>,
) -> Result<Value> {
    let query = query.unwrap_or_else(|| pairs(&[("sid", sid)]));
    let url = with_query(
        &format!("https://www.bilibili.com/audio/music-service-c/web/{path}"),
        &query,
    )?;
    let response = super::download_json(client, &url, name, sid)?;
    response
        .get("data")
        .cloned()
        .ok_or_else(|| named_key_error(name, sid, "data"))
}

fn dict_value(value: Option<&Value>) -> Value {
    value.cloned().unwrap_or(Value::Null)
}

pub struct BilibiliAudio;

impl Extractor for BilibiliAudio {
    fn key(&self) -> &'static str {
        AUDIO_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(au_id) = captured_id(&AUDIO_AU, page.as_str()) else {
            return Ok(None);
        };
        announce(AUDIO_NAME, page);
        let play_data = audio_call(client, AUDIO_NAME, "url", &au_id, None)?;
        let cdn = play_data
            .get("cdns")
            .and_then(Value::as_array)
            .and_then(|cdns| cdns.first())
            .cloned()
            .ok_or_else(|| named_key_error(AUDIO_NAME, &au_id, "cdns"))?;
        let mut format = Map::new();
        format.insert("url".into(), cdn);
        format.insert(
            "filesize".into(),
            int(play_data.get("size")).map_or(Value::Null, Value::from),
        );
        format.insert("vcodec".into(), "none".into());
        format.insert("http_headers".into(), json!({"Referer": page.as_str()}));
        let formats = vec![format_from(format, &au_id)?];

        let song = audio_call(client, AUDIO_NAME, "song/info", &au_id, None)?;
        let title = song
            .get("title")
            .cloned()
            .ok_or_else(|| named_key_error(AUDIO_NAME, &au_id, "title"))?;
        let statistic = song
            .get("statistic")
            .filter(|value| truthy(Some(value)))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let lyric = song
            .get("lyric")
            .filter(|value| truthy(Some(value)))
            .cloned();
        let subtitles = lyric.map_or(Value::Null, |lyric| {
            let ext = lyric
                .as_str()
                .and_then(super::direct::determine_ext)
                .unwrap_or_default();
            json!({"origin": [{"url": lyric, "ext": ext}]})
        });

        let mut dict = Map::new();
        dict.insert("id".into(), au_id.as_str().into());
        dict.insert("title".into(), title);
        dict.insert("formats".into(), Value::Null);
        dict.insert("artist".into(), dict_value(song.get("author")));
        dict.insert(
            "comment_count".into(),
            int(statistic.get("comment")).map_or(Value::Null, Value::from),
        );
        dict.insert("description".into(), dict_value(song.get("intro")));
        dict.insert(
            "duration".into(),
            int(song.get("duration")).map_or(Value::Null, Value::from),
        );
        dict.insert("subtitles".into(), subtitles);
        dict.insert("thumbnail".into(), dict_value(song.get("cover")));
        dict.insert(
            "timestamp".into(),
            int(song.get("passtime")).map_or(Value::Null, Value::from),
        );
        dict.insert("uploader".into(), dict_value(song.get("uname")));
        dict.insert(
            "view_count".into(),
            int(statistic.get("play")).map_or(Value::Null, Value::from),
        );
        Ok(Some(bangumi_video(AUDIO_NAME, page, dict, formats)))
    }
}

pub struct BilibiliAudioAlbum;

impl Extractor for BilibiliAudioAlbum {
    fn key(&self) -> &'static str {
        AUDIO_ALBUM_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(am_id) = captured_id(&AUDIO_AM, page.as_str()) else {
            return Ok(None);
        };
        announce(AUDIO_ALBUM_NAME, page);
        let listing = audio_call(
            client,
            AUDIO_ALBUM_NAME,
            "song/of-menu",
            &am_id,
            Some(pairs(&[
                ("sid", am_id.as_str()),
                ("pn", "1"),
                ("ps", "100"),
            ])),
        )?;
        let songs = listing
            .get("data")
            .ok_or_else(|| named_key_error(AUDIO_ALBUM_NAME, &am_id, "data"))?;
        let sids: Vec<String> = songs
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|song| str_or_none(song.get("id")).filter(|sid| !sid.is_empty()))
            .collect();

        // `album_data = self._call_api('menu/info', am_id) or {}`: the album's
        // title (its entries carry it) and intro, when there are songs.
        let (album_title, album_intro) = if sids.is_empty() {
            (None, None)
        } else {
            let album = audio_call(client, AUDIO_ALBUM_NAME, "menu/info", &am_id, None)?;
            let album = if truthy(Some(&album)) {
                album
            } else {
                json!({})
            };
            (
                str_or_none(album.get("title")).filter(|title| !title.is_empty()),
                str_or_none(album.get("intro")),
            )
        };
        let entries = sids
            .into_iter()
            .map(|sid| {
                let target = format!("https://www.bilibili.com/audio/au{sid}");
                let fields = album_title
                    .iter()
                    .map(|title| ("album", Value::from(title.as_str())))
                    .collect();
                url_result_entry(&target, AUDIO_NAME, Some(sid), AUDIO_ALBUM_NAME, fields)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(bangumi_playlist(
            AUDIO_ALBUM_NAME,
            page,
            &am_id,
            album_title,
            album_intro,
            entries,
            true,
        )))
    }
}

// ---------------------------------------------------------------------------
// Space audio (`BilibiliSpaceAudioIE`): a user's uploaded songs, paged.
// ---------------------------------------------------------------------------

const SPACE_AUDIO_NAME: &str = "BilibiliSpaceAudio";
static SPACE_AUDIO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://space\.bilibili\.com/(\d+)/(?:upload/)?audio").unwrap()
});

/// `BilibiliSpaceAudioIE.suitable`.
pub fn space_audio_suitable(url: &str) -> bool {
    SPACE_AUDIO.is_match(url)
}

pub struct BilibiliSpaceAudio;

impl Extractor for BilibiliSpaceAudio {
    fn key(&self) -> &'static str {
        SPACE_AUDIO_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(uid) = captured_id(&SPACE_AUDIO, page.as_str()) else {
            return Ok(None);
        };
        announce(SPACE_AUDIO_NAME, page);
        let fetch_page = |index: usize| -> Result<Value> {
            let url = with_query(
                "https://api.bilibili.com/audio/music-service/web/song/upper",
                &pairs(&[
                    ("uid", uid.as_str()),
                    ("pn", &(index + 1).to_string()),
                    ("ps", "30"),
                    ("order", "1"),
                    ("jsonp", "jsonp"),
                ]),
            )?;
            let response = super::download_json_note(
                client,
                &url,
                SPACE_AUDIO_NAME,
                &uid,
                &format!("Downloading page {index}"),
            )?;
            response
                .get("data")
                .cloned()
                .ok_or_else(|| named_key_error(SPACE_AUDIO_NAME, &uid, "data"))
        };
        let first_page = fetch_page(0)?;
        let page_count = int(first_page.get("pageCount"))
            .ok_or_else(|| named_key_error(SPACE_AUDIO_NAME, &uid, "pageCount"))?;

        let mut entries = Vec::new();
        for index in 0..page_count.max(0) {
            let data = if index == 0 {
                first_page.clone()
            } else {
                fetch_page(index as usize)?
            };
            // `data` is None when the playlist is empty.
            for song in data
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let sid = str_or_none(song.get("id")).unwrap_or_default();
                entries.push(url_result_entry(
                    &format!("https://www.bilibili.com/audio/au{sid}"),
                    AUDIO_NAME,
                    Some(sid),
                    SPACE_AUDIO_NAME,
                    Vec::new(),
                )?);
            }
        }
        Ok(Some(bangumi_playlist(
            SPACE_AUDIO_NAME,
            page,
            &uid,
            None,
            None,
            entries,
            true,
        )))
    }
}

#[cfg(test)]
mod bangumi_tests {
    use super::*;

    #[test]
    fn bangumi_urls_follow_upstream_valid_url() {
        assert!(bangumi_episode_suitable(
            "https://www.bilibili.com/bangumi/play/ep21495/"
        ));
        assert!(bangumi_season_suitable(
            "https://www.bilibili.com/bangumi/play/ss2251"
        ));
        assert!(bangumi_media_suitable(
            "https://www.bilibili.com/bangumi/media/md1565/"
        ));
        assert!(!bangumi_season_suitable(
            "https://www.bilibili.com/bangumi/play/ep21495/"
        ));
        assert!(!bangumi_episode_suitable(
            "https://www.bilibili.com/bangumi/media/md1565/"
        ));
        assert!(audio_suitable("https://www.bilibili.com/audio/au1003142"));
        assert!(audio_album_suitable(
            "https://www.bilibili.com/audio/am10624"
        ));
        assert!(!audio_suitable("https://www.bilibili.com/audio/am10624"));
        assert!(space_audio_suitable(
            "https://space.bilibili.com/313580179/upload/audio"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.bilibili.com/video/BV13x41117TL"));
        assert!(suitable("https://www.bilibili.com/video/av8903802/"));
        assert!(suitable(
            "https://www.bilibili.com/festival/bh3-7th?bvid=BV1tr4y1f7p2&"
        ));
        assert!(!suitable("https://www.bilibili.com/bangumi/play/ep267851"));
    }

    #[test]
    fn urlencode_follows_quote_plus() {
        assert_eq!(
            urlencode(&pairs(&[("dm_img_inter", "{\"ds\":[]}"), ("a", "b c")])),
            "dm_img_inter=%7B%22ds%22%3A%5B%5D%7D&a=b+c"
        );
    }
}
