//! Upstream `TikTokIE`, `TikTokVMIE`, `DouyinIE`, `TikTokUserIE` and
//! `TikTokCollectionIE` (`yt_dlp/extractor/tiktok.py`): a video's webpage
//! universal data (solving the page's JS challenge like
//! `_solve_challenge_and_set_cookies`), `_parse_aweme_video_web`'s formats,
//! metadata and webpage subtitles (slideshows are their audio track),
//! short-link redirects, Douyin's web detail API through
//! `_parse_aweme_video_app`, and user/collection lists from the web item list
//! APIs as flat `url` entries.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TikTok;
pub struct TikTokVm;
pub struct Douyin;
pub struct TikTokUser;
pub struct TikTokCollection;
pub struct TikTokLive;

const NAME: &str = "TikTok";
const VM_NAME: &str = "vm.tiktok";
const DOUYIN_NAME: &str = "Douyin";
const USER_NAME: &str = "tiktok:user";
const USER_KEY: &str = "TikTokUser";
const COLLECTION_NAME: &str = "tiktok:collection";
const COLLECTION_KEY: &str = "TikTokCollection";
const LIVE_NAME: &str = "tiktok:live";
const LIVE_KEY: &str = "TikTokLive";
const QUALITIES: [&str; 4] = ["360p", "540p", "720p", "1080p"];
/// `_UPLOADER_URL_FORMAT` (`%s` appended).
const TIKTOK_UPLOADER: &str = "https://www.tiktok.com/@";
const DOUYIN_UPLOADER: &str = "https://www.douyin.com/user/";
/// `DouyinIE._WEBPAGE_HOST`.
const DOUYIN_HOST: &str = "https://www.douyin.com/";
const LOGIN_HINT: &str = "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://www\.tiktokv?\.com/(?:embed|(?:share|@(?P<user_id>[\w\.-]+)?)/video)/(?P<id>\d+)",
    )
    .unwrap()
});
static VM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:vm|vt)\.tiktok\.com|(?:www\.)tiktok\.com/t)/(?P<id>\w+)").unwrap()
});
static DOUYIN_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?douyin\.com/video/(?P<id>[0-9]+)").unwrap());
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:tiktokuser:|https?://(?:www\.)?tiktok\.com/@)(?P<id>[\w.-]+)/?(?:$|[#?])")
        .unwrap()
});
static COLLECTION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://www\.tiktok\.com/@(?P<user_id>[\w.-]+)/collection/(?P<title>[^/?#]+)-(?P<id>\d+)/?(?:[?#]|$)",
    )
    .unwrap()
});
static LIVE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www\.)?tiktok\.com/@(?P<uploader>[\w.-]+)/live|m\.tiktok\.com/share/live/(?P<id>\d+))",
    )
    .unwrap()
});
static SEC_UID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^MS4wLjABAAAA[\w-]{64}$").unwrap());
static UNIVERSAL_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<script[^>]+\bid="__UNIVERSAL_DATA_FOR_REHYDRATION__"[^>]*>"#).unwrap()
});
static URL_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"v[^_]+_(?P<id>(?P<codec>[^_]+)_(?P<res>\d+p)_(?P<bitrate>\d+))").unwrap()
});
static ARTISTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:, | & )").unwrap());

/// `TikTokIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `TikTokVMIE._match_valid_url`.
pub fn vm_suitable(url: &str) -> bool {
    VM_URL.is_match(url)
}

/// `DouyinIE._match_valid_url`.
pub fn douyin_suitable(url: &str) -> bool {
    DOUYIN_URL.is_match(url)
}

/// `TikTokUserIE._match_valid_url` (also the `tiktokuser:SEC_UID` pseudo-URL).
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

/// `TikTokLiveIE._match_valid_url`.
pub fn live_suitable(url: &str) -> bool {
    LIVE_URL.is_match(url)
}

/// `TikTokCollectionIE._match_valid_url`.
pub fn collection_suitable(url: &str) -> bool {
    COLLECTION_URL.is_match(url)
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.into()),
        _ => None,
    }
}

fn fail(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{name}] {id}: {message}"))
}

fn login_required(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{name}] {id}: {message}. {LOGIN_HINT}"))
}

/// An extractor error that is not `expected` (a bug report hint follows).
fn unexpected(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{name}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// A Python exception raised inside the extractor (`KeyError`, ...).
fn bug(name: &str, id: &str, cause: &str) -> Error {
    Error::Reported(format!(
        "[{name}] {id}: An extractor error has occurred. (caused by {cause}); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// An error's message for `report_warning` when the request is not fatal.
fn error_text(error: &Error) -> String {
    match error {
        Error::Reported(text) => text.clone(),
        error => crate::upstream_error_text(error),
    }
}

/// `_proto_relative_url(url)`.
fn proto_relative(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    }
}

/// A random number (upstream uses `random`; values are never reproduced).
fn random_u64(salt: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(salt);
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos()),
    );
    hasher.finish()
}

/// SHA-256 (FIPS 180-4), for the webpage's proof-of-work challenge.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 64];
        for (index, word) in block.chunks(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut digest = [0u8; 32];
    for (chunk, word) in digest.chunks_mut(4).zip(state) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// `find_element(id=ID, html=True)`'s `class` attribute.
fn element_class(webpage: &str, id: &str) -> Option<String> {
    let element = crate::html_elements::element_html_by_id(id, webpage).ok()??;
    crate::html_elements::extract_attributes(&element)
        .get("class")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// `_solve_challenge_and_set_cookies`: the proof-of-work number whose
/// SHA-256 (after the base) is the expected digest, as the `wci` cookie.
fn solve_challenge(webpage: &str, name: &str, id: &str) -> Result<Vec<String>> {
    let engine = base64::engine::general_purpose::STANDARD;
    let challenge: Option<Map<String, Value>> = element_class(webpage, "cs")
        .filter(|class| !class.is_empty())
        .and_then(|class| {
            // `f'{x}==='`: Python's decoder ignores the excess padding.
            let trimmed = class.trim_end_matches('=');
            let padded = format!("{trimmed}{}", "=".repeat((4 - trimmed.len() % 4) % 4));
            engine.decode(padded).ok()
        })
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let Some(mut challenge) = challenge else {
        if webpage.contains("Please wait...") {
            return Err(fail(name, id, "Unable to extract challenge data"));
        }
        return Err(fail(name, id, "Unexpected response from webpage request"));
    };
    crate::screen::screen(&format!(
        "[{name}] Solving JS challenge using native Python implementation"
    ));
    let field = |key: &str| {
        challenge
            .get("v")
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .and_then(|text| engine.decode(text).ok())
    };
    let expected =
        field("c").ok_or_else(|| fail(name, id, "Unable to extract challenge expected digest"))?;
    let base = field("a").ok_or_else(|| fail(name, id, "Unable to extract challenge base hash"))?;
    let mut solution = None;
    for number in 0..=1_000_000u32 {
        let mut data = base.clone();
        data.extend_from_slice(number.to_string().as_bytes());
        if sha256(&data)[..] == expected[..] {
            solution = Some(number);
            break;
        }
    }
    let number = solution.ok_or_else(|| fail(name, id, "Unable to solve JS challenge"))?;
    challenge.insert("d".into(), engine.encode(number.to_string()).into());
    let value = engine.encode(serde_json::to_string(&Value::Object(challenge)).unwrap_or_default());
    let wci = element_class(webpage, "wci")
        .ok_or_else(|| fail(name, id, "Unable to extract challenge cookie name"))?;
    crate::cookies::set_cookie(".tiktok.com", &wci, &value);
    let mut names = vec![wci];
    if let (Some(rci), Some(rs)) = (element_class(webpage, "rci"), element_class(webpage, "rs")) {
        crate::cookies::set_cookie(".tiktok.com", &rci, &rs);
        names.push(rci);
    }
    Ok(names)
}

/// `_get_universal_data`: `__DEFAULT_SCOPE__` of the rehydration data.
fn universal_data(webpage: &str) -> Option<Value> {
    match super::search_json(&UNIVERSAL_DATA, webpage, false) {
        Some(Some(data)) => data
            .get("__DEFAULT_SCOPE__")
            .filter(|scope| scope.is_object() && truthy(Some(scope)))
            .cloned(),
        _ => None,
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
    }
}

/// A page request: `what` is the error note's object (`Unable to download
/// WHAT`); a non-fatal request without one fails silently (`errnote=False`).
struct PageRequest<'a> {
    name: &'static str,
    id: &'a str,
    note: &'a str,
    what: Option<&'static str>,
    fatal: bool,
    impersonate: bool,
}

/// `_download_webpage_handle(url, id, note, impersonate=...)`: the page and
/// its final URL; `None` when a non-fatal request failed.
fn download_page(
    client: &Client,
    url: &Url,
    request: PageRequest<'_>,
) -> Result<Option<(String, Url)>> {
    Ok(download_page_target(client, url, request)?.map(|(page, url, _)| (page, url)))
}

/// [`download_page`] with the response's impersonate target
/// (`urlh.extensions.get("impersonate")`).
fn download_page_target(
    client: &Client,
    url: &Url,
    request: PageRequest<'_>,
) -> Result<Option<(String, Url, Option<String>)>> {
    use std::io::Read;
    let PageRequest {
        name,
        id,
        note,
        what,
        fatal,
        impersonate,
    } = request;
    let scope = impersonate.then(|| {
        crate::impersonate::attempt(crate::impersonate::ANY, |message| {
            crate::screen::warning_once(&format!("[{name}] {message}"));
        })
    });
    crate::screen::screen(&format!("[{name}] {id}: {note}"));
    let mut response = match super::direct::request(client, url, None) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            if !fatal {
                if let Some(what) = what {
                    crate::screen::warning(&format!(
                        "[{name}] {id}: Unable to download {what}: {}",
                        crate::upstream_error_text(&error)
                    ));
                }
                return Ok(None);
            }
            return Err(Error::ExtractorRequest {
                extractor: name,
                id: id.to_owned(),
                what: what.unwrap_or("webpage"),
                source: Box::new(error),
            });
        }
        Err(error) => return Err(error),
    };
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let target = scope.and_then(|scope| scope.target().map(ToString::to_string));
    Ok(Some((
        String::from_utf8_lossy(&body).into_owned(),
        page_url,
        target,
    )))
}

/// `get_webpage(note)` of `_extract_web_data_and_status`.
fn webpage(
    client: &Client,
    url: &Url,
    name: &'static str,
    id: &str,
    note: &str,
    fatal: bool,
) -> Result<Option<String>> {
    let request = PageRequest {
        name,
        id,
        note,
        what: Some("webpage"),
        fatal,
        impersonate: true,
    };
    let Some((page, page_url, target)) = download_page_target(client, url, request)? else {
        return Ok(None);
    };
    crate::screen::debug(&format!("Webpage size: {} bytes", page.len()));
    crate::screen::debug(&format!(
        "Impersonation target: {}",
        target.as_deref().unwrap_or("None")
    ));
    if page_url.path() == "/login" {
        let message = "TikTok is requiring login for access to this content";
        if fatal {
            return Err(login_required(name, id, message));
        }
        crate::screen::warning(&format!("[{name}] {id}: {message}. {LOGIN_HINT}"));
        return Ok(None);
    }
    Ok(Some(page))
}

/// `_extract_web_data_and_status(url, video_id, fatal)`: the video's item
/// struct (`{}` when unavailable) and the page's status code.
fn web_data(
    client: &Client,
    url: &Url,
    name: &'static str,
    id: &str,
    fatal: bool,
) -> Result<(Value, i64)> {
    let unavailable = || (json!({}), -1);
    let Some(page) = webpage(client, url, name, id, "Downloading webpage", fatal)? else {
        return Ok(unavailable());
    };
    let mut data = universal_data(&page);
    if data.is_none() {
        let names = match solve_challenge(&page, name, id) {
            Ok(names) => names,
            Err(error) if !fatal => {
                crate::screen::warning(&error_text(&error));
                return Ok(unavailable());
            }
            Err(error) => return Err(error),
        };
        let page = webpage(
            client,
            url,
            name,
            id,
            "Downloading webpage with challenge cookie",
            fatal,
        );
        for name in names {
            crate::cookies::clear_cookie(".tiktok.com", &name);
        }
        let Some(page) = page? else {
            return Ok(unavailable());
        };
        data = universal_data(&page);
    }
    let Some(data) = data else {
        let message = "Unable to extract universal data for rehydration";
        if fatal {
            return Err(fail(name, id, message));
        }
        crate::screen::warning(&format!("[{name}] {id}: {message}"));
        return Ok(unavailable());
    };
    let detail = data.get("webapp.video-detail");
    let status = detail
        .and_then(|detail| detail.get("statusCode"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let video = detail
        .and_then(|detail| detail.pointer("/itemInfo/itemStruct"))
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(Value::Null);
    if !video.get("video").is_some_and(Value::is_object)
        && video
            .get("isContentClassified")
            .and_then(Value::as_bool)
            .is_some()
    {
        let message = "This post may not be comfortable for some audiences. Log in for access";
        if fatal {
            return Err(login_required(name, id, message));
        }
        crate::screen::warning(&format!("[{name}] {id}: {message}. {LOGIN_HINT}"));
    }
    Ok((video, status))
}

/// `_create_url(user_id, video_id)`.
fn create_url(user: &str, id: &str) -> String {
    let user = if user.is_empty() { "_" } else { user };
    format!("https://www.tiktok.com/@{user}/video/{id}")
}

/// `_get_subtitles(aweme_detail, aweme_id, user_name)` through
/// `extract_subtitles` (only with `--write-subs`/`--list-subs`).
fn subtitles(
    client: &Client,
    name: &'static str,
    detail: &Value,
    id: &str,
    user_name: Option<&str>,
) -> Result<crate::Subtitles> {
    if !super::subtitles_requested() {
        return Ok(crate::Subtitles::default());
    }
    let ext_map = |format: Option<&Value>| match format.and_then(Value::as_str) {
        Some("creator_caption") => Some("json"),
        Some("srt") => Some("srt"),
        Some("webvtt") => Some("vtt"),
        _ => None,
    };
    /// Per language (in first-seen order), the caption URLs and extensions.
    type Captions = Vec<(String, Vec<(String, Option<&'static str>)>)>;
    let mut found: Captions = Vec::new();
    let add =
        |found: &mut Captions, lang: String, url: String, ext: Option<&'static str>| match found
            .iter_mut()
            .find(|(existing, _)| *existing == lang)
        {
            Some((_, list)) => list.push((url, ext)),
            None => found.push((lang, vec![(url, ext)])),
        };
    let list = |value: Option<&Value>| -> Vec<Value> {
        value.and_then(Value::as_array).cloned().unwrap_or_default()
    };
    // The aweme/detail endpoint's automatic captions are downloaded and
    // converted into SRT `data`, which this port does not model.
    for sticker in list(detail.get("interaction_stickers")) {
        for caption in list(sticker.pointer("/auto_video_caption_info/auto_captions")) {
            let has_url = list(caption.pointer("/url/url_list"))
                .iter()
                .filter_map(Value::as_str)
                .any(|url| super::json_ld::url_or_none(url).is_some());
            if caption.is_object() && has_url {
                return Err(Error::Reported(format!(
                    "[{name}] {id}: automatic captions converted to SRT data are not supported natively"
                )));
            }
        }
    }
    // The feed endpoint's captions.
    for caption in list(detail.pointer("/video/cla_info/caption_infos")) {
        let Some(url) = caption
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let lang = caption
            .get("lang")
            .and_then(Value::as_str)
            .filter(|lang| !lang.is_empty())
            .unwrap_or("en");
        add(
            &mut found,
            lang.into(),
            url.into(),
            ext_map(caption.get("Format")),
        );
    }
    // The webpage's captions.
    if found.is_empty() {
        let fetched;
        let detail = match user_name.filter(|user| !user.is_empty()) {
            Some(user) => {
                let page = Url::parse(&create_url(user, id))?;
                fetched = web_data(client, &page, name, id, false)?.0;
                &fetched
            }
            None => detail,
        };
        for caption in list(detail.pointer("/video/subtitleInfos")) {
            let Some(url) = caption.get("Url").filter(|url| truthy(Some(url))) else {
                continue;
            };
            let Some(url) = url.as_str() else {
                continue;
            };
            let lang = caption
                .get("LanguageCodeName")
                .and_then(Value::as_str)
                .filter(|lang| !lang.is_empty())
                .unwrap_or("en");
            add(
                &mut found,
                lang.into(),
                url.into(),
                ext_map(caption.get("Format")),
            );
        }
    }
    // `qualities(('json', 'srt', 'vtt'))`: creator captions last but one.
    let preference = |ext: Option<&str>| -> i64 {
        ["json", "srt", "vtt"]
            .iter()
            .position(|known| Some(*known) == ext)
            .map_or(-1, |index| index as i64)
    };
    let mut subtitles = crate::Subtitles::default();
    for (lang, mut list) in found {
        list.sort_by_key(|(_, ext)| preference(*ext));
        let mut formats = Vec::new();
        for (url, ext) in list {
            let ext = ext.map(str::to_owned).unwrap_or_else(|| {
                super::direct::determine_ext(&url)
                    .unwrap_or_else(|| "unknown_video".into())
                    .to_lowercase()
            });
            let Ok(url) = Url::parse(&url) else {
                return Err(fail(name, id, &format!("Invalid subtitle URL: {url}")));
            };
            formats.push(crate::SubtitleFormat {
                name: None,
                url,
                ext,
                protocol: None,
                fragments: Vec::new(),
                extra: Map::new(),
            });
        }
        subtitles.entries.push((Some(lang), formats));
    }
    Ok(subtitles)
}

/// `_parse_url_key`: the bitrate rendition's ID, codec, bitrate, quality.
fn url_key(key: &str) -> (Map<String, Value>, Option<String>) {
    let Some(found) = URL_KEY.captures(key) else {
        return (Map::new(), None);
    };
    let mut info = Map::new();
    info.insert("format_id".into(), found["id"].into());
    let codec = &found["codec"];
    info.insert(
        "vcodec".into(),
        if codec == "bytevc1" { "h265" } else { codec }.into(),
    );
    let tbr = found["bitrate"]
        .parse::<i64>()
        .ok()
        .map(|rate| rate / 1000)
        .filter(|rate| *rate != 0);
    info.insert("tbr".into(), tbr.into());
    let res = found["res"].to_owned();
    info.insert(
        "quality".into(),
        QUALITIES
            .iter()
            .position(|quality| *quality == res)
            .map_or(-1, |index| index as i64)
            .into(),
    );
    (info, Some(res))
}

/// `_remove_duplicate_formats`: the first format of each URL.
fn remove_duplicate_formats(formats: &mut Vec<Map<String, Value>>) {
    let mut seen = Vec::new();
    formats.retain(|format| {
        let url = format.get("url").cloned();
        if seen.contains(&url) {
            return false;
        }
        seen.push(url);
        true
    });
}

/// `_extract_web_formats`.
fn web_formats(detail: &Value) -> Vec<Map<String, Value>> {
    let mut common = Map::new();
    common.insert("ext".into(), "mp4".into());
    common.insert("vcodec".into(), "h264".into());
    common.insert("acodec".into(), "aac".into());
    let video = detail
        .get("video")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let play_width = int(video.get("width"));
    let play_height = int(video.get("height"));
    let ratio = match (play_width, play_height) {
        (Some(width), Some(height)) if height != 0 && width != 0 => width as f64 / height as f64,
        _ => 0.5625,
    };
    let mut formats: Vec<Map<String, Value>> = Vec::new();
    for bitrate in video
        .get("bitrateInfo")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|bitrate| {
            bitrate
                .pointer("/PlayAddr/UrlList")
                .is_some_and(|list| truthy(Some(list)))
        })
    {
        let (mut info, res) = url_key(
            bitrate
                .pointer("/PlayAddr/UrlKey")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        let bytevc2 = info.get("vcodec").and_then(Value::as_str) == Some("bytevc2");
        info.insert(
            "format_note".into(),
            if bytevc2 {
                Value::from("UNPLAYABLE")
            } else {
                Value::Null
            },
        );
        info.insert("preference".into(), if bytevc2 { -100 } else { -1 }.into());
        info.insert(
            "filesize".into(),
            int(bitrate.pointer("/PlayAddr/DataSize")).into(),
        );
        if let Some(mut dimension) = res
            .as_deref()
            .and_then(|res| res.strip_suffix('p'))
            .and_then(|res| res.parse::<i64>().ok())
            .filter(|dimension| *dimension != 0)
        {
            if dimension == 540 {
                dimension = 576;
            }
            if ratio < 1.0 {
                let y = (dimension as f64 / ratio) as i64;
                info.insert("width".into(), dimension.into());
                info.insert("height".into(), (y - y % 2).into());
            } else {
                let x = (dimension as f64 * ratio) as i64;
                info.insert("width".into(), (x + x % 2).into());
                info.insert("height".into(), dimension.into());
            }
        }
        for url in bitrate
            .pointer("/PlayAddr/UrlList")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(super::json_ld::url_or_none)
        {
            let mut format = common.clone();
            format.extend(info.clone());
            format.insert("url".into(), proto_relative(&url).into());
            if Url::parse(&url).is_ok_and(|parsed| parsed.path().ends_with("/media-video-hvc1/")) {
                format.insert("acodec".into(), "none".into());
                format.insert("__needs_testing".into(), true.into());
            }
            formats.push(format);
        }
    }
    let play_quality = formats
        .iter()
        .find(|format| int(format.get("width")) == play_width && play_width.is_some())
        .and_then(|format| format.get("quality").cloned())
        .unwrap_or(Value::Null);
    let play_urls: Vec<String> = match video.get("playAddr") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("src").and_then(Value::as_str))
            .filter_map(super::json_ld::url_or_none)
            .collect(),
        Some(Value::String(url)) => super::json_ld::url_or_none(url).into_iter().collect(),
        _ => Vec::new(),
    };
    for url in play_urls {
        let mut format = common.clone();
        format.insert("format_id".into(), "play".into());
        format.insert("url".into(), proto_relative(&url).into());
        format.insert("width".into(), play_width.into());
        format.insert("height".into(), play_height.into());
        format.insert("quality".into(), play_quality.clone());
        formats.push(format);
    }
    for url in [video.get("downloadAddr"), video.pointer("/download/url")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(super::json_ld::url_or_none)
    {
        let mut format = common.clone();
        format.insert("format_id".into(), "download".into());
        format.insert("url".into(), proto_relative(&url).into());
        format.insert("format_note".into(), "watermarked".into());
        format.insert("preference".into(), (-2).into());
        format.insert("__needs_testing".into(), true.into());
        formats.push(format);
    }
    remove_duplicate_formats(&mut formats);
    if let Some(audio) = detail
        .pointer("/music/playUrl")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
    {
        let ext = Url::parse(&audio)
            .ok()
            .and_then(|parsed| {
                parsed
                    .query_pairs()
                    .filter(|(key, value)| key == "mime_type" && !value.is_empty())
                    .last()
                    .map(|(_, value)| value.replace('_', "/"))
            })
            .map(|mime| super::direct::mimetype2ext_or(&mime, None))
            .filter(|ext| !ext.is_empty())
            .unwrap_or_else(|| "m4a".into());
        let mut format = Map::new();
        format.insert("format_id".into(), "audio".into());
        format.insert("url".into(), proto_relative(&audio).into());
        format.insert("ext".into(), ext.clone().into());
        format.insert(
            "acodec".into(),
            if ext == "m4a" { "aac".to_owned() } else { ext }.into(),
        );
        format.insert("vcodec".into(), "none".into());
        formats.push(format);
    }
    formats.retain(|format| {
        format
            .get("url")
            .and_then(Value::as_str)
            .and_then(|url| Url::parse(url).ok())
            .is_none_or(|url| url.host_str() != Some("www.tiktok.com"))
    });
    formats
}

/// `format_field(info, KEYS, TEMPLATE, default=None)`: the first present
/// key's value, unless empty.
fn format_field(info: &Map<String, Value>, keys: &[&str], prefix: &str) -> Value {
    keys.iter()
        .find_map(|key| info.get(*key).filter(|value| !value.is_null()))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map_or(Value::Null, |value| format!("{prefix}{value}").into())
}

/// `truncate_string(desc, left=72)`.
fn truncate_title(desc: &str) -> String {
    let chars: Vec<char> = desc.chars().collect();
    if chars.len() <= 72 {
        desc.to_owned()
    } else {
        format!("{}...", chars[..69].iter().collect::<String>())
    }
}

/// `_parse_aweme_video_web`'s info dict (without its formats and subtitles,
/// which are `None` for a flat entry).
fn aweme_web(detail: &Value, webpage_url: &str, id: &str) -> Map<String, Value> {
    let mut info = Map::new();
    info.insert("id".into(), id.into());
    info.insert("formats".into(), Value::Null);
    info.insert("subtitles".into(), Value::Null);
    info.insert("http_headers".into(), json!({"Referer": webpage_url}));
    // `(('authorInfo', 'author', None), {...})`, the first non-empty match.
    let author = [detail.get("authorInfo"), detail.get("author"), Some(detail)]
        .into_iter()
        .flatten()
        .filter(|source| source.is_object())
        .map(|source| {
            let mut found = Map::new();
            let text = |keys: &[&str]| {
                keys.iter()
                    .find_map(|key| source.get(*key).and_then(Value::as_str))
                    .map(str::to_owned)
            };
            if let Some(value) = text(&["nickname"]) {
                found.insert("channel".into(), value.into());
            }
            if let Some(value) = text(&["authorSecId", "secUid"]) {
                found.insert("channel_id".into(), value.into());
            }
            if let Some(value) = text(&["uniqueId", "author"]) {
                found.insert("uploader".into(), value.into());
            }
            if let Some(value) = ["authorId", "uid", "id"]
                .iter()
                .find_map(|key| str_or_none(source.get(*key)))
            {
                found.insert("uploader_id".into(), value.into());
            }
            found
        })
        .find(|found| !found.is_empty())
        .unwrap_or_default();
    let channel_url = format_field(&author, &["channel_id"], TIKTOK_UPLOADER);
    let uploader_url = format_field(&author, &["uploader", "uploader_id"], TIKTOK_UPLOADER);
    info.extend(author);
    info.insert("channel_url".into(), channel_url);
    info.insert("uploader_url".into(), uploader_url);
    if let Some(music) = detail.get("music").filter(|music| music.is_object()) {
        if let Some(track) = music.get("title").and_then(Value::as_str) {
            info.insert("track".into(), track.into());
        }
        if let Some(album) = music
            .get("album")
            .and_then(Value::as_str)
            .filter(|album| !album.is_empty())
        {
            info.insert("album".into(), album.into());
        }
        if let Some(artists) = music
            .get("authorName")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        {
            let artists: Vec<Value> = ARTISTS.split(artists).map(Value::from).collect();
            info.insert("artists".into(), Value::Array(artists));
        }
        if let Some(duration) = int(music.get("duration")) {
            info.insert("duration".into(), duration.into());
        }
    }
    if let Some(desc) = detail.get("desc").and_then(Value::as_str) {
        info.insert("title".into(), truncate_title(desc).into());
        info.insert("description".into(), desc.into());
    }
    // Audio-only slideshows have a video duration of 0 (the music's stays).
    if let Some(duration) = int(detail.pointer("/video/duration")).filter(|d| *d != 0) {
        info.insert("duration".into(), duration.into());
    }
    if let Some(timestamp) = int(detail.get("createTime")) {
        info.insert("timestamp".into(), timestamp.into());
    }
    if let Some(stats) = detail.get("stats").filter(|stats| stats.is_object()) {
        for (key, source) in [
            ("view_count", "playCount"),
            ("like_count", "diggCount"),
            ("repost_count", "shareCount"),
            ("comment_count", "commentCount"),
            ("save_count", "collectCount"),
        ] {
            if let Some(count) = int(stats.get(source)) {
                info.insert(key.into(), count.into());
            }
        }
    }
    let mut thumbnails = Vec::new();
    for cover in ["thumbnail", "cover", "dynamicCover", "originCover"] {
        for source in [Some(detail), detail.get("video")].into_iter().flatten() {
            if let Some(url) = source
                .get(cover)
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            {
                thumbnails.push(json!({
                    "id": cover,
                    "url": proto_relative(&url),
                    "preference": if cover == "dynamicCover" { -2 } else { -1 },
                }));
            }
        }
    }
    info.insert("thumbnails".into(), Value::Array(thumbnails));
    info
}

/// `_availability(is_private, needs_subscription, is_unlisted)` (premium
/// and auth unknown, so never `public`).
fn availability(labels: &[&str]) -> Value {
    if labels.contains(&"Private") {
        "private".into()
    } else if labels.contains(&"Friends only") {
        "subscriber_only".into()
    } else if labels.contains(&"Followers only") {
        "unlisted".into()
    } else {
        Value::Null
    }
}

/// `determine_ext(url, default_ext='m4a')`.
fn determine_ext_or_m4a(url: &str) -> String {
    super::direct::determine_ext(url).unwrap_or_else(|| "m4a".into())
}

/// `_parse_aweme_video_app`'s formats (`extract_addr` for each address).
fn app_formats(video: &Value) -> Vec<Map<String, Value>> {
    // `known_resolutions`: the first height/width seen for a resolution.
    let mut known: Vec<(String, Map<String, Value>)> = Vec::new();
    let mut extract_addr = |addr: &Value, mut add_meta: Map<String, Value>| {
        let (mut parsed, res) = url_key(
            addr.get("url_key")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        let bytevc2 = parsed.get("vcodec").and_then(Value::as_str) == Some("bytevc2");
        if let Some(res) = res {
            let index = match known.iter().position(|(key, _)| *key == res) {
                Some(index) => index,
                None => {
                    known.push((res.clone(), Map::new()));
                    known.len() - 1
                }
            };
            let entry = &mut known[index].1;
            if !entry.contains_key("height") {
                entry.insert("height".into(), int(addr.get("height")).into());
            }
            if !entry.contains_key("width") {
                entry.insert("width".into(), int(addr.get("width")).into());
            }
            parsed.extend(entry.clone());
            if !add_meta.contains_key("height") {
                let height = res
                    .strip_suffix('p')
                    .and_then(|res| res.parse::<i64>().ok());
                add_meta.insert("height".into(), height.into());
            }
        }
        let note = add_meta
            .get("format_note")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut formats = Vec::new();
        for url in addr
            .get("url_list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let api = url.contains("aweme/v1");
            let mut format = Map::new();
            format.insert("url".into(), url.into());
            format.insert("filesize".into(), int(addr.get("data_size")).into());
            format.insert("ext".into(), "mp4".into());
            format.insert("acodec".into(), "aac".into());
            format.insert("source_preference".into(), if api { -2 } else { -1 }.into());
            format.extend(add_meta.clone());
            format.extend(parsed.clone());
            format.insert("preference".into(), if bytevc2 { -100 } else { -1 }.into());
            let parts: Vec<&str> = [
                note.as_deref(),
                api.then_some("(API)"),
                bytevc2.then_some("(UNPLAYABLE)"),
            ]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect();
            format.insert("format_note".into(), parts.join(" ").into());
            // `audio_meta(url)`.
            let ext = determine_ext_or_m4a(url);
            if ext == "mp3" || url.contains("-music-") {
                format.insert("format_note".into(), "Music track".into());
                format.insert("ext".into(), ext.clone().into());
                format.insert(
                    "acodec".into(),
                    if ext == "m4a" { "aac".to_owned() } else { ext }.into(),
                );
                format.insert("vcodec".into(), "none".into());
                format.insert("width".into(), Value::Null);
                format.insert("height".into(), Value::Null);
            }
            formats.push(format);
        }
        formats
    };
    let flag = |source: &Value, keys: &[&str]| {
        keys.iter()
            .find_map(|key| source.get(*key).filter(|value| !value.is_null()))
            .is_some_and(|value| truthy(Some(value)))
    };
    let width = int(video.get("width"));
    let height = int(video.get("height"));
    let ratio = match (width, height) {
        (Some(width), Some(height)) if height != 0 && width != 0 => {
            Some(width as f64 / height as f64)
        }
        _ => None,
    }
    .unwrap_or(0.5625);
    let mut formats = Vec::new();
    if let Some(addr) = video.get("play_addr").filter(|addr| truthy(Some(addr))) {
        let mut meta = Map::new();
        meta.insert("format_id".into(), "play_addr".into());
        meta.insert("format_note".into(), "Direct video".into());
        meta.insert(
            "vcodec".into(),
            if flag(video, &["is_bytevc1", "is_h265"]) {
                "h265"
            } else {
                "h264"
            }
            .into(),
        );
        meta.insert("width".into(), width.into());
        meta.insert("height".into(), height.into());
        formats.extend(extract_addr(addr, meta));
    }
    if let Some(addr) = video.get("download_addr").filter(|addr| truthy(Some(addr))) {
        let watermarked = truthy(video.get("has_watermark"));
        let dl_width = int(addr.get("width"));
        let mut meta = Map::new();
        meta.insert("format_id".into(), "download_addr".into());
        meta.insert(
            "format_note".into(),
            format!(
                "Download video{}",
                if watermarked { ", watermarked" } else { "" }
            )
            .into(),
        );
        meta.insert("vcodec".into(), "h264".into());
        meta.insert("width".into(), dl_width.into());
        // `download_addr['height']` is wrong.
        meta.insert(
            "height".into(),
            dl_width.map(|width| (width as f64 / ratio) as i64).into(),
        );
        meta.insert(
            "preference".into(),
            if watermarked { -2 } else { -1 }.into(),
        );
        formats.extend(extract_addr(addr, meta));
    }
    for (key, vcodec) in [("play_addr_h264", "h264"), ("play_addr_bytevc1", "h265")] {
        if let Some(addr) = video.get(key).filter(|addr| truthy(Some(addr))) {
            let mut meta = Map::new();
            meta.insert("format_id".into(), key.into());
            meta.insert("format_note".into(), "Direct video".into());
            meta.insert("vcodec".into(), vcodec.into());
            formats.extend(extract_addr(addr, meta));
        }
    }
    for bitrate in video
        .get("bit_rate")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(addr) = bitrate.get("play_addr").filter(|addr| truthy(Some(addr))) else {
            continue;
        };
        let mut meta = Map::new();
        meta.insert(
            "format_id".into(),
            bitrate.get("gear_name").cloned().unwrap_or(Value::Null),
        );
        meta.insert("format_note".into(), "Playback video".into());
        // `bit_rate / 1000` is a float.
        let tbr = bitrate
            .get("bit_rate")
            .and_then(Value::as_f64)
            .map(|rate| rate / 1000.0);
        meta.insert("tbr".into(), tbr.into());
        meta.insert(
            "vcodec".into(),
            if flag(bitrate, &["is_bytevc1", "is_h265"]) {
                "h265"
            } else {
                "h264"
            }
            .into(),
        );
        meta.insert(
            "fps".into(),
            bitrate.get("FPS").cloned().unwrap_or(Value::Null),
        );
        formats.extend(extract_addr(addr, meta));
    }
    remove_duplicate_formats(&mut formats);
    formats
}

/// A parsed aweme's info dict, formats and subtitles.
type AppVideo = (
    Map<String, Value>,
    Vec<Map<String, Value>>,
    crate::Subtitles,
);

/// `_parse_aweme_video_app`: the info dict, its formats and subtitles.
fn aweme_app(
    client: &Client,
    name: &'static str,
    detail: &Value,
    temp_id: &str,
    uploader_prefix: &str,
    webpage_host: &str,
) -> Result<AppVideo> {
    let id = str_or_none(detail.get("aweme_id"))
        .ok_or_else(|| bug(name, temp_id, "KeyError('aweme_id')"))?;
    let video = detail
        .get("video")
        .ok_or_else(|| bug(name, temp_id, "KeyError('video')"))?;
    if !video.is_object() {
        return Err(bug(
            name,
            temp_id,
            "AttributeError(\"object has no attribute 'get'\")",
        ));
    }
    let formats = app_formats(video);
    if let Some(sid) = crate::cookies::get_cookie(webpage_host, "sid_tt") {
        for format in &formats {
            if let Some(host) = format
                .get("url")
                .and_then(Value::as_str)
                .and_then(|url| Url::parse(url).ok())
                .and_then(|url| url.host_str().map(str::to_owned))
            {
                crate::cookies::set_cookie(&host, "sid_tt", &sid);
            }
        }
    }
    let empty = json!({});
    let stats = detail
        .get("statistics")
        .filter(|stats| truthy(Some(stats)))
        .unwrap_or(&empty);
    let music = detail
        .get("music")
        .filter(|music| truthy(Some(music)))
        .unwrap_or(&empty);
    let labels: Vec<&str> = detail
        .get("hybrid_label")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|label| label.get("text").and_then(Value::as_str))
        .collect();
    let first_str = |paths: &[&str]| {
        paths
            .iter()
            .find_map(|path| music.pointer(path).and_then(Value::as_str))
            .map(str::to_owned)
    };
    let contained_track = first_str(&["/matched_song/title", "/matched_pgc_sound/title"]);
    let contained_author = first_str(&[
        "/matched_song/author",
        "/matched_pgc_sound/author",
        "/author",
    ]);
    let owner = match music.get("owner_handle") {
        None | Some(Value::Null) => "None".to_owned(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    };
    let generic_title = truthy(music.get("is_original_sound"))
        && music.get("title").and_then(Value::as_str) == Some(&format!("original sound - {owner}"));
    let (track, music_author) = if generic_title {
        (
            Value::from(contained_track.unwrap_or_else(|| "original sound".into())),
            contained_author,
        )
    } else {
        (
            music.get("title").cloned().unwrap_or(Value::Null),
            music
                .get("author")
                .and_then(Value::as_str)
                .map(str::to_owned),
        )
    };
    let mut author = Map::new();
    if let Some(source) = detail.get("author").filter(|author| author.is_object()) {
        if let Some(value) = source.get("unique_id").and_then(Value::as_str) {
            author.insert("uploader".into(), value.into());
        }
        if let Some(value) = str_or_none(source.get("uid")) {
            author.insert("uploader_id".into(), value.into());
        }
        if let Some(value) = source.get("nickname").and_then(Value::as_str) {
            author.insert("channel".into(), value.into());
        }
        if let Some(value) = source.get("sec_uid").and_then(Value::as_str) {
            author.insert("channel_id".into(), value.into());
        }
    }
    let mut info = Map::new();
    info.insert("id".into(), id.clone().into());
    if let Some(desc) = detail.get("desc").and_then(Value::as_str) {
        info.insert("title".into(), truncate_title(desc).into());
        info.insert("description".into(), desc.into());
    }
    if let Some(timestamp) = int(detail.get("create_time")) {
        info.insert("timestamp".into(), timestamp.into());
    }
    for (key, source) in [
        ("view_count", "play_count"),
        ("like_count", "digg_count"),
        ("repost_count", "share_count"),
        ("comment_count", "comment_count"),
        ("save_count", "collect_count"),
    ] {
        if let Some(count) = int(stats.get(source)) {
            info.insert(key.into(), count.into());
        }
    }
    let channel_url = format_field(&author, &["channel_id"], uploader_prefix);
    let uploader_url = format_field(&author, &["uploader", "uploader_id"], uploader_prefix);
    let user_name = ["uploader", "uploader_id", "channel_id"]
        .iter()
        .find_map(|key| author.get(*key).and_then(Value::as_str))
        .map(str::to_owned);
    info.extend(author);
    info.insert("channel_url".into(), channel_url);
    info.insert("uploader_url".into(), uploader_url);
    info.insert("track".into(), track);
    info.insert(
        "album".into(),
        str_or_none(music.get("album"))
            .filter(|album| !album.is_empty())
            .map_or(Value::Null, Value::from),
    );
    info.insert(
        "artists".into(),
        match music_author.filter(|author| !author.is_empty()) {
            Some(author) => Value::Array(ARTISTS.split(&author).map(Value::from).collect()),
            None => Value::Null,
        },
    );
    info.insert("formats".into(), Value::Null);
    let subtitles = subtitles(client, name, detail, &id, user_name.as_deref())?;
    info.insert("subtitles".into(), Value::Null);
    let mut thumbnails = Vec::new();
    for cover in [
        "cover",
        "ai_dynamic_cover",
        "animated_cover",
        "ai_dynamic_cover_bak",
        "origin_cover",
        "dynamic_cover",
    ] {
        for url in video
            .pointer(&format!("/{cover}/url_list"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|url| !url.is_null())
        {
            thumbnails.push(json!({
                "id": cover,
                "url": url,
                "preference": if matches!(cover, "cover" | "origin_cover") { -1 } else { -2 },
            }));
        }
    }
    info.insert("thumbnails".into(), Value::Array(thumbnails));
    let duration = [
        video.get("duration"),
        video.pointer("/download_addr/duration"),
    ]
    .into_iter()
    .find_map(|value| int(value).map(|ms| ms.div_euclid(1000)))
    .filter(|duration| *duration != 0)
    .or_else(|| int(music.get("duration")));
    info.insert("duration".into(), duration.into());
    info.insert("availability".into(), availability(&labels));
    info.insert(
        "_format_sort_fields".into(),
        json!(["quality", "codec", "size", "br"]),
    );
    Ok((info, formats, subtitles))
}

/// The result of upstream's dict (`dict;KEYS` order).
fn video(
    url: &Url,
    name: &str,
    dict: Map<String, Value>,
    formats: Vec<Map<String, Value>>,
    subtitles: crate::Subtitles,
    id: &str,
) -> Result<VideoInfo> {
    let formats = formats
        .into_iter()
        .map(|format| {
            let keys: Vec<String> = format.keys().cloned().collect();
            // A float rate (`bit_rate / 1000`) stays a float.
            let float_rate = format.get("tbr").is_some_and(Value::is_f64);
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
                .map_err(|error| fail(name, id, &error.to_string()))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            if !float_rate {
                crate::info_order::mark_int_rates(&mut format);
            }
            Ok(format)
        })
        .collect::<Result<Vec<_>>>()?;
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
    info.extractor = name.into();
    info.extractor_key = name.into();
    let mut keys = Vec::new();
    let mut titled = false;
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => {
                titled = true;
                info.title = value.as_str().unwrap_or_default().to_owned();
            }
            "formats" | "subtitles" => {}
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
        info.title = String::new();
        info.extra
            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{};thumbnail=id,url,preference",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    ))
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(&super::input_text(url))
    ));
}

impl Extractor for TikTok {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        let user = found
            .name("user_id")
            .map_or("", |user| user.as_str())
            .to_owned();
        start(NAME, url);
        let page = Url::parse(&create_url(&user, &id))?;
        let (detail, status) = web_data(client, &page, NAME, &id, true)?;
        if truthy(Some(&detail)) && status == 0 {
            let dict = aweme_web(&detail, page.as_str(), &id);
            let formats = web_formats(&detail);
            let subtitles = subtitles(client, NAME, &detail, &id, None)?;
            // Processing's `webpage_url` is the input URL.
            return video(url, NAME, dict, formats, subtitles, &id).map(Some);
        }
        Err(match status {
            10216 | 10222 => login_required(
                NAME,
                &id,
                "You do not have permission to view this post. Log into an account that has access",
            ),
            10204 => fail(
                NAME,
                &id,
                "Your IP address is blocked from accessing this post",
            ),
            _ => unexpected(
                NAME,
                &id,
                &format!("Video not available, status code {status}"),
            ),
        })
    }
}

impl Extractor for Douyin {
    fn key(&self) -> &'static str {
        DOUYIN_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = DOUYIN_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        start(DOUYIN_NAME, url);
        let api = super::update_url_query(
            &Url::parse("https://www.douyin.com/aweme/v1/web/aweme/detail/")?,
            &[("aweme_id", &id)],
        );
        // `_download_json(..., fatal=False)`.
        let response = super::download_json_note(
            client,
            &api,
            DOUYIN_NAME,
            &id,
            "Downloading web detail JSON",
        );
        let response = match response {
            Ok(response) => response,
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{DOUYIN_NAME}] {id}: Failed to download web detail JSON: {}",
                    crate::upstream_error_text(&source)
                ));
                Value::Null
            }
            Err(Error::Reported(text)) if text.contains(": Failed to parse JSON ") => {
                // `_parse_json(fatal=False)`'s warning; an empty body is the
                // usual response without the signature cookies.
                crate::screen::warning(&format!(
                    "[{DOUYIN_NAME}] {id}: Failed to parse JSON: {}",
                    python_json_error(&text)
                ));
                Value::Null
            }
            Err(error) => return Err(error),
        };
        let detail = response
            .get("aweme_detail")
            .filter(|detail| detail.is_object())
            .cloned();
        let Some(detail) = detail else {
            // Upstream's TODO: the verification challenge for signature cookies.
            let message = "Fresh cookies (not necessarily logged in) are needed";
            if crate::cookies::get_cookie(DOUYIN_HOST, "s_v_web_id").is_some() {
                return Err(unexpected(DOUYIN_NAME, &id, message));
            }
            return Err(fail(DOUYIN_NAME, &id, message));
        };
        let (dict, formats, subtitles) = aweme_app(
            client,
            DOUYIN_NAME,
            &detail,
            &id,
            DOUYIN_UPLOADER,
            DOUYIN_HOST,
        )?;
        video(url, DOUYIN_NAME, dict, formats, subtitles, &id).map(Some)
    }
}

/// Python's `JSONDecodeError` text for the bodies this port recognizes
/// (an empty or blank body), else the parser's own message.
fn python_json_error(text: &str) -> String {
    if text.contains("EOF while parsing a value at line 1 column 0") {
        return "Expecting value in '': line 1 column 1 (char 0)".into();
    }
    text.split_once("(caused by ")
        .map_or(text, |(_, cause)| cause.trim_end_matches(')'))
        .to_owned()
}

impl Extractor for TikTokVm {
    fn key(&self) -> &'static str {
        VM_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        start(VM_NAME, url);
        // `_request_webpage(HEADRequest(url), headers={'User-Agent':
        // 'facebookexternalhit/1.1'}).url`.
        crate::screen::screen(&format!("[{VM_NAME}] {id}: Downloading webpage"));
        let response = super::direct::head_with_headers(
            client,
            url,
            &[("User-Agent", "facebookexternalhit/1.1")],
        )
        .map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: VM_NAME,
                id: id.clone(),
                what: "webpage",
                source: Box::new(error),
            },
            error => error,
        })?;
        let target = response.url().clone();
        if vm_suitable(target.as_str()) {
            return Err(Error::Reported(format!("Unsupported URL: {target}")));
        }
        let mut info = super::direct::build_at(target.clone(), &target, String::new());
        info.formats.clear();
        info.direct = None;
        info.extra
            .insert(crate::FOLLOW_URL.into(), target.as_str().into());
        Ok(Some(ExtractResult::Video(info)))
    }
}

/// `url_result(webpage_url, TikTokIE, **_parse_aweme_video_web(...,
/// extract_flat=True))`: a playlist entry resolved by `TikTokIE`.
fn flat_entry(item: &Value, webpage_url: &str, id: &str) -> Result<VideoInfo> {
    let mut entry = aweme_web(item, webpage_url, id);
    entry.insert("ie_key".into(), NAME.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), webpage_url.into());
    let url = Url::parse(webpage_url)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = id.to_owned();
    info.title = entry
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": webpage_url,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `playlist_result(entries, playlist_id, playlist_title)`.
fn playlist_result(
    page: &Url,
    name: &str,
    key: &str,
    id: &str,
    title: Option<String>,
    state: Value,
) -> ExtractResult {
    let mut extra = Map::new();
    // Without a title, `playlist_result` sets no `title` key.
    let keys = if title.is_some() { "id,title" } else { "id" };
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{keys}").into(),
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
        // The entries are a generator, fetched when the playlist is processed.
        pending: Some(crate::PendingEntries { state }),
        entries: Vec::new(),
        extra,
    })
}

/// The items of an item list response with an `id` (`lambda _, v: v['id']`).
fn items(response: &Value) -> Vec<Value> {
    response
        .get("itemList")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| truthy(item.get("id")))
        .cloned()
        .collect()
}

fn item_id(item: &Value) -> String {
    str_or_none(item.get("id")).unwrap_or_default()
}

/// Upstream's default `extractor_retries`.
const EXTRACTOR_RETRIES: u32 = 3;

impl TikTokUser {
    /// `_build_web_query(sec_uid, cursor)`.
    fn query(sec_uid: &str, cursor: i64, device_id: &str) -> Vec<(&'static str, String)> {
        const HEX: &[u8] = b"0123456789abcdefABCDEF";
        let seed = random_u64(cursor as u64);
        let verify: String = (0..7)
            .map(|index| HEX[((seed >> (index * 5)) % HEX.len() as u64) as usize] as char)
            .collect();
        [
            ("aid", "1988".to_owned()),
            ("app_language", "en".into()),
            ("app_name", "tiktok_web".into()),
            ("browser_language", "en-US".into()),
            ("browser_name", "Mozilla".into()),
            ("browser_online", "true".into()),
            ("browser_platform", "Win32".into()),
            ("browser_version", "5.0 (Windows)".into()),
            ("channel", "tiktok_web".into()),
            ("cookie_enabled", "true".into()),
            ("count", "15".into()),
            ("cursor", cursor.to_string()),
            ("device_id", device_id.into()),
            ("device_platform", "web_pc".into()),
            ("focus_state", "true".into()),
            ("from_page", "user".into()),
            ("history_len", "2".into()),
            ("is_fullscreen", "false".into()),
            ("is_page_visible", "true".into()),
            ("language", "en".into()),
            ("os", "windows".into()),
            ("priority_region", String::new()),
            ("referer", String::new()),
            ("region", "US".into()),
            ("screen_height", "1080".into()),
            ("screen_width", "1920".into()),
            ("secUid", sec_uid.into()),
            // Pagination: 1 is newest to oldest.
            ("type", "1".into()),
            ("tz_name", "UTC".into()),
            ("verifyFp", format!("verify_{verify}")),
            ("webcast_language", "en".into()),
        ]
        .into()
    }

    /// `_DEVICE_ID`: the `device_id` extractor argument, else random.
    fn random_device_id() -> String {
        let span = 7_325_099_899_999_994_577u64 - 7_250_000_000_000_000_000 + 1;
        (7_250_000_000_000_000_000 + random_u64(1) % span).to_string()
    }

    /// `_extract_sec_uid_from_embed`: the `channel_id` of the first video
    /// listed on the user's embed page whose webpage gives one.
    fn sec_uid_from_embed(client: &Client, user_name: &str) -> Result<Option<String>> {
        static FRONTITY: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"<script[^>]+\bid=['"]__FRONTITY_CONNECT_STATE__['"][^>]*>"#).unwrap()
        });
        let embed = Url::parse(&format!("https://www.tiktok.com/embed/@{user_name}"))?;
        let request = PageRequest {
            name: USER_NAME,
            id: user_name,
            note: "Downloading user embed page",
            what: None,
            fatal: false,
            impersonate: false,
        };
        let page = download_page(client, &embed, request)?
            .map(|(page, _)| page)
            .filter(|page| !page.is_empty());
        let Some(page) = page else {
            crate::screen::warning(&format!(
                "[{USER_NAME}] This user's account is either private or has embedding disabled"
            ));
            return Ok(None);
        };
        let data = match super::search_json(&FRONTITY, &page, false) {
            Some(Some(data)) => data,
            _ => json!({}),
        };
        let videos = data
            .get("source")
            .and_then(|source| source.get("data"))
            .and_then(|data| data.get(format!("/embed/@{user_name}")))
            .filter(|data| data.is_object())
            .and_then(|data| data.get("videoList"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for aweme_id in videos
            .iter()
            .filter_map(|video| video.get("id").and_then(Value::as_str))
        {
            let webpage_url = create_url(user_name, aweme_id);
            let page = Url::parse(&webpage_url)?;
            let (video, _) = web_data(client, &page, USER_NAME, aweme_id, false)?;
            let sec_uid = aweme_web(&video, &webpage_url, aweme_id)
                .get("channel_id")
                .and_then(Value::as_str)
                .filter(|sec_uid| !sec_uid.is_empty())
                .map(str::to_owned);
            if sec_uid.is_some() {
                return Ok(sec_uid);
            }
        }
        Ok(None)
    }

    /// One page of `_entries(sec_uid, user_name, fail_early)`: its new
    /// videos, the next cursor, and whether the list ended.
    fn page(
        client: &Client,
        state: &mut Map<String, Value>,
        entries: &mut Vec<VideoInfo>,
    ) -> Result<()> {
        let text = |key: &str| {
            state
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let sec_uid = text("sec_uid");
        let display_id = text("display_id");
        let known_device = state.get("known_device") == Some(&Value::Bool(true));
        let fail_early = state.get("fail_early") == Some(&Value::Bool(true));
        let mut device_id = text("device_id");
        let page = state.get("page").and_then(Value::as_u64).unwrap_or(1);
        let cursor = state.get("cursor").and_then(Value::as_i64).unwrap_or(0);
        let mut seen: Vec<String> = state
            .get("seen")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let mut retry = 0;
        let response = loop {
            let query = Self::query(&sec_uid, cursor, &device_id);
            let pairs: Vec<(&str, &str)> = query
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect();
            let api = super::update_url_query(
                &Url::parse("https://www.tiktok.com/api/creator/item_list/")?,
                &pairs,
            );
            let response = super::download_json_note(
                client,
                &api,
                USER_NAME,
                &display_id,
                &format!("Downloading page {page}"),
            )?;
            // Avoid an infinite loop caused by a bad device_id.
            let mut batch: Vec<String> = response
                .get("itemList")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .map(str::to_owned)
                .collect();
            batch.sort();
            let mut sorted_seen = seen.clone();
            sorted_seen.sort();
            if batch.is_empty() || batch != sorted_seen {
                break response;
            }
            let message = "TikTok API keeps sending the same page";
            if known_device {
                return Err(Error::Reported(format!(
                    "{message}. Try again with a different device_id"
                )));
            }
            device_id = Self::random_device_id();
            let error = format!("{message}. Taking measures to avoid an infinite loop");
            retry += 1;
            if retry > EXTRACTOR_RETRIES {
                return Err(Error::Reported(error));
            }
            crate::screen::warning(&format!(
                "[{USER_NAME}] {error}. Retrying ({retry}/{EXTRACTOR_RETRIES})..."
            ));
        };
        for item in items(&response) {
            let id = item_id(&item);
            if seen.contains(&id) {
                continue;
            }
            seen.push(id.clone());
            let webpage_url = create_url(&display_id, &id);
            entries.push(flat_entry(&item, &webpage_url, &id)?);
        }
        let old_cursor = cursor;
        let mut cursor = response
            .pointer("/itemList")
            .and_then(Value::as_array)
            .and_then(|list| list.last())
            .and_then(|item| item.get("createTime"))
            .and_then(|time| {
                time.as_i64()
                    .map(|time| time * 1000)
                    .or_else(|| time.as_f64().map(|time| (time * 1e3) as i64))
            })
            .unwrap_or(0);
        if cursor == 0 || old_cursor == cursor {
            // The user may not have posted within this ~1 week lookback.
            cursor = old_cursor - 7 * 86_400_000;
        }
        state.insert("device_id".into(), device_id.into());
        state.insert("page".into(), (page + 1).into());
        state.insert("cursor".into(), cursor.into());
        state.insert("seen".into(), json!(seen));
        // `hasMorePrevious` may be wrong: stop before TikTok existed.
        if cursor < 1_472_706_000_000 || !truthy(response.get("hasMorePrevious")) {
            state.insert("done".into(), true.into());
            return Ok(());
        }
        if fail_early && seen.is_empty() {
            return Err(Error::Reported(format!(
                "This user's account is likely either private or all of their videos are private. Log into an account that has access. {LOGIN_HINT}"
            )));
        }
        Ok(())
    }

    /// The lazily fetched `_entries` generator's initial state.
    fn entries_state(sec_uid: &str, user_name: Option<&str>, fail_early: bool) -> Value {
        let known_device = crate::manifest_args::configuration_arg(NAME, "device_id", false)
            .and_then(|values| values.into_iter().next());
        let cursor = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_millis() as i64);
        json!({
            "tiktok": "user",
            "sec_uid": sec_uid,
            "display_id": user_name.unwrap_or(sec_uid),
            "fail_early": fail_early,
            "known_device": known_device.is_some(),
            "device_id": known_device.unwrap_or_else(Self::random_device_id),
            "cursor": cursor,
            "page": 1,
            "seen": [],
        })
    }
}

/// `TikTokCollectionIE._entries`: one page of the collection.
fn collection_page(
    client: &Client,
    state: &mut Map<String, Value>,
    entries: &mut Vec<VideoInfo>,
) -> Result<()> {
    const PAGE_COUNT: i64 = 30;
    let id = state
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let page = state.get("page").and_then(Value::as_u64).unwrap_or(1);
    let cursor = state.get("cursor").and_then(Value::as_i64).unwrap_or(0);
    let api = super::update_url_query(
        &Url::parse("https://www.tiktok.com/api/collection/item_list/")?,
        &[
            ("aid", "1988"),
            ("collectionId", &id),
            ("count", &PAGE_COUNT.to_string()),
            ("cursor", &cursor.to_string()),
            ("sourceType", "113"),
        ],
    );
    let response = super::download_json_note(
        client,
        &api,
        COLLECTION_NAME,
        &id,
        &format!("Downloading page {page}"),
    )?;
    for item in items(&response) {
        let video_id = item_id(&item);
        let author = ["uniqueId", "secUid", "id"]
            .iter()
            .find_map(|key| {
                item.get("author")
                    .and_then(|author| author.get(*key))
                    .and_then(Value::as_str)
            })
            .unwrap_or("_");
        let webpage_url = create_url(author, &video_id);
        entries.push(flat_entry(&item, &webpage_url, &video_id)?);
    }
    state.insert("page".into(), (page + 1).into());
    state.insert("cursor".into(), (cursor + PAGE_COUNT).into());
    if !truthy(response.get("hasMore")) {
        state.insert("done".into(), true.into());
    }
    Ok(())
}

/// The user and collection playlists' lazily fetched entries (upstream
/// generators): pages are fetched until `limit` entries exist or the list
/// ends; the state is kept while the generator is not known to be exhausted.
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let Value::Object(mut state) = state else {
        return Ok(None);
    };
    loop {
        if state.get("done") == Some(&Value::Bool(true)) {
            return Ok(None);
        }
        if limit.is_some_and(|limit| entries.len() >= limit) {
            return Ok(Some(Value::Object(state)));
        }
        match state.get("tiktok").and_then(Value::as_str) {
            Some("user") => TikTokUser::page(client, &mut state, entries)?,
            Some("collection") => collection_page(client, &mut state, entries)?,
            _ => return Ok(None),
        }
    }
}

/// Whether a playlist's pending state is a TikTok list's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state.get("tiktok").is_some()
}

impl Extractor for TikTokUser {
    fn key(&self) -> &'static str {
        USER_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let input = super::input_text(url);
        let Some(found) = USER_URL.captures(&input) else {
            return Ok(None);
        };
        let matched = found["id"].to_owned();
        start(USER_NAME, url);
        let (user_name, sec_uid, fail_early) = if SEC_UID.is_match(&matched) {
            (None, Some(matched.clone()), true)
        } else {
            let page_url = Url::parse(&format!("{TIKTOK_UPLOADER}{matched}"))?;
            let request = PageRequest {
                name: USER_NAME,
                id: &matched,
                note: "Downloading user webpage",
                what: Some("user webpage"),
                fatal: false,
                impersonate: true,
            };
            let page = match download_page(client, &page_url, request)? {
                Some((page, _)) => page,
                None => String::new(),
            };
            let detail = universal_data(&page)
                .and_then(|data| data.get("webapp.user-detail").cloned())
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            let video_count = ["/userInfo/stats/videoCount", "/userInfo/statsV2/videoCount"]
                .iter()
                .find_map(|path| detail.pointer(path).and_then(Value::as_i64));
            if video_count.is_none_or(|count| count == 0)
                && detail.get("statusCode").and_then(Value::as_i64) == Some(10222)
            {
                return Err(login_required(
                    USER_NAME,
                    &matched,
                    "This user's account is private. Log into an account that has access",
                ));
            }
            if video_count == Some(0) {
                return Err(fail(
                    USER_NAME,
                    &matched,
                    "This account does not have any videos posted",
                ));
            }
            match detail
                .pointer("/userInfo/user/secUid")
                .and_then(Value::as_str)
            {
                Some(sec_uid) => {
                    let listed = detail
                        .pointer("/userInfo/itemList")
                        .and_then(Value::as_array)
                        .is_some_and(|items| items.iter().any(|item| !item.is_null()));
                    (Some(matched.clone()), Some(sec_uid.to_owned()), !listed)
                }
                None => (
                    Some(matched.clone()),
                    Self::sec_uid_from_embed(client, &matched)?,
                    false,
                ),
            }
        };
        let Some(sec_uid) = sec_uid else {
            return Err(fail(
                USER_NAME,
                &matched,
                "Unable to extract secondary user ID. If you are able to get the channel_id from a video posted by this user, try using \"tiktokuser:channel_id\" as the input URL (replacing `channel_id` with its actual value)",
            ));
        };
        let state = Self::entries_state(&sec_uid, user_name.as_deref(), fail_early);
        Ok(Some(playlist_result(
            url, USER_NAME, USER_KEY, &sec_uid, user_name, state,
        )))
    }
}

impl Extractor for TikTokCollection {
    fn key(&self) -> &'static str {
        COLLECTION_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = COLLECTION_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        let title = format!("{}-{}", &found["user_id"], &found["title"]);
        start(COLLECTION_NAME, url);
        let state = json!({"tiktok": "collection", "id": id, "cursor": 0, "page": 1});
        Ok(Some(playlist_result(
            url,
            COLLECTION_NAME,
            COLLECTION_KEY,
            &id,
            Some(title),
            state,
        )))
    }
}

/// `TikTokLiveIE._call_api(url, param, room_id, uploader, key)`: the live
/// room's info (status 2 is live).
fn live_api(
    client: &Client,
    url: &str,
    param: &str,
    room_id: &str,
    uploader: Option<&str>,
    key: &str,
) -> Result<Map<String, Value>> {
    let api = super::update_url_query(&Url::parse(url)?, &[("aid", "1988"), (param, room_id)]);
    // `_download_json(..., fatal=False)`.
    let response = match super::download_json(client, &api, LIVE_NAME, room_id) {
        Ok(response) => response,
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{LIVE_NAME}] {room_id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Value::Null
        }
        Err(Error::Reported(text)) if text.contains(": Failed to parse JSON ") => {
            crate::screen::warning(&format!(
                "[{LIVE_NAME}] {room_id}: Failed to parse JSON: {}",
                python_json_error(&text)
            ));
            Value::Null
        }
        Err(error) => return Err(error),
    };
    let info = response
        .get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if int(info.get("status")) == Some(2) {
        return Ok(info);
    }
    // A room ID from a mobile share URL cannot be refreshed: no waiting.
    match uploader {
        None => Err(fail(LIVE_NAME, room_id, "This livestream has ended")),
        Some(uploader) => Err(not_live(uploader)),
    }
}

/// `UserNotLive(video_id=uploader)`.
fn not_live(uploader: &str) -> Error {
    fail(LIVE_NAME, uploader, "The channel is not currently live")
}

/// `merge_dicts(a, b)`: `b`'s keys missing from `a` (or filling its empty
/// strings); `None` values are dropped.
fn merge_dicts(first: Map<String, Value>, second: Map<String, Value>) -> Map<String, Value> {
    let mut merged = Map::new();
    for (key, value) in first.into_iter().chain(second) {
        let fills = value.is_string() && merged.get(&key).and_then(Value::as_str) == Some("");
        if (!value.is_null() && !merged.contains_key(&key)) || fills {
            merged.insert(key, value);
        }
    }
    merged
}

impl Extractor for TikTokLive {
    fn key(&self) -> &'static str {
        LIVE_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        static SIGI_STATE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"<script[^>]+\bid="(?:SIGI_STATE|sigi-persisted-data)"[^>]*>"#).unwrap()
        });
        let Some(found) = LIVE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let mut uploader = found.name("uploader").map(|m| m.as_str().to_owned());
        let mut room_id = found.name("id").map(|m| m.as_str().to_owned());
        start(LIVE_NAME, url);
        if room_id.is_none() {
            let user = uploader.clone().unwrap_or_default();
            let page = Url::parse(&format!("{TIKTOK_UPLOADER}{user}"))?;
            let request = PageRequest {
                name: LIVE_NAME,
                id: &user,
                note: "Downloading webpage",
                what: Some("webpage"),
                fatal: true,
                impersonate: true,
            };
            let page = download_page(client, &page, request)?
                .map(|(page, _)| page)
                .unwrap_or_default();
            room_id = universal_data(&page)
                .and_then(|data| {
                    data.pointer("/webapp.user-detail/userInfo/user/roomId")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .filter(|room| !room.is_empty());
        }
        if uploader.is_none() || room_id.is_none() {
            let display = uploader.clone().or(room_id.clone()).unwrap_or_default();
            let request = PageRequest {
                name: LIVE_NAME,
                id: &display,
                note: "Downloading webpage",
                what: Some("webpage"),
                fatal: room_id.is_none(),
                impersonate: false,
            };
            let page = download_page(client, url, request)?
                .map(|(page, _)| page)
                .unwrap_or_default();
            let data = match super::search_json(&SIGI_STATE, &page, false) {
                Some(Some(data)) => data,
                _ => json!({}),
            };
            let users: Vec<Value> = std::iter::once(
                data.pointer("/LiveRoom/liveRoomUserInfo/user")
                    .cloned()
                    .unwrap_or(Value::Null),
            )
            .chain(
                data.pointer("/UserModule/users")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|users| users.values().cloned()),
            )
            .collect();
            let first = |key: &str| {
                users
                    .iter()
                    .find_map(|user| user.get(key).and_then(Value::as_str))
                    .map(str::to_owned)
            };
            room_id = room_id.or_else(|| first("roomId"));
            uploader = uploader.or_else(|| first("uniqueId"));
        }
        let Some(room_id) = room_id.filter(|room| !room.is_empty()) else {
            return Err(not_live(uploader.as_deref().unwrap_or("None")));
        };
        let mut info = live_api(
            client,
            "https://webcast.tiktok.com/webcast/room/info",
            "room_id",
            &room_id,
            uploader.as_deref(),
            "data",
        )?;
        let quality = |name: &str| -> i64 {
            [
                "SD1", "ld", "SD2", "sd", "HD1", "hd", "FULL_HD1", "uhd", "ORIGION", "origin",
            ]
            .iter()
            .position(|known| *known == name)
            .map_or(-1, |index| index as i64)
        };
        let inner = |value: Option<&Value>| -> Option<Value> {
            serde_json::from_str(value?.as_str()?).ok()
        };
        let mut formats: Vec<Map<String, Value>> = Vec::new();
        let streams = inner(
            info.get("stream_url")
                .and_then(|stream| stream.pointer("/live_core_sdk_data/pull_data/stream_data")),
        )
        .and_then(|data| data.get("data").and_then(Value::as_object).cloned())
        .unwrap_or_default();
        for (name, stream) in &streams {
            let mut sdk = Map::new();
            if let Some(params) = inner(stream.pointer("/main/sdk_params")) {
                if let Some(vcodec) = params.get("VCodec").and_then(Value::as_str) {
                    sdk.insert("vcodec".into(), vcodec.into());
                }
                if let Some(tbr) = int(params.get("vbitrate")).map(|rate| rate.div_euclid(1000)) {
                    sdk.insert("tbr".into(), tbr.into());
                }
                static RESOLUTION: LazyLock<Regex> =
                    LazyLock::new(|| Regex::new(r"^(?i:\d+x\d+|\d+p)").unwrap());
                if let Some(resolution) = params
                    .get("resolution")
                    .and_then(Value::as_str)
                    .and_then(|text| RESOLUTION.find(text))
                {
                    sdk.insert(
                        "resolution".into(),
                        resolution.as_str().to_lowercase().into(),
                    );
                }
            }
            let url_of = |key: &str| {
                stream
                    .pointer(&format!("/main/{key}"))
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
            };
            if let Some(flv) = url_of("flv") {
                let mut format = Map::new();
                format.insert("url".into(), flv.into());
                format.insert("ext".into(), "flv".into());
                format.insert("format_id".into(), format!("flv-{name}").into());
                format.insert("quality".into(), quality(name).into());
                format.extend(sdk.clone());
                formats.push(format);
            }
            if let Some(hls) = url_of("hls") {
                let mut format = Map::new();
                format.insert("url".into(), hls.into());
                format.insert("ext".into(), "mp4".into());
                format.insert("protocol".into(), "m3u8_native".into());
                format.insert("format_id".into(), format!("hls-{name}").into());
                format.insert("quality".into(), quality(name).into());
                format.extend(sdk);
                formats.push(format);
            }
        }
        let stream_url = info.get("stream_url").cloned().unwrap_or(Value::Null);
        let vcodec = |keys: &[&str]| -> Value {
            let mut value = Some(&stream_url);
            for key in keys {
                value = value.and_then(|value| value.get(*key));
            }
            inner(value)
                .and_then(|params| {
                    params
                        .get("VCodec")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .map_or(Value::Null, Value::from)
        };
        for (stream, ext, protocol) in [("hls", "mp4", "m3u8_native"), ("rtmp", "flv", "https")] {
            if let Some(pull) = stream_url
                .get(format!("{stream}_pull_url"))
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            {
                let mut format = Map::new();
                format.insert("url".into(), pull.into());
                format.insert("ext".into(), ext.into());
                format.insert("protocol".into(), protocol.into());
                format.insert("format_id".into(), format!("{stream}-pull").into());
                format.insert(
                    "vcodec".into(),
                    vcodec(&[&format!("{stream}_pull_url_params")]),
                );
                format.insert("quality".into(), quality("ORIGION").into());
                formats.push(format);
            }
        }
        if let Some(pulls) = stream_url.get("flv_pull_url").and_then(Value::as_object) {
            for (name, pull) in pulls {
                let Some(pull) = pull.as_str().and_then(super::json_ld::url_or_none) else {
                    continue;
                };
                let mut format = Map::new();
                format.insert("url".into(), pull.into());
                format.insert("ext".into(), "flv".into());
                format.insert(
                    "format_id".into(),
                    format!("flv-{name}").to_lowercase().into(),
                );
                format.insert("vcodec".into(), vcodec(&["flv_pull_url_params", name]));
                format.insert("quality".into(), quality(name).into());
                formats.push(format);
            }
        }
        // A guest on another's livestream has no m3u8 URLs here.
        if !formats
            .iter()
            .any(|format| format.get("ext").and_then(Value::as_str) == Some("mp4"))
        {
            let detail = live_api(
                client,
                "https://www.tiktok.com/api/live/detail/",
                "roomID",
                &room_id,
                uploader.as_deref(),
                "LiveRoomInfo",
            )?;
            info = merge_dicts(info, detail);
            if let Some(live_url) = info
                .get("liveUrl")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            {
                let mut format = Map::new();
                format.insert("url".into(), live_url.into());
                format.insert("ext".into(), "mp4".into());
                format.insert("protocol".into(), "m3u8_native".into());
                format.insert("format_id".into(), "hls-fallback".into());
                format.insert("vcodec".into(), "h264".into());
                format.insert("quality".into(), quality("origin").into());
                formats.push(format);
            }
        }
        let uploader = uploader.or_else(|| {
            [
                info.get("ownerInfo")
                    .and_then(|owner| owner.get("uniqueId")),
                info.get("owner").and_then(|owner| owner.get("display_id")),
            ]
            .into_iter()
            .flatten()
            .find_map(Value::as_str)
            .map(str::to_owned)
        });
        let mut dict = Map::new();
        dict.insert("id".into(), room_id.clone().into());
        dict.insert("uploader".into(), uploader.clone().into());
        dict.insert(
            "uploader_url".into(),
            uploader
                .filter(|uploader| !uploader.is_empty())
                .map(|uploader| format!("{TIKTOK_UPLOADER}{uploader}"))
                .into(),
        );
        dict.insert("is_live".into(), true.into());
        dict.insert("formats".into(), Value::Null);
        dict.insert("_format_sort_fields".into(), json!(["quality", "ext"]));
        if let Some(title) = info.get("title").filter(|title| !title.is_null()) {
            dict.insert("title".into(), title.clone());
        }
        let owners = [info.get("ownerInfo"), info.get("owner")];
        if let Some(id) = owners
            .iter()
            .flatten()
            .find_map(|owner| str_or_none(owner.get("id")))
        {
            dict.insert("uploader_id".into(), id.into());
        }
        if let Some(nickname) = owners
            .iter()
            .flatten()
            .find_map(|owner| owner.get("nickname").filter(|name| !name.is_null()))
        {
            dict.insert("creator".into(), nickname.clone());
        }
        if let Some(count) = [
            info.get("user_count"),
            info.get("liveRoomStats")
                .and_then(|stats| stats.get("userCount")),
        ]
        .into_iter()
        .find_map(int)
        {
            dict.insert("concurrent_view_count".into(), count.into());
        }
        let mut video = video(
            url,
            LIVE_NAME,
            dict,
            formats,
            crate::Subtitles::default(),
            &room_id,
        )?;
        video.extractor_key = LIVE_KEY.into();
        Ok(Some(video))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.tiktok.com/@leenabhushan/video/6748451240264420610"
        ));
        assert!(suitable("https://www.tiktok.com/embed/6748451240264420610"));
        assert!(suitable(
            "https://www.tiktokv.com/share/video/6748451240264420610"
        ));
        assert!(!suitable("https://www.tiktok.com/@leenabhushan"));
        assert!(vm_suitable("https://vm.tiktok.com/ZTR45GpSF/"));
        assert!(vm_suitable("https://www.tiktok.com/t/ZTRC5xgJp"));
        assert!(douyin_suitable(
            "https://www.douyin.com/video/6961737553342991651"
        ));
        assert!(douyin_suitable(
            "https://douyin.com/video/6961737553342991651"
        ));
        assert!(user_suitable("https://tiktok.com/@corgibobaa?lang=en"));
        assert!(user_suitable("https://www.tiktok.com/@meme"));
        assert!(user_suitable(
            "tiktokuser:MS4wLjABAAAAM3R2BtjzVT-uAtstkl2iugMzC6AtnpkojJbjiOdDDrdsTiTR75-8lyWJCY5VvDrZ"
        ));
        assert!(!user_suitable("https://www.tiktok.com/@meme/video/1"));
        assert!(!user_suitable("https://www.tiktok.com/@meme/live"));
        assert!(collection_suitable(
            "https://www.tiktok.com/@imanoreotwe/collection/count-test-7371330159376370462"
        ));
        assert!(collection_suitable(
            "https://www.tiktok.com/@imanoreotwe/collection/%F0%9F%98%82-7111887189571160875"
        ));
    }

    #[test]
    fn sha256_matches_known_digests() {
        let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(sha256(&[b'a'; 1000])),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }
}
