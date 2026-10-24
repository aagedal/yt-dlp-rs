//! Upstream `yt_dlp/extractor/instagram.py` without browser impersonation,
//! like upstream without `curl_cffi`:
//!
//! - `InstagramIE`: with a `sessionid` cookie the media info API (falling
//!   back to the logged-out path when the cookies are rejected); otherwise the
//!   session's LSD token, the post accessibility check, then the post page's
//!   prefetched `xig_polaris_media`. Both go through `_extract_product`
//!   (formats, an inline `video_dash_manifest`, thumbnails, metadata,
//!   carousel playlists and, logged in with `--write-comments`, the comments
//!   API as the `__post_extractor`).
//! - `InstagramIOSIE`: `instagram://media?id=PK` to its post.
//! - `InstagramStoryIE`: the stories/highlights page's user, then the
//!   `feed/reels_media` API.
//! - `InstagramUserIE` (marked broken upstream) and `InstagramTagIE`: the
//!   page's `sharedData` and the GIS-signed GraphQL timeline pages.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct Instagram;
pub struct InstagramIos;
pub struct InstagramStory;
pub struct InstagramTag;
pub struct InstagramUser;

const NAME: &str = "Instagram";
const IOS_NAME: &str = "InstagramIOS";
const STORY_NAME: &str = "instagram:story";
const STORY_KEY: &str = "InstagramStory";
const TAG_NAME: &str = "instagram:tag";
const TAG_KEY: &str = "InstagramTag";
const USER_NAME: &str = "instagram:user";
const USER_KEY: &str = "InstagramUser";
const ENCODING_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
/// `_APP_IDS`.
const WEB_APP_ID: &str = "936619743392459";
const IOS_APP_ID: &str = "124024574287414";
/// `_COOKIE_DOMAINS`.
const COOKIE_DOMAINS: [&str; 6] = [
    "i.instagram.com",
    ".i.instagram.com",
    "www.instagram.com",
    ".www.instagram.com",
    "instagram.com",
    ".instagram.com",
];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<url>https?://(?:www\.)?instagram\.com(?:/[^/?#]+)?/(?:p|tv|reels?)/(?P<id>[^/?#&]+))",
    )
    .unwrap()
});
static IOS_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^instagram://media\?id=(?P<id>[\d_]+)").unwrap());
static STORY_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?instagram\.com/stories/(?P<user>[^/?#]+)(?:/(?P<id>\d+))?")
        .unwrap()
});
static TAG_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?instagram\.com/explore/tags/(?P<id>[^/]+)").unwrap()
});
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?instagram\.com/(?P<id>[^/]{2,})/?(?:$|[?#])").unwrap()
});
static SJS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<script\b[^>]+\bdata-sjs>(\{.+?\})</script>").unwrap());
static EQMC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script\b[^>]*\bid="__eqmc"[^>]*>"#).unwrap());
static LSD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\["LSD",\[\],\{"token":"([^"]+)""#).unwrap());
static STORY_USER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""user":"#).unwrap());
static SHARED_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)sharedData\s*=\s*(\{.+?\})\s*;\s*[<\n]").unwrap());
/// `SegmentBase` elements, which upstream's MPD parser does not read.
static SEGMENT_BASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<SegmentBase\b[^>]*?/>|<SegmentBase\b[^>]*>.*?</SegmentBase>").unwrap()
});

/// `_match_valid_url` (`(?!share/)` and `reels?(?!/audio/)` checked apart).
pub fn suitable(url: &str) -> bool {
    let Some(found) = VALID_URL.captures(url) else {
        return false;
    };
    let path = url.split_once("instagram.com").map_or("", |(_, path)| path);
    let after_kind = &url[found.name("id").map_or(0, |id| id.start())..];
    !path.starts_with("/share/")
        && !(path.contains("/reel/audio/") || after_kind.starts_with("audio/"))
}

/// `InstagramIOSIE._VALID_URL`.
pub fn ios_suitable(url: &str) -> bool {
    IOS_URL.is_match(url)
}

/// `InstagramStoryIE._VALID_URL`.
pub fn story_suitable(url: &str) -> bool {
    STORY_URL.is_match(url)
}

/// `InstagramTagIE._VALID_URL`.
pub fn tag_suitable(url: &str) -> bool {
    TAG_URL.is_match(url)
}

/// `InstagramUserIE._VALID_URL`.
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

/// `_id_to_pk`: a shortcode (without a private post's 28 extra characters).
fn id_to_pk(shortcode: &str) -> u128 {
    let shortcode = if shortcode.len() > 28 {
        &shortcode[..shortcode.len() - 28]
    } else {
        shortcode
    };
    shortcode.bytes().fold(0u128, |pk, byte| {
        let digit = ENCODING_CHARS
            .iter()
            .position(|known| *known == byte)
            .unwrap_or(0) as u128;
        pk.wrapping_mul(64).wrapping_add(digit)
    })
}

/// `_pk_to_id`.
fn pk_to_id(pk: &Value) -> Option<String> {
    let text = match pk {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    let mut pk: u128 = text.split('_').next()?.parse().ok()?;
    if pk == 0 {
        return Some("A".into());
    }
    let mut digits = Vec::new();
    while pk > 0 {
        digits.push(ENCODING_CHARS[(pk % 64) as usize]);
        pk /= 64;
    }
    digits.reverse();
    String::from_utf8(digits).ok()
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.into()),
        _ => None,
    }
}

fn url_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

const LOGIN_HINT: &str = "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";

fn fail(id: &str, message: &str) -> Error {
    fail_as(NAME, id, message)
}

fn fail_as(name: &str, id: &str, message: &str) -> Error {
    if id.is_empty() {
        Error::Reported(format!("[{name}] {message}"))
    } else {
        Error::Reported(format!("[{name}] {id}: {message}"))
    }
}

fn login_required(id: &str, message: &str) -> Error {
    fail(id, &format!("{message}. {LOGIN_HINT}"))
}

/// `raise_login_required(message)` (without a video ID).
fn login_required_as(name: &str, message: &str) -> Error {
    Error::Reported(format!("[{name}] {message}. {LOGIN_HINT}"))
}

/// `_app_id`: the `instagram:app_id` extractor argument (`ios`, `web` or an
/// ID), else the web app's.
fn app_id() -> String {
    let input = crate::manifest_args::configuration_arg("instagram", "app_id", false)
        .and_then(|values| values.into_iter().next());
    match input.as_deref() {
        Some("ios") => IOS_APP_ID.into(),
        Some("web") | Some("") | None => WEB_APP_ID.into(),
        Some(other) => other.into(),
    }
}

/// `_API_BASE_URL`.
fn api_base() -> &'static str {
    if app_id() == WEB_APP_ID {
        "https://www.instagram.com/api/v1"
    } else {
        "https://i.instagram.com/api/v1"
    }
}

/// `_api_headers`.
fn api_headers() -> Vec<(&'static str, String)> {
    vec![
        ("X-IG-App-ID", app_id()),
        ("X-ASBD-ID", "359341".into()),
        ("X-IG-WWW-Claim", "0".into()),
        ("Origin", "https://www.instagram.com".into()),
        ("Accept", "*/*".into()),
    ]
}

fn header_refs<'a>(headers: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect()
}

/// `_is_logged_in`: a `sessionid` cookie for `_BASE_URL`.
fn is_logged_in() -> bool {
    crate::cookies::get_cookie("https://www.instagram.com/", "sessionid").is_some()
}

/// `_is_login_redirect`.
fn login_redirect(url: &Url) -> bool {
    url.path().starts_with("/accounts/login") || url.path() == "/"
}

/// `ERRNOTE: ERR (caused by REPR)` of a failed request.
fn request_failure(name: &str, id: &str, errnote: &str, error: &Error) -> Error {
    let text = crate::upstream_error_text(error);
    let status = match error {
        Error::Http(source) | Error::HttpRetryAfter { source, .. } => source.status(),
        _ => None,
    };
    let cause = match status {
        Some(_) => format!("<{}>", text.replacen("HTTP Error ", "HTTPError ", 1)),
        None => format!("TransportError({})", crate::batch::py_repr(&text)),
    };
    fail_as(name, id, &format!("{errnote}: {text} (caused by {cause})"))
}

/// The final URL of a failed HTTP request.
fn error_url(error: &Error) -> Option<&Url> {
    match error {
        Error::Http(source) | Error::HttpRetryAfter { source, .. } => source.url(),
        Error::ExtractorRequest { source, .. } => error_url(source),
        _ => None,
    }
}

fn error_status(error: &Error) -> Option<u16> {
    match error {
        Error::Http(source) | Error::HttpRetryAfter { source, .. } => {
            source.status().map(|status| status.as_u16())
        }
        Error::ExtractorRequest { source, .. } => error_status(source),
        _ => None,
    }
}

/// A request's body (`--dump-pages`/`--write-pages` like other pages).
fn read_body(response: reqwest::blocking::Response, id: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let page_url = response.url().clone();
    let mut response = response;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    Ok(body)
}

/// A non-fatal `_download_json(..., fatal=False)` with the API headers:
/// `None` (after a warning with `errnote`, unless it is `None`) on failure.
fn api_json_optional(
    client: &Client,
    url: &Url,
    name: &str,
    id: &str,
    note: &str,
    errnote: Option<&str>,
) -> Option<Value> {
    crate::screen::screen(&format!("[{name}] {id}: {note}"));
    let headers = api_headers();
    let response =
        match super::direct::request_with_headers(client, url, None, &header_refs(&headers)) {
            Ok(response) => response,
            Err(error) => {
                if let Some(errnote) = errnote {
                    crate::screen::warning(&format!(
                        "[{name}] {id}: {errnote}: {}",
                        crate::upstream_error_text(&error)
                    ));
                }
                return None;
            }
        };
    let body = read_body(response, id).ok()?;
    match serde_json::from_slice::<Value>(&body) {
        Ok(value) => Some(value),
        Err(error) => {
            crate::screen::warning(&format!(
                "[{name}] {id}: Failed to parse JSON  (caused by {error})"
            ));
            None
        }
    }
}

/// `_real_initialize`: the logged-out session's LSD token.
fn lsd_token(client: &Client) -> Result<String> {
    static TOKEN: Mutex<Option<String>> = Mutex::new(None);
    if let Some(token) = TOKEN.lock().ok().and_then(|token| token.clone()) {
        return Ok(token);
    }
    crate::screen::screen(&format!("[{NAME}] Setting up session"));
    let home = Url::parse("https://www.instagram.com/")?;
    let webpage = super::download_webpage_quiet(client, &home, NAME, "")?;
    let eqmc = match super::search_json(&EQMC, &webpage, false) {
        Some(Some(value)) => value,
        _ => Value::Null,
    };
    let token = eqmc
        .get("l")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| LSD.captures(&webpage).map(|found| found[1].to_owned()))
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] Unable to extract LSD token; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    if let Ok(mut slot) = TOKEN.lock() {
        *slot = Some(token.clone());
    }
    Ok(token)
}

/// `_parse_mpd_formats(self._parse_xml(dash, video_id), mpd_id='dash')` of
/// an inline manifest: without a manifest URL, so only representations
/// with an absolute direct `BaseURL` (`SegmentBase` indexes are not read,
/// like upstream) are represented; they are plain HTTPS formats.
fn dash_formats(manifest: &str, video_id: &str) -> Result<Vec<VideoFormat>> {
    // A base no representation can resolve against silently: relative
    // `BaseURL`s and segment addressing need the (missing) manifest URL.
    const UNRESOLVED: &str = "https://unresolved.invalid/instagram/manifest.mpd";
    let unsupported = |reason: &str| {
        Error::UnsupportedHtml5(format!(
            "Instagram inline DASH manifest of {video_id}: {reason}"
        ))
    };
    let text = SEGMENT_BASE.replace_all(manifest, "");
    let base = Url::parse(UNRESOLVED)?;
    let (parsed, subtitles) = crate::dash::parse_with_subtitles(&text, &base)?;
    if !subtitles.is_empty() {
        crate::screen::warning_once(&format!(
            "[{NAME}] Ignoring subtitle tracks found in the DASH manifest; if any subtitle tracks are missing, {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    let mut formats: Vec<VideoFormat> = Vec::new();
    for mut format in parsed {
        // The parser gives an unsegmented representation as its direct URL.
        let direct = match format.fragments.as_slice() {
            [] => Some(format.url.clone()),
            [fragment] if fragment.byte_range.is_none() && fragment.path.is_none() => {
                Some(fragment.url.clone())
            }
            _ => None,
        };
        let Some(direct) = direct.filter(|url| {
            url.host_str() != base.host_str() && !format.extra.contains_key("fragment_base_url")
        }) else {
            return Err(unsupported(
                "representations without an absolute direct BaseURL are not supported natively",
            ));
        };
        // `stream_numbers[f['url']]`: by the representation's own URL.
        let stream = formats.iter().filter(|other| other.url == direct).count();
        format.url = direct;
        format.fragments.clear();
        // Determined from the URL in processing, like upstream.
        format.protocol = None;
        format.manifest_stream_number = Some(stream);
        format.format_id = format!("dash-{}", format.format_id);
        format.extra.insert("manifest_url".into(), Value::Null);
        // Upstream's MPD format dict keys (`**codecs` included).
        crate::info_order::mark_format(
            &mut format,
            "dict:format_id,manifest_url,ext,width,height,tbr,asr,fps,language,format_note,filesize,container,vcodec,acodec,dynamic_range,url,manifest_stream_number,is_dash_periods",
        );
        formats.push(format);
    }
    Ok(formats)
}

/// `_extract_product_media`: its dict, and the inline DASH formats (which
/// follow the dict's formats).
fn product_media(media: &Value) -> Result<(Map<String, Value>, Vec<VideoFormat>)> {
    let mut formats = Vec::new();
    for version in media
        .get("video_versions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|version| url_value(version.get("url")).is_some())
    {
        let mut format = Map::new();
        format.insert(
            "url".into(),
            version.get("url").cloned().unwrap_or(Value::Null),
        );
        // Pinned upstream's `(('id', {str}), ('type', {int}, {str_or_none}),
        // any)` yields a present `id` as it is and never the `type`.
        if let Some(format_id) = version.get("id").filter(|id| !id.is_null()) {
            format.insert("format_id".into(), format_id.clone());
        }
        if let Some(width) = int(version.get("width")) {
            format.insert("width".into(), width.into());
        }
        if let Some(height) = int(version.get("height")) {
            format.insert("height".into(), height.into());
        }
        if let Some(codec) = media.get("video_codec").and_then(Value::as_str) {
            format.insert("vcodec".into(), codec.into());
        }
        if media.get("has_audio") == Some(&Value::Bool(false)) {
            format.insert("acodec".into(), "none".into());
        }
        formats.push(Value::Object(format));
    }
    let id = media.get("pk").and_then(pk_to_id);
    let dash = match media
        .get("video_dash_manifest")
        .and_then(Value::as_str)
        .filter(|manifest| !manifest.is_empty())
    {
        Some(manifest) => dash_formats(manifest, id.as_deref().unwrap_or("None"))?,
        None => Vec::new(),
    };
    let thumbnails: Vec<Value> = media
        .pointer("/image_versions2/candidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|candidate| url_value(candidate.get("url")).is_some())
        .map(|candidate| {
            let mut thumbnail = Map::new();
            thumbnail.insert(
                "url".into(),
                candidate.get("url").cloned().unwrap_or(Value::Null),
            );
            if let Some(width) = candidate.get("width").filter(|width| width.is_i64()) {
                thumbnail.insert("width".into(), width.clone());
            }
            if let Some(height) = candidate.get("height") {
                thumbnail.insert("height".into(), height.clone());
            }
            Value::Object(thumbnail)
        })
        .rev()
        .collect();
    let mut info = Map::new();
    info.insert("id".into(), id.map_or(Value::Null, Value::from));
    info.insert("formats".into(), Value::Array(formats));
    info.insert(
        "duration".into(),
        media
            .get("video_duration")
            .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
            .map_or(Value::Null, |duration| json!(duration)),
    );
    info.insert("thumbnails".into(), Value::Array(thumbnails));
    Ok((info, dash))
}

/// `_extract_product`'s shared `info_dict`.
fn product_info(product: &Value, video_id: Option<&str>) -> Map<String, Value> {
    let mut info = Map::new();
    info.insert("id".into(), video_id.map_or(Value::Null, Value::from));
    let username = product.pointer("/user/username").and_then(Value::as_str);
    info.insert(
        "title".into(),
        username.map(|name| format!("Video by {name}")).into(),
    );
    if let Some(id) = product.get("pk").and_then(pk_to_id) {
        info.insert("id".into(), id.into());
    }
    if let Some(title) = product.get("title").and_then(Value::as_str) {
        info.insert("title".into(), title.into());
    }
    if let Some(description) = product.pointer("/caption/text").and_then(Value::as_str) {
        info.insert("description".into(), description.into());
    }
    if let Some(channel) = username {
        info.insert("channel".into(), channel.into());
    }
    if let Some(uploader_id) = str_or_none(product.pointer("/user/pk")) {
        info.insert("uploader_id".into(), uploader_id.into());
    }
    if let Some(uploader) = product.pointer("/user/full_name").and_then(Value::as_str) {
        info.insert("uploader".into(), uploader.into());
    }
    for (key, source) in [
        ("timestamp", "taken_at"),
        ("view_count", "view_count"),
        ("like_count", "like_count"),
        ("comment_count", "comment_count"),
    ] {
        if let Some(value) = int(product.get(source)) {
            info.insert(key.into(), value.into());
        }
    }
    info.insert(
        "http_headers".into(),
        json!({"Referer": "https://www.instagram.com/"}),
    );
    info
}

/// A media dict with its inline DASH formats.
type Media = (Map<String, Value>, Vec<VideoFormat>);

/// `_extract_product(product_info, video_id)` before its comments.
enum Product {
    /// `{**info_dict, **_extract_product_media(product_info)}`.
    Single(Media),
    /// The carousel's `info_dict` and its entries'
    /// `{**info_dict, **_extract_product_media(item)}`.
    Carousel(Map<String, Value>, Vec<Media>),
}

fn extract_product(product: &Value, video_id: Option<&str>) -> Result<Product> {
    let product = match product {
        Value::Array(items) => items.first().unwrap_or(&Value::Null),
        other => other,
    };
    let info = product_info(product, video_id);
    let carousel: Vec<&Value> = product
        .get("carousel_media")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.is_object())
        .collect();
    if !carousel.is_empty() {
        let mut entries = Vec::new();
        for item in carousel {
            let mut dict = info.clone();
            let (media, dash) = product_media(item)?;
            dict.extend(media);
            entries.push((dict, dash));
        }
        return Ok(Product::Carousel(info, entries));
    }
    let mut dict = info;
    let (media, dash) = product_media(product)?;
    dict.extend(media);
    Ok(Product::Single((dict, dash)))
}

/// A video from upstream's dict (`dict;KEYS` order); the DASH formats
/// follow the dict's.
fn video(url: &Url, (dict, dash): Media, id: &str, (name, key): (&str, &str)) -> Result<VideoInfo> {
    let mut formats = Vec::new();
    for format in dict
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
    {
        let keys: Vec<String> = format.keys().cloned().collect();
        let mut format = format.clone();
        let ext = format
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext)
            .unwrap_or_else(|| "unknown_video".into());
        format.insert("ext".into(), ext.into());
        if !format.contains_key("format_id") {
            format.insert("format_id".into(), "".into());
        }
        let mut format: VideoFormat = serde_json::from_value(Value::Object(format))
            .map_err(|error| fail_as(name, id, &error.to_string()))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        formats.push(format);
    }
    formats.extend(dash);
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
    Ok(crate::info_order::mark_info(
        info,
        &format!("dict;{};thumbnail=url,width,height", keys.join(",")),
    ))
}

/// `{'_type': 'playlist', **info_dict, 'title': 'Post by ...', 'entries':
/// [...]}` of a carousel.
fn carousel_playlist(
    url: &Url,
    info: Map<String, Value>,
    entries: Vec<VideoInfo>,
    name: (&str, &str),
) -> ExtractResult {
    let mut extra = info.clone();
    let id = extra
        .shift_remove("id")
        .and_then(|id| id.as_str().map(str::to_owned))
        .unwrap_or_default();
    extra.shift_remove("title");
    let title = info
        .get("channel")
        .and_then(Value::as_str)
        .map(|channel| format!("Post by {channel}"));
    let mut keys: Vec<String> = vec!["_type".into()];
    keys.extend(info.keys().cloned());
    if title.is_none() {
        extra.insert("title".into(), Value::Null);
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: name.0.into(),
        extractor_key: name.1.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

/// Videos whose comments `post_extract` fetches (upstream's
/// `__post_extractor`): the video ID and the extractor's name.
static PENDING_COMMENTS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// `'__post_extractor': self.extract_comments(info_dict.get('id'))`.
fn defer_comments(id: &str, name: &str) {
    if super::get_comments()
        && let Ok(mut pending) = PENDING_COMMENTS.lock()
    {
        pending.push((id.to_owned(), name.to_owned()));
    }
}

/// `YoutubeDL.post_extract` of a video whose extraction deferred its
/// comments: `_get_comments(video_id)` (the comments API, non-fatal) as the
/// video's `comments` and `comment_count`.
pub(crate) fn post_extract(client: &Client, info: &mut VideoInfo) -> Result<()> {
    let pending = PENDING_COMMENTS.lock().ok().and_then(|mut pending| {
        let index = pending.iter().position(|(id, _)| *id == info.id)?;
        Some(pending.remove(index))
    });
    let Some((video_id, name)) = pending else {
        return Ok(());
    };
    let url = Url::parse(&format!(
        "{}/media/{}/comments/?can_support_threading=true&permalink_enabled=false",
        api_base(),
        id_to_pk(&video_id)
    ))?;
    let data = api_json_optional(
        client,
        &url,
        &name,
        &video_id,
        "Downloading comments info",
        Some("Comments extraction failed"),
    )
    .unwrap_or_else(|| json!({}));
    // `traverse_obj(comments_info, ('edge_media_to_parent_comment',
    // 'edges'), 'comments')`.
    let items = data
        .pointer("/edge_media_to_parent_comment/edges")
        .filter(|value| !value.is_null())
        .or_else(|| data.get("comments").filter(|value| !value.is_null()))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let first = |comment: &Value, paths: &[&str], map: &dyn Fn(&Value) -> Option<Value>| {
        paths
            .iter()
            .filter_map(|path| comment.pointer(path))
            .find_map(map)
            .unwrap_or(Value::Null)
    };
    let any = |value: &Value| (!value.is_null()).then(|| value.clone());
    let string = |value: &Value| str_or_none(Some(value)).map(Value::from);
    let integer = |value: &Value| int(Some(value)).map(Value::from);
    let link = |value: &Value| url_value(Some(value)).map(Value::from);
    let comments: Vec<Value> = items
        .iter()
        .map(|comment| {
            let mut entry = Map::new();
            entry.insert(
                "author".into(),
                first(comment, &["/node/owner/username", "/user/username"], &any),
            );
            entry.insert(
                "author_id".into(),
                first(comment, &["/node/owner/id", "/user/pk"], &string),
            );
            entry.insert(
                "author_thumbnail".into(),
                first(
                    comment,
                    &["/node/owner/profile_pic_url", "/user/profile_pic_url"],
                    &link,
                ),
            );
            entry.insert("id".into(), first(comment, &["/node/id", "/pk"], &string));
            entry.insert(
                "text".into(),
                first(comment, &["/node/text", "/text"], &any),
            );
            entry.insert(
                "like_count".into(),
                first(
                    comment,
                    &["/node/edge_liked_by/count", "/comment_like_count"],
                    &integer,
                ),
            );
            entry.insert(
                "timestamp".into(),
                first(comment, &["/node/created_at", "/created_at"], &integer),
            );
            Value::Object(entry)
        })
        .collect();
    crate::screen::screen(&format!("[{name}] Extracted {} comments", comments.len()));
    let count = comments.len();
    crate::metadata_parser::set_field(info, "comments", Value::Array(comments));
    crate::metadata_parser::set_field(info, "comment_count", count.into());
    Ok(())
}

/// `_extract_product(product_info)` with `get_comments=True` (the logged-in
/// API and stories): a carousel's entries have `comments: None`.
fn logged_in_result(
    url: &Url,
    product: &Value,
    video_id: Option<&str>,
    names: (&str, &str),
) -> Result<ExtractResult> {
    match extract_product(product, video_id)? {
        Product::Single(media) => {
            let id = media
                .0
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            defer_comments(&id, names.0);
            Ok(ExtractResult::Video(video(url, media, &id, names)?))
        }
        Product::Carousel(info, items) => {
            if super::get_comments() {
                // Upstream shares one comments generator between the entries.
                return Err(Error::UnsupportedHtml5(
                    "comments of logged-in Instagram carousel posts are not supported natively"
                        .into(),
                ));
            }
            let id = info
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let mut entries = Vec::new();
            for (mut dict, dash) in items {
                dict.insert("comments".into(), Value::Null);
                entries.push(video(url, (dict, dash), &id, names)?);
            }
            Ok(carousel_playlist(url, info, entries, names))
        }
    }
}

/// The media info API (logged in): `None` when the request was redirected
/// to the login page.
fn media_info(client: &Client, video_id: &str, media_id: &str) -> Result<Option<Value>> {
    let url = Url::parse(&format!("{}/media/{media_id}/info/", api_base()))?;
    crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading video info"));
    let headers = api_headers();
    let response =
        match super::direct::request_with_headers(client, &url, None, &header_refs(&headers)) {
            Ok(response) => response,
            Err(error) => {
                if error_url(&error).is_some_and(login_redirect) {
                    return Ok(None);
                }
                return Err(request_failure(
                    NAME,
                    video_id,
                    "Video info extraction failed",
                    &error,
                ));
            }
        };
    let body = read_body(response, video_id)?;
    let data: Value = serde_json::from_slice(&body).map_err(|error| {
        fail(
            video_id,
            &format!("Failed to parse JSON  (caused by {error})"),
        )
    })?;
    let key_error = |key: &str| {
        fail(
            video_id,
            &format!(
                "An extractor error has occurred. (caused by KeyError({key})); {}",
                crate::BUG_REPORTS_MESSAGE
            ),
        )
    };
    let items = data.get("items").ok_or_else(|| key_error("'items'"))?;
    Ok(Some(items.get(0).cloned().ok_or_else(|| key_error("0"))?))
}

/// `traverse_obj(webpage, ({_SJS_RE.findall}, ..., {json.loads}, 'require',
/// ..., ..., ..., '__bbox', 'require', RelayPrefetchedStreamCache, ...,
/// ..., '__bbox', 'result', 'data', 'xig_polaris_media', {dict}, any))`.
fn prefetched_media(webpage: &str) -> Option<Value> {
    fn children(value: &Value) -> Vec<&Value> {
        match value {
            Value::Array(items) => items.iter().collect(),
            Value::Object(object) => object.values().collect(),
            _ => Vec::new(),
        }
    }
    for found in SJS.captures_iter(webpage) {
        let Ok(data) = serde_json::from_str::<Value>(&found[1]) else {
            continue;
        };
        let Some(require) = data.get("require") else {
            continue;
        };
        for a in children(require) {
            for b in children(a) {
                for c in children(b) {
                    let Some(inner) = c.pointer("/__bbox/require") else {
                        continue;
                    };
                    for entry in children(inner) {
                        if entry.get(0).and_then(Value::as_str)
                            != Some("RelayPrefetchedStreamCache")
                        {
                            continue;
                        }
                        for item in children(entry) {
                            for candidate in children(item) {
                                if let Some(media) = candidate
                                    .pointer("/__bbox/result/data/xig_polaris_media")
                                    .filter(|media| media.is_object())
                                {
                                    return Some(media.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for Instagram {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        // `_real_initialize` before `Extracting URL`.
        let logged_in = is_logged_in();
        if !logged_in {
            lsd_token(client)?;
        }
        announce(NAME, url);
        let media_id = id_to_pk(&video_id).to_string();
        let names = (NAME, NAME);
        if logged_in {
            if let Some(product) = media_info(client, &video_id, &media_id)? {
                return logged_in_result(url, &product, None, names).map(Some);
            }
            crate::screen::warning(&format!(
                "[{NAME}] The provided Instagram account cookies are no longer valid"
            ));
            for domain in COOKIE_DOMAINS {
                crate::cookies::clear_cookie(domain, "sessionid");
            }
            // Re-initialize to set the LSD token for logged-out extraction.
            lsd_token(client)?;
        }
        // `get_ruling_for_content` (non-fatal, without an error note).
        let mut ruling = Url::parse(&format!("{}/web/get_ruling_for_content/", api_base()))?;
        ruling
            .query_pairs_mut()
            .append_pair("content_type", "MEDIA")
            .append_pair("target_id", &media_id);
        let api_check = api_json_optional(
            client,
            &ruling,
            NAME,
            &video_id,
            "Checking post accessibility",
            None,
        )
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
        match crate::cookies::get_cookie("https://www.instagram.com", "csrftoken") {
            None => crate::screen::warning(&format!(
                "[{NAME}] {video_id}: No CSRF token set by Instagram API"
            )),
            Some(_) if api_check.get("status").and_then(Value::as_str) != Some("ok") => {
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Instagram API is not granting access"
                ));
            }
            Some(_) => {}
        }
        // The GraphQL request needs impersonation, which is unavailable.
        let error = ["title", "description"]
            .iter()
            .filter_map(|key| api_check.get(*key))
            .filter(|value| match value {
                Value::Null => false,
                Value::String(text) => !text.is_empty(),
                _ => true,
            })
            .map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(": ");
        if error.contains("Restricted Video") {
            return Err(login_required(&video_id, &error));
        }
        if !error.is_empty() {
            return Err(fail(&video_id, &error));
        }
        if video_id.len() > 28 {
            return Err(login_required(
                &video_id,
                "This content is only available for registered users who follow this account",
            ));
        }
        let page = Url::parse(&format!("https://www.instagram.com/p/{video_id}"))?;
        let (webpage, final_url) = super::download_webpage_handle(client, &page, NAME, &video_id)?;
        if login_redirect(&final_url) {
            return Err(login_required(
                &video_id,
                "The webpage request was redirected to the login page. You have exceeded the rate-limit for accessing posts anonymously",
            ));
        }
        let media = prefetched_media(&webpage);
        let Some(product) = media
            .as_ref()
            .and_then(|media| media.get("if_not_gated_logged_out"))
            .filter(|product| product.is_object())
        else {
            return Err(fail(
                &video_id,
                &format!(
                    "Instagram sent an empty media response. Check if this post is accessible in your browser without being logged-in. If it is not, then u{}. Otherwise, if the post is accessible in browser without being logged-in, {}",
                    &LOGIN_HINT[1..],
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        };
        let product = extract_product(product, Some(&video_id))?;
        if let Product::Single((dict, dash)) = &product {
            let no_video = dash.is_empty()
                && dict
                    .get("formats")
                    .and_then(Value::as_array)
                    .is_none_or(|formats| formats.is_empty());
            if no_video {
                crate::no_formats::raise_expected(
                    NAME,
                    None,
                    Some(&video_id),
                    "There is no video in this post",
                )?;
            }
        }
        let media = media.unwrap_or(Value::Null);
        let comments: Vec<Value> = media
            .pointer("/comments_connection/edges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|edge| {
                edge.pointer("/node/text")
                    .is_some_and(|text| !text.is_null())
            })
            .filter_map(|edge| edge.get("node"))
            .map(|node| {
                let mut comment = Map::new();
                if let Some(author) = node.pointer("/user/username").and_then(Value::as_str) {
                    comment.insert("author".into(), author.into());
                }
                if let Some(author_id) = str_or_none(node.pointer("/user/pk")) {
                    comment.insert("author_id".into(), author_id.into());
                }
                if let Some(id) = str_or_none(node.get("pk")) {
                    comment.insert("id".into(), id.into());
                }
                if let Some(text) = node.get("text").and_then(Value::as_str) {
                    comment.insert("text".into(), text.into());
                }
                if let Some(timestamp) = int(node.get("created_at")) {
                    comment.insert("timestamp".into(), timestamp.into());
                }
                if let Some(likes) = int(node.get("comment_like_count")) {
                    comment.insert("like_count".into(), likes.into());
                }
                Value::Object(comment)
            })
            .collect();
        match product {
            Product::Carousel(info, items) => {
                let mut entries = Vec::new();
                for (mut dict, dash) in items {
                    dict.insert("comments".into(), Value::Array(comments.clone()));
                    entries.push(video(url, (dict, dash), &video_id, names)?);
                }
                Ok(Some(carousel_playlist(url, info, entries, names)))
            }
            Product::Single((mut dict, dash)) => {
                // `__post_extractor` (None without comments) is popped in processing.
                dict.insert("comments".into(), Value::Array(comments));
                Ok(Some(ExtractResult::Video(video(
                    url,
                    (dict, dash),
                    &video_id,
                    names,
                )?)))
            }
        }
    }
}

impl Extractor for InstagramIos {
    fn key(&self) -> &'static str {
        IOS_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    /// `url_result(f'http://instagram.com/tv/{_pk_to_id(id)}', InstagramIE)`.
    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = IOS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        announce(IOS_NAME, url);
        let Some(video_id) = pk_to_id(&Value::from(&found["id"])) else {
            return Err(fail_as(
                IOS_NAME,
                "",
                &format!(
                    "An extractor error has occurred. (caused by ValueError(\"invalid literal for int() with base 10: ''\")); {}",
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        };
        let target = Url::parse(&format!("http://instagram.com/tv/{video_id}"))?;
        let mut info = super::direct::build_at(target.clone(), &target, String::new());
        info.formats.clear();
        info.direct = None;
        info.extra
            .insert(crate::FOLLOW_URL.into(), target.as_str().into());
        Ok(Some(ExtractResult::Video(info)))
    }
}

impl Extractor for InstagramStory {
    fn key(&self) -> &'static str {
        STORY_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = STORY_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let username = found["user"].to_owned();
        let story_id = found.name("id").map(|id| id.as_str().to_owned());
        announce(STORY_NAME, url);
        if username == "highlights" && story_id.is_none() {
            return Err(fail_as(
                STORY_NAME,
                "",
                "Input URL is missing a highlight ID",
            ));
        }
        let display_id = story_id.clone().unwrap_or_else(|| username.clone());
        let story_info = super::download_webpage(client, url, STORY_NAME, &display_id)?;
        let user_info = match super::search_json(&STORY_USER, &story_info, false) {
            Some(Some(value)) => value,
            Some(None) => {
                crate::screen::warning(&format!(
                    "[{STORY_NAME}] {display_id}: Unable to extract user info - Failed to parse JSON"
                ));
                Value::Null
            }
            None => {
                super::search_warning(STORY_NAME, "user info");
                Value::Null
            }
        };
        let empty = match &user_info {
            Value::Object(object) => object.is_empty(),
            Value::Array(items) => items.is_empty(),
            Value::String(text) => text.is_empty(),
            Value::Null => true,
            Value::Bool(flag) => !flag,
            Value::Number(number) => number.as_f64() == Some(0.0),
        };
        if empty {
            return Err(login_required_as(STORY_NAME, "This content is unreachable"));
        }
        // `traverse_obj(user_info, 'pk', 'id', expected_type=str_or_none)`.
        let user_id = ["pk", "id"]
            .iter()
            .find_map(|key| str_or_none(user_info.get(*key)));
        let highlight_key = story_id.as_ref().map(|id| format!("highlight:{id}"));
        let reel_ids = if username == "highlights" {
            highlight_key.clone().unwrap_or_default()
        } else {
            user_id.clone().ok_or_else(|| {
                fail_as(
                    STORY_NAME,
                    &display_id,
                    &format!("Unable to extract user id; {}", crate::BUG_REPORTS_MESSAGE),
                )
            })?
        };
        let api = Url::parse(&format!(
            "{}/feed/reels_media/?reel_ids={reel_ids}",
            api_base()
        ))?;
        let videos = api_json_optional(
            client,
            &api,
            STORY_NAME,
            &display_id,
            "Downloading JSON metadata",
            None,
        )
        .and_then(|data| data.get("reels").cloned())
        .filter(|reels| match reels {
            Value::Object(object) => !object.is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Null | Value::Bool(false) => false,
            Value::String(text) => !text.is_empty(),
            _ => true,
        });
        let Some(videos) = videos else {
            return Err(login_required_as(
                STORY_NAME,
                "You need to log in to access this content",
            ));
        };
        let reel = |key: Option<&str>| key.and_then(|key| videos.get(key));
        let highlight = reel(highlight_key.as_deref());
        let user_reel = reel(user_id.as_deref());
        let reel_user = user_reel
            .and_then(|reel| reel.get("user"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let full_name = highlight
            .and_then(|reel| reel.pointer("/user/full_name"))
            .filter(|value| !value.is_null())
            .or_else(|| {
                user_reel
                    .and_then(|reel| reel.pointer("/user/full_name"))
                    .filter(|value| !value.is_null())
            })
            .cloned()
            .unwrap_or(Value::Null);
        let story_title = highlight
            .and_then(|reel| reel.get("title"))
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .map_or_else(|| format!("Story by {username}"), str::to_owned);
        let items = highlight
            .and_then(|reel| reel.get("items"))
            .filter(|items| !items.is_null())
            .or_else(|| {
                user_reel
                    .and_then(|reel| reel.get("items"))
                    .filter(|items| !items.is_null())
            })
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let names = (STORY_NAME, STORY_KEY);
        let mut entries = Vec::new();
        for mut item in items {
            let Some(object) = item.as_object_mut() else {
                continue;
            };
            // `highlight.setdefault('user', {}).update(user_info)`.
            let user = object.entry("user").or_insert_with(|| json!({}));
            if let Some(user) = user.as_object_mut() {
                for (key, value) in &reel_user {
                    user.insert(key.clone(), value.clone());
                }
            }
            let Product::Single((data, dash)) = extract_product(&item, None)? else {
                // A carousel result has no `formats`.
                continue;
            };
            let has_formats = !dash.is_empty()
                || data
                    .get("formats")
                    .and_then(Value::as_array)
                    .is_some_and(|formats| !formats.is_empty());
            if !has_formats {
                continue;
            }
            // `{'uploader': full_name, 'uploader_id': user_id,
            // **filter_dict(highlight_data)}`.
            let mut dict = Map::new();
            dict.insert("uploader".into(), full_name.clone());
            dict.insert(
                "uploader_id".into(),
                user_id.clone().map_or(Value::Null, Value::from),
            );
            for (key, value) in data {
                if !value.is_null() {
                    dict.insert(key, value);
                }
            }
            let id = dict
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            defer_comments(&id, STORY_NAME);
            entries.push(video(url, (dict, dash), &id, names)?);
        }
        if username != "highlights"
            && let Some(story_id) = &story_id
        {
            // `_yes_playlist(username, story_id)`.
            if super::no_playlist() {
                crate::screen::screen(&format!(
                    "[{STORY_NAME}] Downloading just the video {story_id} because of --no-playlist"
                ));
                let wanted = pk_to_id(&Value::from(story_id.as_str()));
                return match entries
                    .into_iter()
                    .find(|entry| Some(&entry.id) == wanted.as_ref())
                {
                    Some(entry) => Ok(Some(ExtractResult::Video(entry))),
                    None => Err(Error::UnsupportedHtml5(format!(
                        "Instagram story item {story_id} is not in its reel"
                    ))),
                };
            }
            crate::screen::screen(&format!(
                "[{STORY_NAME}] Downloading playlist {username} - add --no-playlist to download just the video {story_id}"
            ));
        }
        // `playlist_result(info_data, playlist_id=story_id,
        // playlist_title=story_title)`.
        let mut extra = Map::new();
        let keys = if story_id.is_some() {
            "id,title"
        } else {
            "title"
        };
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        if story_id.is_none() {
            extra.insert("id".into(), Value::Null);
        }
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: story_id.unwrap_or_default(),
            title: story_title,
            webpage_url: url.clone(),
            extractor: STORY_NAME.into(),
            extractor_key: STORY_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// `InstagramPlaylistBaseIE`'s user or tag timeline.
struct Timeline {
    name: &'static str,
    key: &'static str,
    /// `_QUERY_HASH`.
    query_hash: &'static str,
    /// `_query_vars_for(data)`: the variable's name and its path.
    variable: (&'static str, &'static str),
    /// `_parse_timeline_from(data)`.
    timeline: &'static str,
}

const USER_TIMELINE: Timeline = Timeline {
    name: USER_NAME,
    key: USER_KEY,
    query_hash: "42323d64886122307be10013ad2dcc44",
    variable: ("id", "/entry_data/ProfilePage/0/graphql/user/id"),
    timeline: "/data/user/edge_owner_to_timeline_media",
};

const TAG_TIMELINE: Timeline = Timeline {
    name: TAG_NAME,
    key: TAG_KEY,
    query_hash: "f92f56d47dc7a55b606908374b43a314",
    variable: ("tag_name", "/entry_data/TagPage/0/graphql/hashtag/name"),
    timeline: "/data/hashtag/edge_hashtag_to_media",
};

/// `_get_count(media, kind, *keys)`.
fn get_count(node: &Value, kind: &str, keys: &[&str]) -> Value {
    std::iter::once(format!("/{kind}/count"))
        .chain(keys.iter().map(|key| format!("/edge_media_{key}/count")))
        .find_map(|path| int(node.pointer(&path)))
        .map_or(Value::Null, Value::from)
}

/// `_extract_nodes(nodes)`: `url_result` entries of the video nodes.
fn node_entries(nodes: &[Value]) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    for node in nodes {
        if node.get("__typename").and_then(Value::as_str) != Some("GraphVideo")
            && node.get("is_video") != Some(&Value::Bool(true))
        {
            continue;
        }
        let Some(shortcode) = node
            .get("shortcode")
            .filter(|code| match code {
                Value::String(text) => !text.is_empty(),
                Value::Null => false,
                _ => true,
            })
            .and_then(|code| str_or_none(Some(code)))
        else {
            continue;
        };
        let target = format!("https://instagram.com/p/{shortcode}");
        let title = node
            .get("title")
            .filter(|title| match title {
                Value::String(text) => !text.is_empty(),
                Value::Null | Value::Bool(false) => false,
                _ => true,
            })
            .cloned()
            .unwrap_or(Value::Null);
        let description = node
            .pointer("/edge_media_to_caption/edges/0/node/text")
            .filter(|text| text.is_string())
            .cloned()
            .unwrap_or(Value::Null);
        let thumbnail = ["display_url", "thumbnail_src", "display_src"]
            .iter()
            .find_map(|key| url_value(node.get(*key)))
            .map_or(Value::Null, Value::from);
        let duration = node
            .get("video_duration")
            .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
            .map_or(Value::Null, |duration| json!(duration));
        let mut entry = Map::new();
        entry.insert("_type".into(), "url".into());
        entry.insert("ie_key".into(), NAME.into());
        entry.insert("id".into(), shortcode.clone().into());
        entry.insert("url".into(), target.clone().into());
        entry.insert("title".into(), title);
        entry.insert("description".into(), description);
        entry.insert("thumbnail".into(), thumbnail);
        entry.insert("duration".into(), duration);
        entry.insert(
            "timestamp".into(),
            int(node.get("taken_at_timestamp")).map_or(Value::Null, Value::from),
        );
        entry.insert(
            "view_count".into(),
            int(node.get("video_view_count")).map_or(Value::Null, Value::from),
        );
        entry.insert(
            "comment_count".into(),
            get_count(
                node,
                "comments",
                &["preview_comment", "to_comment", "to_parent_comment"],
            ),
        );
        entry.insert(
            "like_count".into(),
            get_count(node, "likes", &["preview_like"]),
        );
        let url = Url::parse(&target)?;
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
                "extractor": NAME.to_lowercase(),
                "entry": Value::Object(entry),
            }),
        );
        entries.push(info);
    }
    Ok(entries)
}

/// `InstagramPlaylistBaseIE._real_extract`.
fn timeline_playlist(
    client: &Client,
    url: &Url,
    timeline: &Timeline,
) -> Result<Option<ExtractResult>> {
    let regex = if timeline.key == USER_KEY {
        &USER_URL
    } else {
        &TAG_URL
    };
    let Some(found) = regex.captures(url.as_str()) else {
        return Ok(None);
    };
    let item_id = found["id"].to_owned();
    let name = timeline.name;
    announce(name, url);
    let webpage = super::download_webpage(client, url, name, &item_id)?;
    // `_parse_graphql`.
    let data: Value = match SHARED_DATA.captures(&webpage) {
        Some(found) => serde_json::from_str(&found[1]).map_err(|error| {
            fail_as(
                name,
                &item_id,
                &format!("Failed to parse JSON  (caused by {error})"),
            )
        })?,
        None => {
            return Err(fail_as(
                name,
                "",
                &format!("Unable to extract data; {}", crate::BUG_REPORTS_MESSAGE),
            ));
        }
    };
    crate::cookies::set_cookie("instagram.com", "ig_pr", "1");
    let key_error = |key: &str| {
        fail_as(
            name,
            &item_id,
            &format!(
                "An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ),
        )
    };
    // `_extract_graphql(data, url)`.
    let csrf_token = data
        .pointer("/config/csrf_token")
        .ok_or_else(|| key_error("config"))?;
    let csrf_token = str_or_none(Some(csrf_token)).unwrap_or_else(|| "None".into());
    let rhx_gis = data
        .get("rhx_gis")
        .and_then(Value::as_str)
        .filter(|gis| !gis.is_empty())
        .unwrap_or("3c7ca9dcefcf966d11dacf1f151335e8")
        .to_owned();
    let variable = data
        .pointer(timeline.variable.1)
        .cloned()
        .ok_or_else(|| key_error("entry_data"))?;
    let user_agent = super::user_agent();
    let mut cached: Option<String> = None;
    let mut cursor = String::new();
    let mut entries = Vec::new();
    for page in 1.. {
        // `json.dumps({'first': 12, 'after': cursor, **_query_vars_for(data)})`.
        let variables = format!(
            "{{\"first\": 12, \"after\": {}, \"{}\": {}}}",
            crate::output::traverse::py_json(&Value::from(cursor.as_str()), true),
            timeline.variable.0,
            crate::output::traverse::py_json(&variable, true)
        );
        let templates = match cached.as_ref().filter(|template| !template.is_empty()) {
            Some(template) => vec![template.clone()],
            None => vec![
                rhx_gis.clone(),
                String::new(),
                format!("{rhx_gis}:{csrf_token}"),
                format!("{rhx_gis}:{csrf_token}:{user_agent}"),
            ],
        };
        let mut query = Url::parse("https://www.instagram.com/graphql/query/")?;
        query
            .query_pairs_mut()
            .append_pair("query_hash", timeline.query_hash)
            .append_pair("variables", &variables);
        let mut media = None;
        for (index, template) in templates.iter().enumerate() {
            let signature = crate::md5::hex(format!("{template}:{variables}").as_bytes());
            let result = super::download_json_note_headers(
                client,
                &query,
                name,
                &item_id,
                &format!("Downloading JSON page {page}"),
                &[
                    ("X-Requested-With", "XMLHttpRequest"),
                    ("X-Instagram-GIS", &signature),
                ],
            );
            match result {
                Ok(json_data) => {
                    let timeline_data = json_data
                        .pointer(timeline.timeline)
                        .cloned()
                        .ok_or_else(|| key_error("data"))?;
                    cached = Some(template.clone());
                    media = Some(timeline_data);
                    break;
                }
                // A bad query: the next way to sign it.
                Err(error) if error_status(&error) == Some(403) && index + 1 < templates.len() => {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        let media = media.unwrap_or(Value::Null);
        let nodes: Vec<Value> = media
            .get("edges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|edge| edge.get("node"))
            .filter(|node| node.is_object())
            .cloned()
            .collect();
        if nodes.is_empty() {
            break;
        }
        entries.extend(node_entries(&nodes)?);
        let has_next_page = media
            .pointer("/page_info/has_next_page")
            .is_some_and(|value| match value {
                Value::Bool(flag) => *flag,
                Value::Null => false,
                Value::Number(number) => number.as_f64() != Some(0.0),
                Value::String(text) => !text.is_empty(),
                _ => true,
            });
        cursor = media
            .pointer("/page_info/end_cursor")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !has_next_page || cursor.is_empty() {
            break;
        }
    }
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;id,title{}", crate::info_order::GENERATOR).into(),
    );
    Ok(Some(ExtractResult::Playlist(PlaylistInfo {
        id: item_id.clone(),
        title: item_id,
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: timeline.key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })))
}

impl Extractor for InstagramTag {
    fn key(&self) -> &'static str {
        TAG_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        timeline_playlist(client, url, &TAG_TIMELINE)
    }
}

impl Extractor for InstagramUser {
    fn key(&self) -> &'static str {
        USER_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !user_suitable(url.as_str()) {
            return Ok(None);
        }
        // Upstream `_WORKING = False`.
        super::broken_warning();
        timeline_playlist(client, url, &USER_TIMELINE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://instagram.com/p/aye83DjauH/?foo=bar#abc"));
        assert!(suitable("https://www.instagram.com/reel/Chunk8-jurw/"));
        assert!(suitable(
            "https://www.instagram.com/marvelskies.fc/reel/CWqAgUZgCku/"
        ));
        assert!(!suitable("https://www.instagram.com/share/reel/ABC/"));
        assert!(!suitable("https://www.instagram.com/reels/audio/123/"));
        assert!(!suitable("https://www.instagram.com/someuser/"));
        assert!(ios_suitable("instagram://media?id=482584233761418119"));
        assert!(story_suitable(
            "https://www.instagram.com/stories/highlights/18090946048123978/"
        ));
        assert!(story_suitable(
            "https://www.instagram.com/stories/fruits_zipper"
        ));
        assert!(tag_suitable("https://instagram.com/explore/tags/lolcats"));
        assert!(user_suitable("https://instagram.com/porsche"));
        assert!(user_suitable("https://www.instagram.com/someuser/"));
        assert!(!user_suitable("https://instagram.com/explore/tags/lolcats"));
        assert!(!user_suitable("https://instagram.com/p/aye83DjauH/"));
    }

    #[test]
    fn shortcodes_and_pks_convert_like_upstream() {
        assert_eq!(id_to_pk("aye83DjauH"), 482584233761418119);
        assert_eq!(
            pk_to_id(&json!(482584233761418119_u64)).as_deref(),
            Some("aye83DjauH")
        );
        assert_eq!(
            pk_to_id(&json!("482584233761418119_1")).as_deref(),
            Some("aye83DjauH")
        );
    }

    #[test]
    fn inline_dash_needs_absolute_direct_representations() {
        let manifest = |base: &str, address: &str| {
            format!(
                "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT2S\"><Period><AdaptationSet><Representation id=\"1v\" mimeType=\"video/mp4\" codecs=\"avc1.4D401E\" width=\"2\" height=\"2\" bandwidth=\"1000\"><BaseURL>{base}</BaseURL>{address}</Representation></AdaptationSet></Period></MPD>"
            )
        };
        let formats = dash_formats(
            &manifest(
                "https://cdn.example/v.mp4",
                "<SegmentBase indexRange=\"1-2\"><Initialization range=\"0-1\"/></SegmentBase>",
            ),
            "x",
        )
        .unwrap();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].format_id, "dash-1v");
        assert_eq!(formats[0].url.as_str(), "https://cdn.example/v.mp4");
        assert!(formats[0].fragments.is_empty() && formats[0].protocol.is_none());
        assert!(dash_formats(&manifest("v.mp4", ""), "x").is_err());
    }
}
