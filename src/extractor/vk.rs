//! Upstream `VKIE` (`yt_dlp/extractor/vk.py`): `al_video.php`'s `show`
//! payload for video/clip pages and `video_ext.php` embeds (with their
//! error pages), the player params' progressive, HLS, DASH and RTMP
//! formats, subtitles, chapters and metadata. A YouTube embed is followed;
//! other embedded players are reported as unsupported. VK's JS challenge
//! is solved with the native [`crate::jsinterp`] interpreter.
//!
//! `VKWallPostIE`: `wkview.php`'s wall post HTML as a playlist of its
//! `data-audio` tracks and linked videos. `VKUserVideosIE`: user/club video
//! sections and playlists through `al_video.php`'s `load_videos_silent`
//! pages. `VKPlayIE`/`VKPlayLiveIE` are in [`super::vkplay`].
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Vk;
pub struct VkWallPost;
pub struct VkUserVideos;

const NAME: &str = "vk";
const KEY: &str = "VK";
const WALL_POST: &str = "vk:wallpost";
const USER_VIDEOS: &str = "vk:uservideos";
const LOGIN_HINT: &str = "Use --cookies, --cookies-from-browser, --username and --password, --netrc-cmd, or --netrc (vk) to provide account credentials. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^https?://(?:",
        r"(?:(?:(?:m|new|vksport)\.)?vk(?:(?:video)?\.ru|\.com)/video_|(?:www\.)?daxab\.com/)",
        r"ext\.php\?(?P<embed_query>.*?\boid=(?P<oid>-?\d+).*?\bid=(?P<id>\d+).*)|",
        r"(?:(?:(?:m|new|vksport)\.)?vk(?:(?:video)?\.ru|\.com)/(?:.+?\?.*?z=)?(?:video|clip)|(?:www\.)?daxab\.com/embed/)",
        r"(?P<videoid>-?\d+_\d+)(?:.*\blist=(?P<list_id>([\da-f]+)|(ln-[\da-zA-Z]+)))?",
        r")"
    ))
    .unwrap()
});
static ERROR_MESSAGES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"(?s)<!><div[^>]+class="video_layer_message"[^>]*>(.+?)</div>"#).unwrap(),
        Regex::new(r#"(?s)<div[^>]+id="video_ext_msg"[^>]*>(.+?)</div>"#).unwrap(),
    ]
});
static SECURITY_CHECK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!>/login\.php\?.*\bact=security_check").unwrap());
static ERRORS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    const COPYRIGHT: &str =
        "Video %s has been removed from public access due to rightholder complaint.";
    const BLOCKED: &str = "Video %s is no longer available, because its author has been blocked.";
    [
        (
            r">Видеозапись .*? была изъята из публичного доступа в связи с обращением правообладателя.<",
            COPYRIGHT,
        ),
        (
            r">The video .*? was removed from public access by request of the copyright holder.<",
            COPYRIGHT,
        ),
        (
            r"<!>Please log in or <",
            "Video %s is only available for registered users, use --username and --password options to provide account credentials.",
        ),
        (r"<!>Unknown error", "Video %s does not exist."),
        (
            r"<!>Видео временно недоступно",
            "Video %s is temporarily unavailable.",
        ),
        (r"<!>Access denied", "Access denied to video %s."),
        (
            r"<!>Видеозапись недоступна, так как её автор был заблокирован.",
            BLOCKED,
        ),
        (
            r"<!>This video is no longer available, because its author has been blocked.",
            BLOCKED,
        ),
        (
            r"<!>This video is no longer available, because it has been deleted.",
            "Video %s is no longer available, because it has been deleted.",
        ),
        (
            r"<!>The video .+? is not available in your region.",
            "Video %s is not available in your region.",
        ),
    ]
    .into_iter()
    .map(|(pattern, message)| (Regex::new(pattern).unwrap(), message))
    .collect()
});
static PLAYER_PARAMS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)var\s+playerParams\s*=\s*(\{.+?\})\s*;\s*\n").unwrap());
/// Embedded players upstream hands to extractors this port lacks.
static OTHER_EMBEDS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (
            r#"<iframe[^>]+?src=["'](?:https?:)?//player\.vimeo\.com/video/\d+"#,
            "Vimeo",
        ),
        (
            r#"<embed[^>]+?src=["'](?:https?:)?//(?:www\.)?vimeo\.com/moogaloop\.swf"#,
            "Vimeo",
        ),
        (
            r#"<video[^>]+src=["'](?:https?:)?//(?:www\.)?vimeo\.com/[0-9]+["']"#,
            "Vimeo",
        ),
        (
            r#"\ssrc="(?:https?:)?//rutube\.ru\\?/(?:video|play)\\?/embed"#,
            "Rutube",
        ),
        (
            r#"(?i)<(?:(?:embed|iframe)[^>]+?src=|input[^>]+id=['"]dmcloudUrlEmissionSelect['"][^>]+value=)["'](?:https?:)?//(?:(?:(?:www|touch|geo)\.)?dailymotion\.[a-z]{2,3}|dai\.ly|(?:www\.)?dailymotion\.com/(?:embed/video|player\.html\?video=))"#,
            "Dailymotion",
        ),
        (
            r#"(?s)DM\.player\([^,]+,\s*\{.*?video['"]?\s*:\s*["']?[0-9a-zA-Z]+.+?\}\s*\);"#,
            "Dailymotion",
        ),
        (
            r#"<iframe[^>]+src=["'](?:https?:)?//(?:odnoklassniki|ok)\.ru/videoembed/"#,
            "Odnoklassniki",
        ),
        (
            r#"<iframe\b[^>]+\bsrc=["'](?:https?:)?//video\.sibnet\.ru/shell\.php\?.*?\bvideoid=\d+"#,
            "SibnetEmbed",
        ),
    ]
    .into_iter()
    .map(|(pattern, name)| (Regex::new(pattern).unwrap(), name))
    .collect()
});
static OPTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)var\s+opts\s*=\s*(\{.+?\});").unwrap());
static OPTS_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"url\s*:\s*'([^']+)").unwrap());
static DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class=["']mv_info_date[^>]+>([^<]+)(?:<|from)"#).unwrap());
static VIEWS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class=["']mv_views_count[^>]+>\s*([\d,.]+)"#).unwrap());
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:url|cache)(\d+)").unwrap());
static WALL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:(?:(?:m|new)\.)?vk\.com/(?:[^?]+\?.*\bw=)?wall(?P<id>-?\d+_\d+)))")
        .unwrap()
});
/// `VKUserVideosIE._VALID_URL`; the second pattern's `(?!\?.*\bz=video)`
/// is checked on `rest` (see [`user_videos_id`]).
static USER_VIDEOS_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    const BASE: &str = r"^https?://(?:(?:m|new)\.)?vk(?:video\.ru|\.com/video)";
    [
        Regex::new(&format!(r"{BASE}/playlist/(?P<id>-?\d+_-?\d+)")).unwrap(),
        Regex::new(&format!(
            r"{BASE}/(?P<id>@[^/?#]+)(?:/all)?/?(?P<rest>(?:[?#].*)?)$"
        ))
        .unwrap(),
    ]
});
static Z_VIDEO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\?.*\bz=video").unwrap());
static NEW_CUR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bvar newCur\s*=").unwrap());
static DATA_AUDIO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-audio="([^"]+)"#).unwrap());
static POST_VIDEO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+href=(?:["'])(/video(?:-?[\d_]+)[^"']*)"#).unwrap());

/// `VKIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `VKWallPostIE.suitable`.
pub fn wall_post_suitable(url: &str) -> bool {
    WALL_URL.is_match(url)
}

/// `VKUserVideosIE.suitable`.
pub fn user_videos_suitable(url: &str) -> bool {
    user_videos_id(url).is_some()
}

/// The `id` group of the first matching `VKUserVideosIE._VALID_URL`.
fn user_videos_id(url: &str) -> Option<String> {
    let [playlist, handle] = &*USER_VIDEOS_URLS;
    if let Some(found) = playlist.captures(url) {
        return Some(found["id"].to_owned());
    }
    let found = handle.captures(url)?;
    (!Z_VIDEO.is_match(&found["rest"])).then(|| found["id"].to_owned())
}

/// An error with upstream's temp ID (`None` for video/clip page URLs).
fn fail(id: Option<&str>, message: &str) -> Error {
    fail_as(NAME, id, message)
}

/// An `ExtractorError` of the extractor `name` with its temp ID.
fn fail_as(name: &str, id: Option<&str>, message: &str) -> Error {
    Error::Reported(match id {
        Some(id) => format!("[{name}] {id}: {message}"),
        None => format!("[{name}] {message}"),
    })
}

/// An unexpected exception (`KeyError` and the like) wrapped by `extract`.
fn bug_as(name: &str, id: Option<&str>, cause: &str) -> Error {
    fail_as(
        name,
        id,
        &format!(
            "An extractor error has occurred. (caused by {cause}); {}",
            crate::BUG_REPORTS_MESSAGE
        ),
    )
}

/// `extractor` as the `'static` name [`Error::ExtractorRequest`] needs.
fn static_name(name: &str) -> &'static str {
    match name {
        WALL_POST => WALL_POST,
        USER_VIDEOS => USER_VIDEOS,
        _ => NAME,
    }
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn unescaped(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(crate::html::unescape_html)
}

fn cleaned(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(crate::html::clean_html)
        .filter(|text| !text.is_empty())
}

/// `VKBaseIE._download_webpage_handle`'s JS challenge: for a response that
/// ended on `/challenge.html`, the page's salt function runs in the native
/// [`crate::jsinterp`] interpreter and `key=md5(HASH429:SALT)` is submitted
/// (which sets the `solution429` cookie). `true` when the request is to be
/// made again.
fn solve_challenge(
    client: &Client,
    name: &str,
    temp: Option<&str>,
    final_url: &Url,
    webpage: &str,
    video_id: &str,
) -> Result<bool> {
    static SALT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)var\s+salt\s*=\s*\(\s*function\s*\(\)\s*(\{.+?\})\s*\)\(\);\s*var\s+hash")
            .unwrap()
    });
    if final_url.path() != "/challenge.html" {
        return Ok(false);
    }
    crate::screen::screen(&if video_id.is_empty() {
        format!("[{name}] Received a JS challenge response")
    } else {
        format!("[{name}] [{video_id}] Received a JS challenge response")
    });
    let bug = |message: &str| {
        fail_as(
            name,
            temp,
            &format!("{message}; {}", crate::BUG_REPORTS_MESSAGE),
        )
    };
    // `parse_qs` (blank values dropped), last `hash429`.
    let query = parse_qs(final_url);
    let Some(hash) = query
        .iter()
        .find(|(key, _)| key == "hash429")
        .and_then(|(_, values)| values.last())
    else {
        return Err(bug("Unable to extract challenge hash"));
    };
    let Some(found) = SALT.captures(webpage) else {
        return Err(bug("Unable to extract JS challenge salt function"));
    };
    let jsi = crate::jsinterp::JsInterpreter::new(format!("function salt() {}", &found[1]));
    let salt = jsi
        .extract_function("salt")
        .and_then(|salt| jsi.call(&salt, Vec::new()))
        .map_err(|error| match error {
            crate::jsinterp::JsError::Python {
                kind: "KeyError", ..
            } => bug_as(name, temp, &error.repr()),
            error => fail_as(name, temp, &error.to_string()),
        })?;
    let salt = crate::jsinterp::py_str(&salt);
    crate::screen::debug(&format!(
        "[{name}] Generated salt with native JS interpreter: {salt}"
    ));
    let key_hash = crate::md5::hex(format!("{hash}:{salt}").as_bytes());
    crate::screen::debug(&format!("[{name}] JS challenge key hash: {key_hash}"));
    // `update_url_query(challenge_url, {'key': key_hash})`.
    let mut query = query;
    match query.iter_mut().find(|(key, _)| key == "key") {
        Some((_, values)) => *values = vec![key_hash],
        None => query.push(("key".into(), vec![key_hash])),
    }
    let mut submit = final_url.clone();
    submit.set_query(None);
    {
        let mut pairs = submit.query_pairs_mut();
        for (key, values) in &query {
            for value in values {
                pairs.append_pair(key, value);
            }
        }
    }
    // Request with the challenge key and the response should set a
    // 'solution429' cookie.
    crate::screen::screen(&format!(
        "[{name}] {video_id}: Submitting JS challenge solution"
    ));
    super::direct::request(client, &submit, None)
        .map_err(|error| {
            Error::Reported(super::request_errnote_text(
                name,
                video_id,
                "Unable to solve JS challenge",
                &error,
            ))
        })
        .map(drop)?;
    Ok(true)
}

/// `urllib.parse.parse_qs(url.query)`: values grouped by key, blank values
/// dropped.
fn parse_qs(url: &Url) -> Vec<(String, Vec<String>)> {
    let mut query: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in url.query_pairs() {
        if value.is_empty() {
            continue;
        }
        match query.iter_mut().find(|(other, _)| *other == key) {
            Some((_, values)) => values.push(value.into_owned()),
            None => query.push((key.into_owned(), vec![value.into_owned()])),
        }
    }
    query
}

/// `_download_webpage_handle(url, video_id)` through the JS challenge.
fn download_webpage_handle(
    client: &Client,
    page: &Url,
    name: &'static str,
    temp: Option<&str>,
    video_id: &str,
) -> Result<(String, Url)> {
    let (webpage, final_url) = super::download_webpage_handle(client, page, name, video_id)?;
    if solve_challenge(client, name, temp, &final_url, &webpage, video_id)? {
        return super::download_webpage_handle(client, page, name, video_id);
    }
    Ok((webpage, final_url))
}

/// `VKBaseIE._download_payload(path, video_id, data)`: a POST of `data` and
/// `al=1` to `https://vk.com/PATH.php`, then its `payload` pair's payload.
/// `temp` is the extractor's temp ID for its errors.
fn download_payload(
    client: &Client,
    name: &str,
    temp: Option<&str>,
    path: &str,
    video_id: &str,
    data: &[(&str, &str)],
) -> Result<Value> {
    use std::io::Read;
    let endpoint = Url::parse(&format!("https://vk.com/{path}.php"))?;
    let mut body = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in data {
        body.append_pair(key, value);
    }
    body.append_pair("al", "1");
    let body = body.finish();
    let extractor = static_name(name);
    let fetch = || -> Result<(String, Url)> {
        crate::screen::screen(&format!("[{name}] {video_id}: Downloading JSON metadata"));
        let mut response = super::direct::post(
            client,
            &endpoint,
            &[
                ("Content-Type", "application/x-www-form-urlencoded"),
                ("Referer", endpoint.as_str()),
                ("X-Requested-With", "XMLHttpRequest"),
            ],
            body.clone(),
        )
        .map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor,
                id: video_id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            },
            error => error,
        })?;
        let mut bytes = Vec::new();
        response
            .read_to_end(&mut bytes)
            .map_err(Error::ExtractionRead)?;
        let text = super::decode_webpage(&bytes, response.headers());
        Ok((text, response.url().clone()))
    };
    let (mut text, final_url) = fetch()?;
    if solve_challenge(client, name, temp, &final_url, &text, video_id)? {
        text = fetch()?.0;
    }
    let json: Value = crate::json::from_str(&text).map_err(|error| {
        fail_as(
            name,
            Some(video_id),
            &format!("Failed to parse JSON  (caused by {error})"),
        )
    })?;
    let Some(pair) = json.get("payload") else {
        return Err(bug_as(name, temp, "KeyError('payload')"));
    };
    let Some([code, payload]) = pair.as_array().map(Vec::as_slice) else {
        return Err(bug_as(
            name,
            temp,
            "ValueError('not enough values to unpack')",
        ));
    };
    match code.as_str() {
        Some("3") => {
            return Err(fail_as(
                name,
                temp,
                &format!("This video is only available for registered users. {LOGIN_HINT}"),
            ));
        }
        Some("8") => {
            let text = payload.get(0).and_then(Value::as_str).unwrap_or_default();
            let mut chars = text.chars();
            chars.next();
            chars.next_back();
            return Err(fail_as(
                name,
                temp,
                &crate::html::clean_html(chars.as_str()),
            ));
        }
        _ => {}
    }
    Ok(payload.clone())
}

/// `url_result(url, VKIE, video_id)`: a playlist entry resolved when
/// processed.
fn vk_entry(target: &str, id: Option<&str>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), KEY.into());
    if let Some(id) = id {
        info.id = id.to_owned();
        entry.insert("id".into(), id.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// Upstream `get_element_by_class(class_name, html)`: the first element's
/// unescaped content.
fn element_by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(text, _)| text)
}

/// `initialize()` (logging in once per extractor) and the `Extracting URL`
/// line.
fn start(
    extractor: &dyn Extractor,
    client: &Client,
    (name, temp): (&'static str, Option<&str>),
    page: &Url,
) -> Result<()> {
    crate::screen::set_extractor(name);
    extractor.initialize(client, name, temp.unwrap_or_default())?;
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(page.as_str())
    ));
    Ok(())
}

/// `str.encode('cp1251')` for the login form; characters outside the code
/// page are upstream's `UnicodeEncodeError`.
fn cp1251(text: &str) -> Result<Vec<u8>> {
    let (bytes, _, unmappable) = encoding_rs::WINDOWS_1251.encode(text);
    if unmappable {
        return Err(Error::Reported(format!(
            "[{NAME}] the credentials are not encodable as cp1251"
        )));
    }
    Ok(bytes.into_owned())
}

/// `VKBaseIE._perform_login`: the home page's hidden inputs with the
/// cp1251 `email`/`pass`, its first `remixlhk` cookie, and the login POST
/// (`onLoginFailed` is a wrong username or password).
fn perform_login(
    client: &Client,
    session: &super::login::Session,
    username: &str,
    password: &str,
) -> Result<()> {
    // `_download_webpage_handle` through the JS challenge: a solved
    // challenge repeats the request.
    let request = |url: &str, note: &'static str, body: Option<String>| -> Result<_> {
        let url = Url::parse(url)?;
        let page = session.request(
            client,
            &url,
            (Some(note), "Unable to download webpage"),
            &[],
            body.clone(),
            &[],
        )?;
        if solve_challenge(client, session.ie, None, &page.url, &page.text, "")? {
            return session.request(
                client,
                &url,
                (Some(note), "Unable to download webpage"),
                &[],
                body,
                &[],
            );
        }
        Ok(page)
    };
    let login_page = request("https://vk.com", "Downloading login page", None)?;
    let mut form: Vec<(String, Vec<u8>)> = super::login::hidden_inputs(&login_page.text)
        .into_iter()
        .map(|(key, value)| (key, value.into_bytes()))
        .collect();
    for (key, value) in [("email", cp1251(username)?), ("pass", cp1251(password)?)] {
        match form.iter_mut().find(|(name, _)| name == key) {
            Some(entry) => entry.1 = value,
            None => form.push((key.to_owned(), value)),
        }
    }
    // `_apply_first_set_cookie_header(url_handle, 'remixlhk')`.
    static REMIXLHK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"remixlhk=(.+?);.*?\b[Dd]omain=(.+?)(?:[,;]|$)").unwrap());
    for header in login_page.headers.get_all(reqwest::header::SET_COOKIE) {
        let header = String::from_utf8_lossy(header.as_bytes());
        if let Some(found) = REMIXLHK.captures(&header) {
            crate::cookies::set_cookie(&found[2], "remixlhk", &found[1]);
            break;
        }
    }
    let body = form
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                super::login::quote_plus(key.as_bytes()),
                super::login::quote_plus(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let page = request("https://vk.com/login", "Logging in", Some(body))?;
    if page.text.contains("onLoginFailed") {
        return Err(session.error("Unable to login, incorrect username and/or password"));
    }
    Ok(())
}

/// `VKBaseIE`'s login hooks.
macro_rules! vk_login {
    () => {
        fn netrc_machine(&self) -> Option<&'static str> {
            Some("vk")
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

/// An extractor's info dict (`dict`, in upstream's key order) as an info.
#[allow(clippy::too_many_arguments)]
fn video_info(
    page: &Url,
    (extractor, extractor_key): (&str, &str),
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
    format_sort_fields: &[&str],
    parts: &str,
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
    info.subtitles = subtitles;
    info.webpage_url = page.clone();
    info.extractor = extractor.into();
    info.extractor_key = extractor_key.into();
    info.format_sort_fields = format_sort_fields.iter().map(|&f| f.into()).collect();
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
            "formats" | "subtitles" => {}
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    if !format_sort_fields.is_empty() {
        keys.push("_format_sort_fields".into());
    }
    crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{}{parts}",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    )
}

/// Python's `str()` of a JSON scalar (`f'{value}'`).
fn py_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(number.to_string()),
        _ => None,
    }
}

impl Extractor for VkWallPost {
    vk_login!();

    fn key(&self) -> &'static str {
        WALL_POST
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    /// `VKWallPostIE._real_extract`.
    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = WALL_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let post_id = found["id"].to_owned();
        let temp = Some(post_id.as_str());
        start(self, client, (WALL_POST, temp), page)?;
        let wall = format!("wall{post_id}");
        let payload = download_payload(
            client,
            WALL_POST,
            temp,
            "wkview",
            &post_id,
            &[("act", "show"), ("w", &wall)],
        )?;
        let webpage = match payload.get(1) {
            Some(Value::String(webpage)) => webpage.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "VK wall posts whose payload is not HTML are not supported natively".into(),
                ));
            }
            None => {
                return Err(bug_as(
                    WALL_POST,
                    temp,
                    "IndexError('list index out of range')",
                ));
            }
        };
        let uploader = element_by_class("PostHeaderTitle__authorName", &webpage)
            .map(|text| crate::html::clean_html(&text));

        let mut entries = Vec::new();
        for found in DATA_AUDIO.captures_iter(&webpage) {
            let audio: Value = serde_json::from_str(&crate::html::unescape_html(&found[1]))
                .map_err(|error| {
                    fail_as(
                        WALL_POST,
                        Some(&post_id),
                        &format!("Failed to parse JSON  (caused by {error})"),
                    )
                })?;
            let Some(audio) = audio.as_object() else {
                return Err(Error::UnsupportedHtml5(
                    "VK wall post audio data that is not an object is not supported natively"
                        .into(),
                ));
            };
            let Some(url) = audio.get("url") else {
                return Err(bug_as(WALL_POST, temp, "KeyError('url')"));
            };
            if !truthy(url) {
                continue;
            }
            let Some(url) = url.as_str() else {
                return Err(Error::UnsupportedHtml5(
                    "VK wall post audio URLs that are not strings are not supported natively"
                        .into(),
                ));
            };
            let title = unescaped(audio.get("title"));
            let artist = unescaped(audio.get("artist"));
            let mut id = Vec::new();
            for key in ["owner_id", "id"] {
                let Some(value) = audio.get(key) else {
                    return Err(bug_as(WALL_POST, temp, &format!("KeyError('{key}')")));
                };
                id.push(py_scalar(value).ok_or_else(|| {
                    Error::UnsupportedHtml5(format!(
                        "VK wall post audio {key}s that are not strings or integers are not supported natively"
                    ))
                })?);
            }
            let joined: Vec<&str> = [artist.as_deref(), title.as_deref()]
                .into_iter()
                .flatten()
                .filter(|part| !part.is_empty())
                .collect();
            let thumbnails = audio
                .get("coverUrl")
                .and_then(Value::as_str)
                .map(|urls| Value::Array(urls.split(',').map(|url| json!({"url": url})).collect()))
                .unwrap_or(Value::Null);
            let mut dict = Map::new();
            dict.insert("id".into(), id.join("_").into());
            dict.insert("title".into(), joined.join(" - ").into());
            dict.insert("thumbnails".into(), thumbnails);
            dict.insert("duration".into(), int(audio.get("duration")).into());
            dict.insert("uploader".into(), uploader.clone().into());
            dict.insert("artist".into(), artist.into());
            dict.insert("track".into(), title.into());
            dict.insert("formats".into(), json!([]));
            let mut format = Map::new();
            format.insert("url".into(), url.into());
            format.insert("ext".into(), "m4a".into());
            format.insert("vcodec".into(), "none".into());
            format.insert("acodec".into(), "mp3".into());
            format.insert("container".into(), "m4a_dash".into());
            let format = format_from(format, &post_id)?;
            entries.push(video_info(
                page,
                (WALL_POST, "VKWallPost"),
                dict,
                vec![format],
                crate::Subtitles::default(),
                &[],
                ";thumbnail=url",
            ));
        }

        let body = crate::html_elements::element_html_by_id("wl_post_body", &webpage)
            .map_err(Error::Reported)?
            .ok_or_else(|| {
                // `re.findall` over `None`, which `extract` does not wrap.
                Error::Reported("expected string or bytes-like object, got 'NoneType'".into())
            })?;
        // Upstream deduplicates with a `set` (in hash order); the links stay
        // in page order here.
        let mut links: Vec<String> = Vec::new();
        for found in POST_VIDEO.captures_iter(&body) {
            let link = found[1].to_owned();
            if !links.contains(&link) {
                links.push(link);
            }
        }
        for link in links {
            entries.push(vk_entry(page.join(&link)?.as_str(), None)?);
        }

        let title = match uploader.as_deref().filter(|name| !name.is_empty()) {
            Some(uploader) => format!("{uploader} - Wall post {post_id}"),
            None => format!("Wall post {post_id}"),
        };
        let description =
            element_by_class("wall_post_text", &webpage).map(|text| crate::html::clean_html(&text));
        let mut keys = vec!["id", "title"];
        let mut extra = Map::new();
        if let Some(description) = description {
            keys.push("description");
            extra.insert("description".into(), description.into());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: post_id,
            title,
            webpage_url: page.clone(),
            extractor: WALL_POST.into(),
            extractor_key: "VKWallPost".into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

impl Extractor for VkUserVideos {
    vk_login!();

    fn key(&self) -> &'static str {
        USER_VIDEOS
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    /// `VKUserVideosIE._real_extract`. Upstream's lazy `_entries` pages are
    /// all fetched here.
    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(u_id) = user_videos_id(page.as_str()) else {
            return Ok(None);
        };
        let temp = Some(u_id.as_str());
        start(self, client, (USER_VIDEOS, temp), page)?;
        let (webpage, _) = download_webpage_handle(client, page, USER_VIDEOS, temp, &u_id)?;
        let (page_id, section) = if u_id.starts_with('@') {
            let cursor = match super::search_json(&NEW_CUR, &webpage, false) {
                Some(Some(cursor)) => cursor,
                Some(None) => {
                    return Err(Error::UnsupportedHtml5(
                        "VK cursor data that does not parse is not supported natively".into(),
                    ));
                }
                None => {
                    return Err(fail_as(
                        USER_VIDEOS,
                        temp,
                        &format!(
                            "Unable to extract cursor data; {}",
                            crate::BUG_REPORTS_MESSAGE
                        ),
                    ));
                }
            };
            let page_id = match cursor.get("oid") {
                Some(Value::Number(oid)) if oid.is_i64() || oid.is_u64() => oid.to_string(),
                _ => {
                    return Err(fail_as(
                        USER_VIDEOS,
                        temp,
                        &format!("Unable to extract page id; {}", crate::BUG_REPORTS_MESSAGE),
                    ));
                }
            };
            let section = page
                .query_pairs()
                .find(|(key, value)| key == "section" && !value.is_empty())
                .map_or_else(|| "all".to_owned(), |(_, value)| value.into_owned());
            (page_id, section)
        } else {
            let (page_id, section) = u_id.split_once('_').unwrap_or((&u_id, ""));
            (page_id.to_owned(), format!("playlist_{section}"))
        };
        let title = element_by_class("VideoInfoPanel__title", &webpage)
            .map(|text| crate::html::clean_html(&text))
            .filter(|title| !title.is_empty());

        let load = |offset: i64| -> Result<(i64, i64, Vec<Value>)> {
            let offset = offset.to_string();
            let payload = download_payload(
                client,
                USER_VIDEOS,
                temp,
                "al_video",
                &page_id,
                &[
                    ("act", "load_videos_silent"),
                    ("offset", &offset),
                    ("oid", &page_id),
                    ("section", &section),
                ],
            )?;
            let Some(first) = payload.get(0) else {
                return Err(bug_as(
                    USER_VIDEOS,
                    temp,
                    "IndexError('list index out of range')",
                ));
            };
            let Some(list_json) = first.get(&section) else {
                return Err(bug_as(USER_VIDEOS, temp, &format!("KeyError('{section}')")));
            };
            let field = |key: &str| {
                list_json
                    .get(key)
                    .ok_or_else(|| bug_as(USER_VIDEOS, temp, &format!("KeyError('{key}')")))
            };
            let count = field("count")?;
            let total = field("total")?;
            let list = field("list")?;
            let number = |value: &Value| {
                value.as_i64().ok_or_else(|| {
                    Error::UnsupportedHtml5(
                        "VK video list counts that are not integers are not supported natively"
                            .into(),
                    )
                })
            };
            let count = number(count)?;
            let list = list.as_array().cloned().ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "VK video lists that are not arrays are not supported natively".into(),
                )
            })?;
            Ok((count, number(total)?, list))
        };
        let (mut count, total, mut list) = load(0)?;
        let mut entries = Vec::new();
        loop {
            for video in &list {
                let pair = video
                    .as_array()
                    .filter(|video| video.len() >= 2)
                    .and_then(|video| Some((video[0].as_i64()?, video[1].as_i64()?)));
                let Some((owner_id, id)) = pair else {
                    return Err(Error::UnsupportedHtml5(
                        "VK video list items without integer owner and video IDs are not supported natively"
                            .into(),
                    ));
                };
                let video_id = format!("{owner_id}_{id}");
                entries.push(vk_entry(
                    &format!("https://vk.com/video{video_id}"),
                    Some(&video_id),
                )?);
            }
            if count >= total {
                break;
            }
            let (new_count, _, next) = load(count)?;
            if new_count == 0 {
                crate::screen::screen(&format!(
                    "[{USER_VIDEOS}] {page_id}: Skipping {} unavailable videos",
                    total - count
                ));
                break;
            }
            count += new_count;
            list = next;
        }

        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: format!("{page_id}_{section}"),
            title: title.unwrap_or_default(),
            webpage_url: page.clone(),
            extractor: USER_VIDEOS.into(),
            extractor_key: "VKUserVideos".into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

fn url_follow(page: &Url, target: &Url) -> ExtractResult {
    let mut follow = super::direct::build_at(page.clone(), page, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow
        .extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    ExtractResult::Video(follow)
}

fn format_from(mut format: Map<String, Value>, id: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = format.keys().cloned().collect();
    format.retain(|_, value| !value.is_null());
    if !format.contains_key("format_id") {
        format.insert("format_id".into(), "".into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(Some(id), &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    Ok(format)
}

impl Extractor for Vk {
    vk_login!();

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
        // `get_temp_id`: the embed's `id` group, `None` for pages.
        let temp_id = found.name("id").map(|id| id.as_str().to_owned());
        let temp = temp_id.as_deref();
        start(self, client, (NAME, temp), page)?;
        let mut mv_data = json!({});
        let (video_id, info_page, player) = if let Some(video_id) = found.name("videoid") {
            let video_id = video_id.as_str().to_owned();
            let mut data = vec![("act", "show"), ("video", video_id.as_str())];
            if let Some(list_id) = found.name("list_id") {
                data.push(("list", list_id.as_str()));
            }
            let payload = download_payload(client, NAME, temp, "al_video", &video_id, &data)?;
            let items = payload.as_array().cloned().unwrap_or_default();
            let info_page = items
                .get(1)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let opts = items.last().cloned().unwrap_or_default();
            if let Some(data) = opts.get("mvData").filter(|data| truthy(data)) {
                mv_data = data.clone();
            }
            let player = opts
                .get("player")
                .filter(|player| truthy(player))
                .cloned()
                .unwrap_or_else(|| json!({}));
            (video_id, info_page, player)
        } else {
            let video_id = format!("{}_{}", &found["oid"], &found["id"]);
            let embed = Url::parse(&format!(
                "https://vk.com/video_ext.php?{}",
                &found["embed_query"]
            ))?;
            let (info_page, _) = download_webpage_handle(client, &embed, NAME, temp, &video_id)?;
            if let Some(message) = ERROR_MESSAGES
                .iter()
                .find_map(|pattern| pattern.captures(&info_page))
                .map(|found| crate::html::clean_html(&found[1]))
                .filter(|message| !message.is_empty())
            {
                return Err(fail(temp, &message));
            }
            if SECURITY_CHECK.is_match(&info_page) {
                return Err(fail(
                    temp,
                    "You are trying to log in from an unusual location. You should confirm ownership at vk.com to log in with this IP.",
                ));
            }
            if let Some((_, message)) = ERRORS
                .iter()
                .find(|(pattern, _)| pattern.is_match(&info_page))
            {
                return Err(fail(temp, &message.replace("%s", &video_id)));
            }
            let Some(params) = PLAYER_PARAMS.captures(&info_page) else {
                return Err(fail(
                    temp,
                    &format!(
                        "Unable to extract player params; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ),
                ));
            };
            let player: Value = crate::json::from_str(&params[1]).map_err(|error| {
                fail(
                    Some(&video_id),
                    &format!("Failed to parse JSON  (caused by {error})"),
                )
            })?;
            (video_id, info_page, player)
        };

        // Embedded players: YouTube is followed, others are unported.
        if let Some(youtube) = super::youtube::embed_urls(page, &info_page)
            .into_iter()
            .next()
        {
            return Ok(Some(url_follow(page, &youtube)));
        }
        if let Some((_, name)) = OTHER_EMBEDS
            .iter()
            .find(|(pattern, _)| pattern.is_match(&info_page))
        {
            return Err(Error::UnsupportedHtml5(format!(
                "VK videos embedding a {name} player are not supported natively"
            )));
        }
        if let Some(opts) = OPTS.captures(&info_page)
            && let Some(target) = OPTS_URL.captures_iter(&opts[1]).find_map(|found| {
                // `(?!/\b)`: not a site-relative path.
                let url = found[1].to_owned();
                let mut chars = url.chars();
                let relative = chars.next() == Some('/')
                    && chars
                        .next()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_');
                (!relative).then_some(url)
            })
        {
            let target = match target.strip_prefix("//") {
                Some(rest) => format!("https://{rest}"),
                None => target,
            };
            return Ok(Some(url_follow(page, &Url::parse(&target)?)));
        }

        let Some(data) = player
            .get("params")
            .and_then(|params| params.get(0))
            .cloned()
        else {
            return Err(fail(
                temp,
                &format!(
                    "An extractor error has occurred. (caused by KeyError('params')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        };
        let is_live = int(data.get("live")) == Some(2);
        let timestamp = DATE
            .captures(&info_page)
            .map(|found| crate::html::clean_html(&found[1]))
            .and_then(|date| crate::dates::unified_timestamp(&date, true))
            .or_else(|| int(data.get("date")));
        let view_count = VIEWS
            .captures(&info_page)
            .and_then(|found| found[1].replace([',', '.', '+'], "").parse::<i64>().ok());

        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        for (format_id, value) in data.as_object().into_iter().flatten() {
            let Some(format_url) = value.as_str().and_then(super::json_ld::url_or_none) else {
                continue;
            };
            if !["http", "//", "rtmp"]
                .iter()
                .any(|prefix| format_url.starts_with(prefix))
            {
                continue;
            }
            let absolute = super::proto_relative(&format_url);
            if format_id.starts_with("url")
                || format_id.starts_with("cache")
                || ["extra_data", "live_mp4", "postlive_mp4"].contains(&format_id.as_str())
            {
                let mut format = Map::new();
                format.insert("format_id".into(), format_id.clone().into());
                format.insert("url".into(), absolute.into());
                format.insert("ext".into(), "mp4".into());
                format.insert("source_preference".into(), 1.into());
                format.insert(
                    "height".into(),
                    HEIGHT
                        .captures(format_id)
                        .and_then(|found| found[1].parse::<i64>().ok())
                        .into(),
                );
                formats.push(format_from(format, &video_id)?);
            } else if format_id.starts_with("hls") && format_id != "hls_live_playback" {
                if let Some(stream) = Url::parse(&absolute).ok().and_then(|manifest| {
                    super::extract_m3u8_optional_note(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some(format_id),
                        true,
                        is_live,
                        "Downloading m3u8 information",
                    )
                }) {
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            } else if format_id.starts_with("dash")
                && !["dash_live_playback", "dash_uni"].contains(&format_id.as_str())
            {
                if let Some(stream) = Url::parse(&absolute).ok().and_then(|manifest| {
                    super::extract_mpd_optional_note(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some(format_id),
                        "Downloading MPD manifest",
                    )
                }) {
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            } else if format_id == "rtmp" {
                let mut format = Map::new();
                format.insert("format_id".into(), format_id.clone().into());
                format.insert("url".into(), absolute.into());
                format.insert("ext".into(), "flv".into());
                formats.push(format_from(format, &video_id)?);
            }
        }
        for sub in data
            .get("subs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(url) = sub
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .and_then(|url| Url::parse(&super::proto_relative(&url)).ok())
            else {
                continue;
            };
            let ext = sub
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(".srt")
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_owned();
            let language = sub
                .get("lang")
                .and_then(Value::as_str)
                .unwrap_or("en")
                .to_owned();
            let mut track = Map::new();
            track.insert("ext".into(), ext.into());
            track.insert("url".into(), url.as_str().into());
            let track: crate::SubtitleFormat = serde_json::from_value(Value::Object(track))
                .map_err(|error| fail(Some(&video_id), &error.to_string()))?;
            let mut single = crate::Subtitles::default();
            single.entries.push((Some(language), vec![track]));
            super::html5::merge_subtitles(&mut subtitles, single);
        }

        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("formats".into(), json!([]));
        dict.insert("subtitles".into(), json!({}));
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                dict.insert(key.into(), value);
            }
        };
        put("title", unescaped(mv_data.get("title")).map(Value::from));
        put("description", cleaned(mv_data.get("desc")).map(Value::from));
        put("duration", int(mv_data.get("duration")).map(Value::from));
        put("like_count", int(mv_data.get("likes")).map(Value::from));
        put(
            "comment_count",
            int(mv_data.get("commcount")).map(Value::from),
        );
        put("title", unescaped(data.get("md_title")).map(Value::from));
        put(
            "description",
            cleaned(data.get("description")).map(Value::from),
        );
        put(
            "thumbnail",
            data.get("jpg")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(Value::from),
        );
        put(
            "uploader",
            unescaped(data.get("md_author")).map(Value::from),
        );
        put(
            "uploader_id",
            ["author_id", "authorId"]
                .iter()
                .find_map(|key| match data.get(*key)? {
                    Value::String(text) => Some(Value::from(text.clone())),
                    Value::Number(number) => Some(Value::from(number.to_string())),
                    _ => None,
                }),
        );
        put("duration", int(data.get("duration")).map(Value::from));
        let chapters: Vec<Value> = data
            .get("time_codes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|code| code.get("time").is_some_and(Value::is_i64))
            .map(|code| {
                let mut chapter = Map::new();
                if let Some(title) = unescaped(code.get("text")) {
                    chapter.insert("title".into(), title.into());
                }
                chapter.insert("start_time".into(), code["time"].clone());
                Value::Object(chapter)
            })
            .collect();
        if !chapters.is_empty() {
            dict.insert("chapters".into(), Value::Array(chapters));
        }
        dict.insert("timestamp".into(), timestamp.into());
        dict.insert("view_count".into(), view_count.into());
        dict.insert("is_live".into(), is_live.into());

        Ok(Some(ExtractResult::Video(video_info(
            page,
            (NAME, KEY),
            dict,
            formats,
            subtitles,
            &["res", "source"],
            ";subtitle=ext,url;chapter=title,start_time",
        ))))
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        let video = VALID_URL
            .captures("https://vk.com/videos-77521?z=video-77521_162222515%2Fclub77521")
            .unwrap();
        assert_eq!(&video["videoid"], "-77521_162222515");
        let embed = VALID_URL
            .captures("https://vk.com/video_ext.php?oid=-77521&id=162222515&hash=87b046504ccd8bfa")
            .unwrap();
        assert_eq!((&embed["oid"], &embed["id"]), ("-77521", "162222515"));
        let listed = VALID_URL
            .captures("https://vk.com/video-93049196_456239755?list=ln-cBjJ7S4jYYx3ADnmDT")
            .unwrap();
        assert_eq!(&listed["list_id"], "ln-cBjJ7S4jYYx3ADnmDT");
        assert!(suitable("https://vkvideo.ru/clip-220265624_456239031"));
        assert!(!suitable("https://vk.com/videos-77521"));
    }

    #[test]
    fn wall_post_and_user_video_urls_follow_upstream() {
        assert!(wall_post_suitable(
            "https://vk.com/bs.official?w=wall-23538238_35"
        ));
        assert!(wall_post_suitable("https://m.vk.com/wall-23538238_35"));
        assert!(!wall_post_suitable("https://vkvideo.ru/wall-1_2"));
        for (url, id) in [
            ("https://vk.com/video/@mobidevices", "@mobidevices"),
            (
                "https://vk.com/video/@mobidevices?section=uploaded",
                "@mobidevices",
            ),
            (
                "https://vk.com/video/@gorkyfilmstudio/all",
                "@gorkyfilmstudio",
            ),
            ("https://vkvideo.ru/@mobidevices", "@mobidevices"),
            ("https://vk.com/video/playlist/-174476437_2", "-174476437_2"),
            ("https://vkvideo.ru/playlist/-51890028_-2", "-51890028_-2"),
        ] {
            assert_eq!(user_videos_id(url).as_deref(), Some(id), "{url}");
        }
        assert!(!user_videos_suitable("https://vk.com/video/@x?z=video-1_2"));
        assert!(!user_videos_suitable("https://vk.com/video/@x/other"));
    }
}
