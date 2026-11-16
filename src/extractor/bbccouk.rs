//! Upstream `BBCCoUkIE` (`bbc.co.uk`, BBC iPlayer) and `BBCIE` (`bbc`) from
//! `yt_dlp/extractor/bbc.py`.
//!
//! Both resolve a programme's version ID through the BBC media selector
//! (`_download_media_selector`): each media set's JSON (403/404 answers are
//! read too), its error IDs (`notukerror`, `geolocation` and
//! `selectionunavailable` try the next set, then warn or fail), the video and
//! audio connections as non-fatal HLS, DASH and HDS manifests, ASX playlists,
//! direct HTTP and RTMP formats, and the captions connection as a TTML
//! subtitle (when subtitles are requested, after checking it parses).
//!
//! `BBCCoUkIE` reads the programme page (its error message, the
//! `mediator.bind` player or the page's `vpid`), else the programme's
//! `playlist.json` versions, else the legacy EMP playlist XML.
//! `BBCIE` tries upstream's page layouts in its order: `playlist.sxml`
//! parameters, `data-playable` settings, a `data-pid` video `div` (handed to
//! `BBCCoUkIE`), page `vpid`s with `digitalData`, BBC Reel's `initial-data`,
//! Morph payloads, `__PRELOADED_STATE__` programmes and article assets, BBC
//! Three's config, `__INITIAL_DATA__` articles and media experiences,
//! `SIMORGH_DATA`, Next.js pages, SMP embeds and `data-media-meta`/
//! `mediaAssetPage`/`vxp-playlist-data` media.
//!
//! Not supported natively: Python `TypeError`/`AttributeError` paths on
//! values of unexpected types (explicit native errors instead), and a Morph
//! lead media `duration`, which upstream's `traverse_obj` turns into a list.
use super::{
    Extractor, dict_format, dict_info, download_webpage, download_webpage_note, search_warning,
};
use crate::extractor::UrlRegex;
use crate::{
    Error, ExtractResult, PlaylistInfo, Result, SubtitleFormat, Subtitles, VideoFormat, VideoInfo,
};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BbcCoUk;
pub struct Bbc;

pub(crate) const COUK_NAME: &str = "bbc.co.uk";
pub(crate) const COUK_KEY: &str = "BBCCoUk";
pub(crate) const BBC_NAME: &str = "bbc";
pub(crate) const BBC_KEY: &str = "BBC";

/// `BBCCoUkIE._ID_REGEX`.
const ID_REGEX: &str = r"(?:[pbml][\da-z]{7}|w[\da-z]{7,14})";
const LOGIN_URL: &str = "https://account.bbc.com/signin";
/// `_MEDIA_SELECTOR_URL_TEMPL`.
const MEDIA_SELECTOR: &str =
    "https://open.live.bbc.co.uk/mediaselector/6/select/version/2.0/mediaset/";
/// `BBCCoUkIE._MEDIA_SETS`.
const COUK_MEDIA_SETS: &[&str] = &["iptv-all", "pc"];
/// `BBCIE._MEDIA_SETS`.
const BBC_MEDIA_SETS: &[&str] = &["pc", "mobile-tablet-main"];
/// `_EMP_PLAYLIST_NS`.
const EMP_NS: &str = "http://bbc.co.uk/2008/emp/playlist";
const RTMP_PLAYER: &str =
    "http://www.bbc.co.uk/emp/releases/iplayer/revisions/617463_618125_4/617463_618125_4_emp.swf";

static COUK_URL: LazyLock<UrlRegex> = LazyLock::new(|| {
    UrlRegex::new(format!(
        r"^https?://(?:www\.)?bbc\.co\.uk/(?:programmes/|iplayer(?:/[^/]+)?/(?:episode/|playlist/)|music/(?:clips|audiovideo/popular)[/#]|radio/player/|events/[^/]+/play/[^/]+/)(?P<id>{ID_REGEX})"
    ))
});
/// `(?!/(?:episodes|broadcasts|clips))` after the ID.
static COUK_EXCLUDED: LazyLock<UrlRegex> = LazyLock::new(|| {
    UrlRegex::new(format!(
        r"^https?://(?:www\.)?bbc\.co\.uk/programmes/articles/|^https?://(?:www\.)?bbc\.co\.uk/(?:programmes/|iplayer(?:/[^/]+)?/(?:episode/|playlist/)|music/(?:clips|audiovideo/popular)[/#]|radio/player/|events/[^/]+/play/[^/]+/){ID_REGEX}/(?:episodes|broadcasts|clips)"
    ))
});
static BBC_URL: LazyLock<UrlRegex> = LazyLock::new(|| {
    UrlRegex::new(
        r"^https?://(?:www\.)?(?:bbc\.(?:com|co\.uk)|bbcnewsd73hkzno2ini43t4gblxvycyac5aw4gnv7t2rccijh7745uqd\.onion|bbcweb3hytmzhn5d532owbu6oqadra5z3ar726vq5kgwwn6aucdccrad\.onion)/(?:[^/]+/)+(?P<id>[^/#?]+)",
    )
});
/// `_EMBED_REGEX`.
static EMBED: LazyLock<[Regex; 1]> =
    LazyLock::new(|| {
        [Regex::new(
            r#"setPlaylist\("(?P<url>https?://www\.bbc\.co\.uk/iplayer/[^/]+/[\da-z]{8})"\)"#,
        )
        .unwrap()]
    });

static PAGE_ERROR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div\b[^>]+\bclass=["'](?:smp|playout)__message delta["'][^>]*>\s*([^<]+?)\s*<"#)
        .unwrap()
});
static TVIPLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"mediator\.bind\((\{.+?\})\s*,\s*document\.getElementById").unwrap()
});
static VPID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r#""vpid"\s*:\s*"({ID_REGEX})""#)).unwrap());
static PARENT_TITLE: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<h2[^>]+id="parent-title"[^>]*>(.+?)</h2>"#).unwrap(),
        Regex::new(r#"<div[^>]+class="info"[^>]*>\s*<h1>(.+?)</h1>"#).unwrap(),
    ]
});
static COUK_DESCRIPTION: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<p class="[^"]*medium-description[^"]*">([^<]+)</p>"#).unwrap(),
        Regex::new(r#"<div[^>]+class="info_+synopsis"[^>]*>([^<]+)</div>"#).unwrap(),
    ]
});
static LEGACY_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[pb][\da-z]{7}\z").unwrap());
static FORM_ACTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<form[^>]+action=(?:"(.+?)"|'(.+?)')"#).unwrap());

/// `re.sub(r'(.+)\s*-\s*BBC.*?$', r'\1', title)`.
static TITLE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(.+)\s*-\s*BBC.*?\n?\z").unwrap());
static PUBLISHED: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r#"<meta[^>]+property="article:published_time"[^>]+content="([^"]+)""#).unwrap(),
        Regex::new(r#"itemprop="datePublished"[^>]+datetime="([^"]+)""#).unwrap(),
        Regex::new(r#""datePublished":\s*"([^"]+)"#).unwrap(),
    ]
});
static PARAM_PLAYLIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<param[^>]+name="playlist"[^>]+value="([^"]+)""#).unwrap());
static MEDIA_ID_SXML: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-media-id="([^"]+/playlist\.sxml)""#).unwrap());
/// `data-playable=(["\'])({.+?})\1`.
static DATA_PLAYABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-playable=(?:"(\{.+?\})"|'(\{.+?\})')"#).unwrap());
static GROUP_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"<div[^>]+\bclass=["']video["'][^>]+\bdata-pid=["']({ID_REGEX})"#
    ))
    .unwrap()
});
static PAGE_VPID: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(&format!(
            r#"data-(?:video-player|media)-vpid="({ID_REGEX})""#
        ))
        .unwrap(),
        Regex::new(&format!(
            r#"<param[^>]+name="externalIdentifier"[^>]+value="({ID_REGEX})""#
        ))
        .unwrap(),
        Regex::new(&format!(r#"videoId\s*:\s*["']({ID_REGEX})["']"#)).unwrap(),
    ]
});
static DIGITAL_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+digitalData\s*=\s*(\{.+?\});?\n").unwrap());
/// `<script[^>]+id=(["\'])initial-data\1[^>]+data-json=(["\'])(?P<json>(?:(?!\2).)+)`.
static REEL_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<script[^>]+id=(?:"initial-data"|'initial-data')[^>]+data-json=(?:"([^"\n]+)|'([^'\n]+))"#,
    )
    .unwrap()
});
static MORPH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bMorph\s*\.\s*setPayload\s*\([^,]+,").unwrap());
static MORPH_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:"leadMedia"|\\"videoData\\")\s*:"#).unwrap());
static PRELOADED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__(?:PWA_)?PRELOADED_STATE__\s*=").unwrap());
static BBC3_CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)bbcthreeConfig\s*=\s*(\{.+?\})\s*;\s*<").unwrap());
static QUOTED_INITIAL_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"window\.__INITIAL_DATA__\s*=\s*("\{.+?\}")\s*;"#).unwrap());
static INITIAL_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__INITIAL_DATA__\s*=\s*(\{.+?\})\s*;").unwrap());
static SIMORGH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\s*\.\s*SIMORGH_DATA\s*=").unwrap());
static SMP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"new\s+SMP\((\{.+?\})\)").unwrap());
static EMBED_URL: LazyLock<String> = LazyLock::new(|| {
    format!(r#"https?://(?:www\.)?bbc\.co\.uk/(?:[^/]+/)+{ID_REGEX}(?:\b[^"]+)?"#)
});
static EMBED_URL_MATCH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^(?:{})", *EMBED_URL)).unwrap());
static SET_PLAYLIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r#"setPlaylist\("({})"\)"#, *EMBED_URL)).unwrap());
static MEDIA_META: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"data-media-meta='(\{[^']+\})'").unwrap());
static MEDIA_ASSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"mediaAssetPage\.init\(\s*(\{.+?\}), "/"#).unwrap());
static VXP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<script[^>]+class="vxp-playlist-data"[^>]+type="application/json"[^>]*>([^<]+)</script>"#,
    )
    .unwrap()
});

/// `BBCCoUkIE._match_valid_url`.
pub fn couk_suitable(url: &str) -> bool {
    COUK_URL.is_match(url) && !COUK_EXCLUDED.is_match(url)
}

/// `BBCIE.suitable`: not a `BBCCoUkIE`, article, iPlayer playlist or
/// programme playlist URL.
pub fn bbc_suitable(url: &str) -> bool {
    BBC_URL.is_match(url)
        && !couk_suitable(url)
        && !super::bbc::article_suitable(url)
        && !super::bbc::iplayer_episodes_suitable(url)
        && !super::bbc::iplayer_group_suitable(url)
        && !super::bbc::suitable(url)
}

/// `BBCCoUkIE._extract_embed_urls`.
pub(crate) fn couk_embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    super::embed_regex_urls(url, webpage, &*EMBED, couk_suitable)
}

/// `BBCIE._extract_embed_urls` (the inherited pattern, `BBCIE.suitable`).
pub(crate) fn bbc_embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    super::embed_regex_urls(url, webpage, &*EMBED, bbc_suitable)
}

pub(crate) fn couk_temp_id(url: &str) -> Option<String> {
    COUK_URL.captures(url).map(|found| found["id"].to_owned())
}

pub(crate) fn bbc_temp_id(url: &str) -> Option<String> {
    BBC_URL.captures(url).map(|found| found["id"].to_owned())
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("BBC {what} are not supported natively"))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// `dict.get(key)` with `None` for a missing key or a JSON `null`.
fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|found| !found.is_null())
}

/// `try_get(value, lambda x: x[k1][k2]...)` of object keys and list indices.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .try_fold(value, |value, key| match value {
            Value::Array(items) => key.parse::<usize>().ok().and_then(|index| items.get(index)),
            _ => value.get(*key),
        })
        .filter(|found| !found.is_null())
}

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

/// A truthy string, else `None`.
fn nonempty(value: Option<&Value>) -> Option<String> {
    text(value).filter(|text| !text.is_empty())
}

/// `int_or_none`: Python's `int()` of numbers, booleans and integer text.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    super::streamable::int_or_none(value).as_i64()
}

fn int_value(value: Option<&Value>) -> Value {
    int_or_none(value).map_or(Value::Null, Value::from)
}

/// `int_or_none(v, scale=1000)`: `int(v) // 1000`.
fn int_scaled(value: Option<&Value>) -> Option<i64> {
    int_or_none(value).map(|value| value.div_euclid(1000))
}

/// `float_or_none(v, scale)`.
fn float_or_none(value: Option<&Value>, scale: f64) -> Option<f64> {
    let number = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        _ => return None,
    };
    Some(number / scale)
}

fn parse_duration(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(crate::parse_duration)
        .map_or(Value::Null, |duration| json!(duration))
}

fn parse_iso8601(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
}

/// `url_or_none`.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// Python's `str()` of a JSON scalar (an error ID or a format ID part).
fn py_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".into(),
        Value::Bool(flag) => if *flag { "True" } else { "False" }.into(),
        other => other.to_string(),
    }
}

/// `traverse_obj(value, (..., ))`: a list's items or an object's values.
fn children(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    }
}

/// `traverse_obj(value, (key, is_type(*types)))`: the children whose `type`
/// is one of `types`.
fn of_type<'a>(value: Option<&'a Value>, types: &[&str]) -> Vec<&'a Value> {
    children(value)
        .into_iter()
        .filter(|child| {
            child
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| types.contains(&kind))
        })
        .collect()
}

/// `traverse_obj(video, ('model', 'blocks', is_type(OUTER), 'model',
/// 'blocks', is_type(INNER), 'model', {dict}, any))`.
fn nested_model<'a>(video: &'a Value, outer: &str, inner: &str) -> Option<&'a Value> {
    of_type(path(video, &["model", "blocks"]), &[outer])
        .into_iter()
        .flat_map(|block| of_type(path(block, &["model", "blocks"]), &[inner]))
        .find_map(|block| block.get("model").filter(|model| model.is_object()))
}

/// The first match of several patterns (`_search_regex` with a list).
fn search_first(patterns: &[Regex], html: &str) -> Option<String> {
    patterns
        .iter()
        .find_map(|pattern| pattern.captures(html))
        .map(|found| found[1].to_owned())
}

/// The status of a failed request's HTTP error (`e.cause.status`).
fn http_status(error: &Error) -> Option<u16> {
    match error {
        Error::HttpStatus { status, .. } => Some(*status),
        Error::Http(error) => error.status(),
        Error::ExtractorRequest { source, .. } | Error::Webpage { source, .. } => {
            http_status(source)
        }
        _ => None,
    }
}

/// `_parse_json(text, video_id, fatal=False)`: `None` after the warning.
fn parse_json_optional(name: &str, text: &str, id: &str) -> Option<Value> {
    match crate::json::from_str::<Value>(text) {
        Ok(value) => Some(value),
        Err(error) => {
            let text =
                crate::json::python_decode_error_text(text).unwrap_or_else(|| error.to_string());
            crate::screen::warning(&format!("[{name}] {id}: Failed to parse JSON: {text}"));
            None
        }
    }
}

/// `_parse_json(text, video_id)` (fatal).
fn parse_json_fatal(text: &str, id: &str) -> Result<Value> {
    crate::json::from_str::<Value>(text).map_err(|error| {
        let text = crate::json::python_decode_error_text(text).unwrap_or_else(|| error.to_string());
        Error::Reported(format!(
            "{id}: Failed to parse JSON  (caused by JSONDecodeError('{text}')); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })
}

/// A media-selection result's `MediaSelectionError` or any other failure.
enum Failure {
    Selection(String),
    Error(Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Failure::Error(error)
    }
}

impl From<url::ParseError> for Failure {
    fn from(error: url::ParseError) -> Self {
        Failure::Error(error.into())
    }
}

/// The extractor running `BBCCoUkIE`'s shared methods.
struct Ctx<'a> {
    client: &'a Client,
    name: &'static str,
    key: &'static str,
    media_sets: &'static [&'static str],
    /// `get_temp_id(url)`: the URL's ID, for errors without a video ID.
    temp_id: String,
    url: &'a Url,
}

/// `(programme_id, title, description, duration, formats, subtitles)`.
struct Programme {
    id: Value,
    title: Option<String>,
    description: Option<String>,
    duration: Option<i64>,
    formats: Vec<VideoFormat>,
    subtitles: Option<Subtitles>,
}

/// `UnboundLocalError` for a variable a loop never set.
fn unbound(variable: &str) -> Error {
    Error::Reported(format!(
        "cannot access local variable '{variable}' where it is not associated with a value"
    ))
}

impl Ctx<'_> {
    fn warning(&self, message: &str) {
        crate::screen::warning(&format!("[{}] {message}", self.name));
    }

    /// `_raise_extractor_error`: `ExtractorError(f'{IE_NAME} returned error:
    /// {id}', expected=True)` with the URL's temporary ID.
    fn selection_error(&self, error_id: &str) -> Error {
        Error::Reported(format!(
            "[{}] {}: {} returned error: {error_id}",
            self.name, self.temp_id, self.name
        ))
    }

    /// `_download_xml(url, video_id, note)` (fatal): the parsed document's
    /// text.
    fn download_xml(&self, url: &str, id: &str, note: &str) -> Result<String> {
        let target = Url::parse(url)
            .map_err(|_| unsupported(&format!("relative or invalid XML URLs ({url})")))?;
        let xml = super::download_xml_note(self.client, &target, self.name, id, note)?;
        if let Err(error) = roxmltree::Document::parse(&xml) {
            let text = super::expat_error(&xml).unwrap_or_else(|| error.to_string());
            let prefix = if id.is_empty() {
                String::new()
            } else {
                format!("{id}: ")
            };
            return Err(Error::Reported(format!(
                "[{}] {prefix}Failed to parse XML  (caused by ParseError('{text}')); {}",
                self.name,
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        Ok(xml)
    }

    /// `_download_xml(url, video_id, note, fatal=False)`: `None` after the
    /// download or parse warning.
    fn download_xml_optional(&self, url: &str, id: &str, note: &str) -> Result<Option<String>> {
        let Ok(target) = Url::parse(url) else {
            return Err(unsupported(&format!(
                "relative or invalid XML URLs ({url})"
            )));
        };
        let xml = match super::download_xml_note(self.client, &target, self.name, id, note) {
            Ok(xml) => xml,
            Err(Error::ExtractorRequest { source, .. }) => {
                self.warning(&format!(
                    "{id}: Unable to download XML: {}",
                    crate::upstream_error_text(&source)
                ));
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if let Err(error) = roxmltree::Document::parse(&xml) {
            let text = super::expat_error(&xml).unwrap_or_else(|| error.to_string());
            self.warning(&format!("{id}: Failed to parse XML: {text}"));
            return Ok(None);
        }
        Ok(Some(xml))
    }

    /// `_download_json(url, programme_id, 'Downloading media selection
    /// JSON', expected_status=(403, 404))`.
    fn media_selection_json(&self, url: &Url, id: &str) -> Result<Value> {
        use std::io::Read;
        crate::screen::screen(&format!(
            "[{}] {id}: Downloading media selection JSON",
            self.name
        ));
        let mut response = super::direct::request_expecting(self.client, url, &[], &[403, 404])
            .map_err(|error| match error {
                Error::Http(_) | Error::HttpRetryAfter { .. } | Error::HttpStatus { .. } => {
                    Error::ExtractorRequest {
                        extractor: self.name,
                        id: id.to_owned(),
                        what: "JSON metadata",
                        source: Box::new(error),
                    }
                }
                error => error,
            })?;
        let page_url = response.url().clone();
        let mut body = Vec::new();
        response
            .read_to_end(&mut body)
            .map_err(Error::ExtractionRead)?;
        let body =
            super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
        crate::screen::expect_page(id, None, None);
        crate::screen::page_read(&page_url, &body)?;
        let text = super::decode_webpage(&body, response.headers());
        crate::json::from_str(&text).map_err(|error| {
            Error::Reported(format!(
                "[{}] {id}: Failed to parse JSON  (caused by {error})",
                self.name
            ))
        })
    }

    /// `_download_media_selector(programme_id)`.
    fn media_selector(&self, programme_id: &str) -> Result<(Vec<VideoFormat>, Subtitles)> {
        let mut last = None;
        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        for media_set in self.media_sets {
            let url = Url::parse(&format!("{MEDIA_SELECTOR}{media_set}/vpid/{programme_id}"))?;
            let selection = self.media_selection_json(&url, programme_id)?;
            match self.process_media_selector(&selection, programme_id) {
                Ok((found, subs)) => {
                    formats.extend(found);
                    if let Some(subs) = subs.filter(|subs| !subs.entries.is_empty()) {
                        super::svt::merge_subtitles(&mut subtitles, subs);
                    }
                }
                Err(Failure::Selection(id))
                    if matches!(
                        id.as_str(),
                        "notukerror" | "geolocation" | "selectionunavailable"
                    ) =>
                {
                    last = Some(id);
                }
                Err(Failure::Selection(id)) => return Err(self.selection_error(&id)),
                Err(Failure::Error(error)) => return Err(error),
            }
        }
        if let Some(id) = last {
            if formats.is_empty() && subtitles.entries.is_empty() {
                return Err(self.selection_error(&id));
            }
            self.warning(&format!("{} returned error: {id}", self.name));
        }
        Ok((formats, subtitles))
    }

    /// `_process_media_selector(media_selection, programme_id)`: formats and
    /// the captions' subtitles (`None` without a captions media).
    fn process_media_selector(
        &self,
        selection: &Value,
        programme_id: &str,
    ) -> std::result::Result<(Vec<VideoFormat>, Option<Subtitles>), Failure> {
        if !selection.is_object() {
            return Err(unsupported("media selections that are not objects").into());
        }
        // `_extract_medias`.
        if let Some(error) = selection.get("result").filter(|error| truthy(error)) {
            return Err(Failure::Selection(py_str(error)));
        }
        let medias = match selection.get("media") {
            Some(Value::Array(medias)) => medias.as_slice(),
            Some(media) if truthy(media) => {
                return Err(unsupported("media lists that are not lists").into());
            }
            _ => &[],
        };
        let mut formats = Vec::new();
        let mut subtitles = None;
        let mut urls: Vec<Value> = Vec::new();
        for media in medias {
            if !media.is_object() {
                return Err(unsupported("media entries that are not objects").into());
            }
            let kind = media.get("kind").and_then(Value::as_str);
            match kind {
                Some("video" | "audio") => {
                    let bitrate = int_or_none(media.get("bitrate"));
                    let encoding = media.get("encoding").cloned().unwrap_or(Value::Null);
                    let width = int_value(media.get("width"));
                    let height = int_value(media.get("height"));
                    let file_size = int_value(media.get("media_file_size"));
                    for connection in connections(media)? {
                        let href = connection.get("href").cloned().unwrap_or(Value::Null);
                        if urls.contains(&href) {
                            continue;
                        }
                        if truthy(&href) {
                            urls.push(href.clone());
                        }
                        let field = |key: &str| connection.get(key).cloned().unwrap_or(Value::Null);
                        let (conn_kind, protocol, supplier) =
                            (field("kind"), field("protocol"), field("supplier"));
                        let transfer_format =
                            connection.get("transferFormat").and_then(Value::as_str);
                        // `supplier or conn_kind or protocol`.
                        let format_id = [&supplier, &conn_kind]
                            .into_iter()
                            .find(|value| truthy(value))
                            .unwrap_or(&protocol)
                            .clone();
                        let format_text = match &format_id {
                            Value::String(text) => Some(text.clone()),
                            Value::Null => None,
                            _ => return Err(unsupported("non-string format IDs").into()),
                        };
                        let href_text = href.as_str();
                        if supplier.as_str() == Some("asx") {
                            let asx = self.download_xml(
                                href_text.unwrap_or_default(),
                                programme_id,
                                "Downloading ASX playlist",
                            )?;
                            let document = roxmltree::Document::parse(&asx)
                                .map_err(|error| Error::Reported(error.to_string()))?;
                            let refs = document
                                .root_element()
                                .children()
                                .filter(|entry| entry.has_tag_name("Entry"))
                                .flat_map(|entry| {
                                    entry.children().filter(|node| node.has_tag_name("ref"))
                                });
                            for (index, reference) in refs.enumerate() {
                                let Some(target) = reference.attribute("href") else {
                                    return Err(
                                        unsupported("ASX references without an href").into()
                                    );
                                };
                                formats.push(dict_format(
                                    vec![
                                        ("url", target.into()),
                                        (
                                            "format_id",
                                            format!("ref{index}_{}", py_str(&format_id)).into(),
                                        ),
                                    ],
                                    self.name,
                                    programme_id,
                                )?);
                            }
                            continue;
                        }
                        if matches!(transfer_format, Some("dash" | "hls" | "hds")) {
                            let Some(manifest) = href_text else {
                                return Err(
                                    unsupported("manifest connections without an href").into()
                                );
                            };
                            let manifest = Url::parse(manifest)?;
                            match transfer_format {
                                Some("dash") => {
                                    if let Some(stream) = super::extract_mpd_optional_note(
                                        self.client,
                                        &manifest,
                                        self.name,
                                        programme_id,
                                        format_text.as_deref(),
                                        "Downloading MPD manifest",
                                    ) {
                                        formats.extend(stream.formats);
                                    }
                                }
                                Some("hls") => {
                                    if let Some(stream) = super::extract_m3u8_master_optional(
                                        self.client,
                                        &manifest,
                                        self.name,
                                        programme_id,
                                        format_text.as_deref(),
                                        false,
                                        false,
                                    ) {
                                        formats.extend(stream.formats);
                                    }
                                }
                                _ => formats.extend(super::extract_f4m_optional(
                                    self.client,
                                    self.name,
                                    &manifest,
                                    programme_id,
                                    format_text.as_deref(),
                                )?),
                            }
                            continue;
                        }
                        // `if not supplier and bitrate: format_id += f'-{bitrate}'`.
                        let format_id = match (
                            format_text,
                            bitrate.filter(|bitrate| *bitrate != 0 && !truthy(&supplier)),
                        ) {
                            (Some(id), Some(bitrate)) => Value::from(format!("{id}-{bitrate}")),
                            (None, Some(_)) => {
                                return Err(Error::Reported(
                                    "unsupported operand type(s) for +=: 'NoneType' and 'str'"
                                        .into(),
                                )
                                .into());
                            }
                            (id, None) => id.map_or(Value::Null, Value::from),
                        };
                        let mut fields =
                            vec![("format_id", format_id), ("filesize", file_size.clone())];
                        if kind == Some("video") {
                            fields.extend([
                                ("width", width.clone()),
                                ("height", height.clone()),
                                ("tbr", bitrate.map_or(Value::Null, Value::from)),
                                ("vcodec", encoding.clone()),
                            ]);
                        } else {
                            fields.extend([
                                ("abr", bitrate.map_or(Value::Null, Value::from)),
                                ("acodec", encoding.clone()),
                                ("vcodec", "none".into()),
                            ]);
                        }
                        match protocol.as_str() {
                            Some("http" | "https") => fields.push(("url", href.clone())),
                            Some("rtmp") => {
                                let application = connection
                                    .get("application")
                                    .map_or_else(|| "ondemand".into(), py_str);
                                let auth = py_str(&field("authString"));
                                let server = py_str(&field("server"));
                                fields.extend([
                                    (
                                        "url",
                                        format!("rtmp://{server}/{application}?{auth}").into(),
                                    ),
                                    ("play_path", field("identifier")),
                                    ("app", format!("{application}?{auth}").into()),
                                    ("page_url", "http://www.bbc.co.uk".into()),
                                    ("player_url", RTMP_PLAYER.into()),
                                    ("rtmp_live", false.into()),
                                    ("ext", "flv".into()),
                                ]);
                            }
                            _ => continue,
                        }
                        if fields
                            .iter()
                            .any(|(key, value)| *key == "url" && !value.is_string())
                        {
                            return Err(unsupported("direct connections without an href").into());
                        }
                        let mut format = dict_format(fields, self.name, programme_id)?;
                        if bitrate.is_some() {
                            // `int_or_none(media.get('bitrate'))` as `tbr`/`abr`.
                            crate::info_order::mark_int_rates(&mut format);
                        }
                        formats.push(format);
                    }
                }
                Some("captions") => {
                    // `extract_subtitles`: `{}` unless subtitles are requested.
                    subtitles = Some(if super::subtitles_requested() {
                        self.captions(media, programme_id)?
                    } else {
                        Subtitles::default()
                    });
                }
                _ => {}
            }
        }
        Ok((formats, subtitles))
    }

    /// `_get_subtitles(media, programme_id)`: the first connection whose
    /// captions parse, as English TTML.
    fn captions(&self, media: &Value, programme_id: &str) -> Result<Subtitles> {
        let mut subtitles = Subtitles::default();
        for connection in connections(media)? {
            let Some(cc_url) = url_or_none(connection.get("href")) else {
                continue;
            };
            if self
                .download_xml_optional(&cc_url, programme_id, "Downloading captions")?
                .is_none()
            {
                continue;
            }
            subtitles.entries.push((
                Some("en".into()),
                vec![SubtitleFormat {
                    name: None,
                    url: Url::parse(&cc_url)?,
                    ext: "ttml".into(),
                    protocol: None,
                    fragments: Vec::new(),
                    extra: Map::new(),
                }],
            ));
            break;
        }
        Ok(subtitles)
    }

    /// `_download_playlist(playlist_id)`: `playlist.json`'s versions, else
    /// (HTTP 404) the legacy playlist.
    fn download_playlist(&self, playlist_id: &str) -> Result<Programme> {
        match self.playlist_json(playlist_id) {
            Err(error) if http_status(&error) == Some(404) => {}
            result => return result,
        }
        self.process_legacy_playlist_url(
            &format!("http://www.bbc.co.uk/iplayer/playlist/{playlist_id}"),
            playlist_id,
        )
    }

    fn playlist_json(&self, playlist_id: &str) -> Result<Programme> {
        let url = Url::parse(&format!(
            "http://www.bbc.co.uk/programmes/{playlist_id}/playlist.json"
        ))?;
        let playlist = super::download_json_note(
            self.client,
            &url,
            self.name,
            playlist_id,
            "Downloading playlist JSON",
        )?;
        let key = |value: &Value, name: &str| -> Result<Value> {
            value
                .get(name)
                .cloned()
                .ok_or_else(|| key_error(&self.temp_id, name))
        };
        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        let (mut programme_id, mut title, mut description, mut duration) = (None, None, None, None);
        let versions = match playlist.get("allAvailableVersions") {
            None => Vec::new(),
            Some(Value::Array(versions)) => versions.clone(),
            Some(_) => return Err(unsupported("version lists that are not lists")),
        };
        for version in &versions {
            let smp_config = key(version, "smpConfig")?;
            title = Some(key(&smp_config, "title")?);
            description = Some(key(&smp_config, "summary")?);
            let items = key(&smp_config, "items")?;
            let Value::Array(items) = items else {
                return Err(unsupported("playlist items that are not lists"));
            };
            for item in &items {
                let kind = key(item, "kind")?;
                if !matches!(kind.as_str(), Some("programme" | "radioProgramme")) {
                    continue;
                }
                let vpid = item.get("vpid").cloned().unwrap_or(Value::Null);
                duration = Some(int_or_none(item.get("duration")));
                let (mut version_formats, version_subtitles) =
                    self.media_selector(&py_str(&vpid))?;
                programme_id = Some(vpid);
                let types: Vec<String> = match key(version, "types")? {
                    Value::Array(types) => types.iter().map(py_str).collect(),
                    _ => return Err(unsupported("version types that are not lists")),
                };
                for format in &mut version_formats {
                    format.format_note = Some(types.join(", "));
                    if types.iter().any(|kind| kind.contains("AudioDescribed")) {
                        format
                            .extra
                            .insert("language_preference".into(), (-10).into());
                    }
                }
                formats.extend(version_formats);
                for (tag, tracks) in version_subtitles.entries {
                    match subtitles.entries.iter_mut().find(|(key, _)| *key == tag) {
                        Some((_, existing)) => existing.extend(tracks),
                        None => subtitles.entries.push((tag, tracks)),
                    }
                }
            }
        }
        let id = programme_id.ok_or_else(|| unbound("programme_id"))?;
        let title = title.ok_or_else(|| unbound("title"))?;
        let description = description.ok_or_else(|| unbound("description"))?;
        let duration = duration.ok_or_else(|| unbound("duration"))?;
        Ok(Programme {
            id,
            title: title.as_str().map(str::to_owned),
            description: description.as_str().map(str::to_owned),
            duration,
            formats,
            subtitles: Some(subtitles),
        })
    }

    /// `_process_legacy_playlist_url(url, display_id)`.
    fn process_legacy_playlist_url(&self, url: &str, display_id: &str) -> Result<Programme> {
        let xml = self.download_xml(url, display_id, "Downloading legacy playlist XML")?;
        self.extract_from_legacy_playlist(&xml, display_id)
    }

    /// `_extract_from_legacy_playlist(playlist, playlist_id)`.
    fn extract_from_legacy_playlist(&self, xml: &str, playlist_id: &str) -> Result<Programme> {
        let document =
            roxmltree::Document::parse(xml).map_err(|error| Error::Reported(error.to_string()))?;
        let playlist = document.root_element();
        if let Some(no_items) = child(playlist, "noItems") {
            let reason = no_items.attribute("reason");
            let message = match reason {
                Some("preAvailability") => format!("Episode {playlist_id} is not yet available"),
                Some("postAvailability") => {
                    format!("Episode {playlist_id} is no longer available")
                }
                Some("noMedia") => format!("Episode {playlist_id} is not currently available"),
                _ => format!(
                    "Episode {playlist_id} is not available: {}",
                    reason.unwrap_or("None")
                ),
            };
            return Err(Error::Reported(format!(
                "[{}] {}: {message}",
                self.name, self.temp_id
            )));
        }
        let mut found = None;
        for item in playlist
            .children()
            .filter(|child| child.has_tag_name((EMP_NS, "item")))
        {
            if !matches!(item.attribute("kind"), Some("programme" | "radioProgramme")) {
                continue;
            }
            let title = child(playlist, "title")
                .ok_or_else(|| Error::Reported("'NoneType' object has no attribute 'text'".into()))?
                .text()
                .map(str::to_owned);
            let description =
                child(playlist, "summary").and_then(|summary| summary.text().map(str::to_owned));
            // `get_programme_id`: the item's (else its mediator's)
            // `identifier` or `group` that looks like a programme ID.
            let from_attributes = |node: roxmltree::Node<'_, '_>| {
                ["identifier", "group"].into_iter().find_map(|name| {
                    node.attribute(name)
                        .filter(|value| LEGACY_ID.is_match(value))
                        .map(str::to_owned)
                })
            };
            let programme_id =
                from_attributes(item).or_else(|| child(item, "mediator").and_then(from_attributes));
            let duration = item
                .attribute("duration")
                .and_then(|duration| int_or_none(Some(&Value::from(duration))));
            let (id, formats, subtitles) = match programme_id {
                Some(programme_id) => {
                    let (formats, subtitles) = self.media_selector(&programme_id)?;
                    (programme_id, formats, Some(subtitles))
                }
                // `_process_media_selector(item, ...)` of an XML element:
                // its `result` attribute, else no media.
                None => {
                    if let Some(error) = item.attribute("result").filter(|error| !error.is_empty())
                    {
                        return Err(Error::Reported(error.to_owned()));
                    }
                    (playlist_id.to_owned(), Vec::new(), None)
                }
            };
            found = Some(Programme {
                id: id.into(),
                title,
                description,
                duration,
                formats,
                subtitles,
            });
        }
        found.ok_or_else(|| unbound("programme_id"))
    }

    /// `_perform_login`.
    fn login(&self, session: &super::login::Session, username: &str, password: &str) -> Result<()> {
        let login_url = Url::parse(LOGIN_URL)?;
        let page = session.request(
            self.client,
            &login_url,
            (
                Some("Downloading signin page"),
                "Unable to download webpage",
            ),
            &[],
            None,
            &[],
        )?;
        let mut form = super::login::hidden_inputs(&page.text);
        super::login::set_field(&mut form, "username", username);
        super::login::set_field(&mut form, "password", password);
        let action = FORM_ACTION
            .captures(&page.text)
            .and_then(|found| found.get(1).or_else(|| found.get(2)))
            .map_or(LOGIN_URL, |found| found.as_str());
        let post_url = login_url.join(action)?;
        let pairs: Vec<(&str, &str)> = form
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let response = session.request(
            self.client,
            &post_url,
            (Some("Logging in"), "Unable to download webpage"),
            &[("Referer", LOGIN_URL)],
            Some(super::login::urlencode(&pairs)),
            &[],
        )?;
        if response.url.as_str().contains(LOGIN_URL) {
            let error = crate::html_elements::elements_text_and_html_by_class(
                "form-message",
                &response.text,
            )
            .ok()
            .and_then(|elements| elements.into_iter().next())
            .map(|(text, _)| crate::html::clean_html(&text))
            .filter(|error| !error.is_empty());
            return Err(match error {
                Some(error) => session.error(&format!("Unable to login: {error}")),
                None => session.bug("Unable to log in"),
            });
        }
        Ok(())
    }
}

/// `element.find(f'./{{{_EMP_PLAYLIST_NS}}}NAME')`.
fn child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| child.has_tag_name((EMP_NS, name)))
}

/// `_extract_connections(media)`: `media.get('connection') or []`.
fn connections(media: &Value) -> Result<&[Value]> {
    match media.get("connection") {
        Some(Value::Array(connections)) => Ok(connections),
        Some(connection) if truthy(connection) => {
            Err(unsupported("connection lists that are not lists"))
        }
        _ => Ok(&[]),
    }
}

/// An info dict's fields: `None` values keep their key's position.
type Fields = Vec<(&'static str, Value)>;

fn opt_str(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::from)
}

fn opt_int(value: Option<i64>) -> Value {
    value.map_or(Value::Null, Value::from)
}

/// An info dict with its formats and subtitles.
fn video(
    ctx: &Ctx<'_>,
    fields: Fields,
    formats: Vec<VideoFormat>,
    subtitles: Option<Subtitles>,
) -> VideoInfo {
    let mut info = dict_info(ctx.url, (ctx.name, ctx.key), fields, formats);
    if let Some(subtitles) = subtitles {
        info.subtitles = subtitles;
    }
    info
}

/// `playlist_result(entries, playlist_id, playlist_title,
/// playlist_description)`.
fn playlist_result(
    ctx: &Ctx<'_>,
    id: &str,
    title: Option<String>,
    description: Option<String>,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut extra = Map::new();
    let mut keys = vec!["id"];
    let title = title.filter(|title| !title.is_empty());
    if title.is_some() {
        keys.push("title");
    }
    if let Some(description) = description {
        keys.push("description");
        extra.insert("description".into(), description.into());
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id: id.to_owned(),
        title: title.unwrap_or_default(),
        webpage_url: ctx.url.clone(),
        extractor: ctx.name.into(),
        extractor_key: ctx.key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

/// `url_result(url, 'BBCCoUk')` as a playlist entry.
fn url_entry(
    target: &str,
    id: Option<&str>,
    transparent: Option<Map<String, Value>>,
) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), COUK_KEY.into());
    if let Some(id) = id {
        info.id = id.to_owned();
        entry.insert("id".into(), id.into());
    }
    let fields = match transparent {
        Some(fields) => {
            entry.insert("_type".into(), "url_transparent".into());
            entry.insert("url".into(), target.into());
            for (key, value) in &fields {
                entry.insert(key.clone(), value.clone());
            }
            if let Some(title) = fields.get("title").and_then(Value::as_str) {
                info.title = title.to_owned();
            }
            fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .collect()
        }
        None => {
            entry.insert("_type".into(), "url".into());
            entry.insert("url".into(), target.into());
            Map::new()
        }
    };
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": fields,
            "ie_key": COUK_KEY,
            "extractor": COUK_KEY,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for BbcCoUk {
    fn key(&self) -> &'static str {
        COUK_KEY
    }

    fn netrc_machine(&self) -> Option<&'static str> {
        Some("bbc")
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
        let page = Url::parse(LOGIN_URL)?;
        let ctx = Ctx {
            client,
            name: COUK_NAME,
            key: COUK_KEY,
            media_sets: COUK_MEDIA_SETS,
            temp_id: String::new(),
            url: &page,
        };
        ctx.login(session, username, password)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !couk_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(group_id) = couk_temp_id(url.as_str()) else {
            return Ok(None);
        };
        self.initialize(client, COUK_NAME, &group_id)?;
        announce(COUK_NAME, url);
        let ctx = Ctx {
            client,
            name: COUK_NAME,
            key: COUK_KEY,
            media_sets: COUK_MEDIA_SETS,
            temp_id: group_id.clone(),
            url,
        };
        let webpage =
            download_webpage_note(client, url, COUK_NAME, &group_id, "Downloading video page")?;
        if let Some(found) = PAGE_ERROR.captures(&webpage) {
            return Err(Error::Reported(format!(
                "[{COUK_NAME}] {group_id}: {}",
                &found[1]
            )));
        }

        let mut programme_id = None;
        let mut duration = None;
        if let Some(found) = TVIPLAYER.captures(&webpage) {
            let parsed = parse_json_fatal(&found[1], &group_id)
                .map_err(|error| Error::Reported(format!("[{COUK_NAME}] {error}")))?;
            if !parsed.is_object() {
                return Err(unsupported("player data that is not an object"));
            }
            let player = parsed.get("player").cloned().unwrap_or_else(|| json!({}));
            duration = int_or_none(player.get("duration"));
            programme_id = player.get("vpid").filter(|vpid| truthy(vpid)).map(py_str);
        }
        if programme_id.is_none() {
            programme_id = VPID.captures(&webpage).map(|found| found[1].to_owned());
        }

        let programme = match programme_id {
            Some(programme_id) => {
                let (formats, subtitles) = ctx.media_selector(&programme_id)?;
                let title = match crate::html::og_search_property(&webpage, "title") {
                    Some(title) => Some(title),
                    None => Some(
                        search_first(&*PARENT_TITLE, &webpage)
                            .map(|title| crate::html::clean_html(&title).trim().to_owned())
                            .ok_or_else(|| {
                                Error::Reported(format!(
                                    "[{COUK_NAME}] {group_id}: Unable to extract title; {}",
                                    crate::BUG_REPORTS_MESSAGE
                                ))
                            })?,
                    ),
                };
                let mut description = search_first(&*COUK_DESCRIPTION, &webpage);
                if description.as_deref().is_none_or(str::is_empty) {
                    description = crate::html::html_search_meta(&webpage, "description");
                    if description.is_none() {
                        search_warning(COUK_NAME, "description");
                    }
                }
                Programme {
                    id: programme_id.into(),
                    title,
                    description,
                    duration,
                    formats,
                    subtitles: Some(subtitles),
                }
            }
            None => ctx.download_playlist(&group_id)?,
        };

        let thumbnail = crate::html::og_search_property(&webpage, "image");
        let id = match &programme.id {
            Value::String(id) => id.clone(),
            Value::Null => String::new(),
            _ => return Err(unsupported("programme IDs that are not strings")),
        };
        Ok(Some(video(
            &ctx,
            vec![
                (
                    "id",
                    if programme.id.is_null() {
                        Value::Null
                    } else {
                        id.into()
                    },
                ),
                ("title", opt_str(programme.title)),
                ("description", opt_str(programme.description)),
                ("thumbnail", opt_str(thumbnail)),
                ("duration", opt_int(programme.duration)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            programme.formats,
            programme.subtitles,
        )))
    }
}

/// `BBCIE._real_extract`'s state.
struct Page<'a> {
    ctx: Ctx<'a>,
    webpage: String,
    playlist_id: String,
    playlist_title: Option<String>,
    playlist_description: Option<String>,
    timestamp: Option<i64>,
}

impl Page<'_> {
    fn playlist(&self, entries: Vec<VideoInfo>) -> ExtractResult {
        playlist_result(
            &self.ctx,
            &self.playlist_id,
            self.playlist_title.clone(),
            self.playlist_description.clone(),
            entries,
        )
    }

    /// `_og_search_title(webpage)`: `None` after the warning.
    fn og_title(&self) -> Option<String> {
        let title = crate::html::og_search_property(&self.webpage, "title");
        if title.is_none() {
            search_warning(BBC_NAME, "OpenGraph title");
        }
        title
    }

    /// `_extract_from_playlist_sxml(url, playlist_id, timestamp)`.
    fn playlist_sxml(&self, url: &str) -> Result<VideoInfo> {
        let programme = self
            .ctx
            .process_legacy_playlist_url(url, &self.playlist_id)?;
        Ok(self.programme_entry(programme))
    }

    fn programme_entry(&self, programme: Programme) -> VideoInfo {
        video(
            &self.ctx,
            vec![
                ("id", programme.id),
                ("title", opt_str(programme.title)),
                ("description", opt_str(programme.description)),
                ("duration", opt_int(programme.duration)),
                ("timestamp", opt_int(self.timestamp)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            programme.formats,
            programme.subtitles,
        )
    }

    /// `parse_model(model)`: a `versions[0].versionId` with its metadata.
    fn parse_model(&self, model: Option<&Value>) -> Result<Option<(VideoInfo, bool)>> {
        let Some(model) = model else {
            return Ok(None);
        };
        let Some(item_id) = path(model, &["versions", "0", "versionId"]).and_then(Value::as_str)
        else {
            return Ok(None);
        };
        let (formats, subtitles) = self.ctx.media_selector(item_id)?;
        let thumbnail = model
            .get("imageUrl")
            .and_then(Value::as_str)
            .and_then(|image| {
                self.ctx
                    .url
                    .join(&image.replace("$recipe", "raw"))
                    .ok()
                    .map(|url| url.to_string())
            });
        let description = ["long", "medium", "short"]
            .into_iter()
            .find_map(|key| nonempty(path(model, &["synopses", key])));
        let duration = path(model, &["versions", "0", "duration"])
            .and_then(|duration| duration.as_i64().map(Value::from));
        let timestamp = int_scaled(path(model, &["versions", "0", "availableFrom"]));
        let mut fields: Fields = vec![
            ("id", item_id.into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        let optional = [
            ("title", text(model.get("title")).map(Value::from)),
            ("thumbnail", thumbnail.map(Value::from)),
            ("description", description.map(Value::from)),
            ("duration", duration),
            ("timestamp", timestamp.map(Value::from)),
        ];
        let has_timestamp = timestamp.is_some();
        fields.extend(
            optional
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        );
        Ok(Some((
            video(&self.ctx, fields, formats, Some(subtitles)),
            has_timestamp,
        )))
    }
}

impl Extractor for Bbc {
    fn key(&self) -> &'static str {
        BBC_KEY
    }

    fn netrc_machine(&self) -> Option<&'static str> {
        Some("bbc")
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
        let page = Url::parse(LOGIN_URL)?;
        let ctx = Ctx {
            client,
            name: BBC_NAME,
            key: BBC_KEY,
            media_sets: BBC_MEDIA_SETS,
            temp_id: String::new(),
            url: &page,
        };
        ctx.login(session, username, password)
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !bbc_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(playlist_id) = bbc_temp_id(url.as_str()) else {
            return Ok(None);
        };
        self.initialize(client, BBC_NAME, &playlist_id)?;
        announce(BBC_NAME, url);
        let webpage = download_webpage(client, url, BBC_NAME, &playlist_id)?;
        let ctx = Ctx {
            client,
            name: BBC_NAME,
            key: BBC_KEY,
            media_sets: BBC_MEDIA_SETS,
            temp_id: playlist_id.clone(),
            url,
        };

        let json_ld = super::json_ld::search(&webpage);
        let mut timestamp = json_ld.get("timestamp").and_then(Value::as_i64);
        let playlist_title = nonempty(json_ld.get("title")).or_else(|| {
            let title = super::html5::page_title(&webpage).unwrap_or_default();
            let title = TITLE_SUFFIX.replace(&title, "$1");
            Some(title.trim().to_owned()).filter(|title| !title.is_empty())
        });
        let playlist_description = nonempty(json_ld.get("description"))
            .or_else(|| crate::html::og_search_property(&webpage, "description"));
        if timestamp.is_none() {
            timestamp = search_first(&*PUBLISHED, &webpage)
                .and_then(|date| crate::dates::parse_iso8601(&date));
        }
        let page = Page {
            ctx,
            webpage,
            playlist_id,
            playlist_title,
            playlist_description,
            timestamp,
        };
        extract_page(&page).map(Some)
    }
}

/// `BBCIE._real_extract` after the page metadata.
fn extract_page(page: &Page<'_>) -> Result<ExtractResult> {
    let ctx = &page.ctx;
    let webpage = page.webpage.as_str();
    let playlist_id = page.playlist_id.as_str();
    let mut entries = Vec::new();

    // Articles with `playlist.sxml` players.
    let playlists: Vec<String> = PARAM_PLAYLIST
        .captures_iter(webpage)
        .chain(MEDIA_ID_SXML.captures_iter(webpage))
        .map(|found| found[1].to_owned())
        .collect();
    for playlist_url in &playlists {
        entries.push(page.playlist_sxml(playlist_url)?);
    }

    // News articles with `data-playable` players.
    for found in DATA_PLAYABLE.captures_iter(webpage) {
        let json_text = found
            .get(1)
            .or_else(|| found.get(2))
            .map_or("", |found| found.as_str());
        let Some(data_playable) = parse_json_optional(
            BBC_NAME,
            &crate::html::unescape_html(json_text),
            playlist_id,
        )
        .filter(truthy) else {
            continue;
        };
        if !data_playable.is_object() {
            return Err(unsupported("data-playable values that are not objects"));
        }
        let settings = data_playable
            .get("settings")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !truthy(&settings) {
            continue;
        }
        if !settings.is_object() {
            return Err(unsupported("data-playable settings that are not objects"));
        }
        let playlist_object = settings
            .get("playlistObject")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if truthy(&playlist_object) {
            let Some(Value::Array(items)) = playlist_object.get("items") else {
                continue;
            };
            if items.is_empty() {
                continue;
            }
            let title = playlist_object
                .get("title")
                .cloned()
                .ok_or_else(|| key_error(playlist_id, "title"))?;
            let description = playlist_object
                .get("summary")
                .cloned()
                .unwrap_or(Value::Null);
            let duration = int_or_none(items[0].get("duration"));
            let programme_id = py_str(items[0].get("vpid").unwrap_or(&Value::Null));
            let (formats, subtitles) = ctx.media_selector(&programme_id)?;
            entries.push(video(
                ctx,
                vec![
                    ("id", programme_id.into()),
                    ("title", title),
                    ("description", description),
                    ("timestamp", opt_int(page.timestamp)),
                    ("duration", opt_int(duration)),
                    ("formats", Value::Null),
                    ("subtitles", Value::Null),
                ],
                formats,
                Some(subtitles),
            ));
            continue;
        }
        // Settings without a playlist object: `playlist.sxml` URLs in
        // `otherSettings` (upstream's `else` of `if playlist_object`).
        let playlist = path(&data_playable, &["otherSettings", "playlist"])
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !truthy(&playlist) {
            continue;
        }
        let mut entry: Option<VideoInfo> = None;
        for key in ["streaming", "progressiveDownload"] {
            let Some(playlist_url) = nonempty(playlist.get(format!("{key}Url").as_str())) else {
                continue;
            };
            let info = match page.playlist_sxml(&playlist_url) {
                Ok(info) => info,
                // Some playlist URLs fail with HTTP 500.
                Err(error) if http_status(&error) == Some(500) => continue,
                Err(error) => return Err(error),
            };
            match &mut entry {
                None => entry = Some(info),
                Some(entry) => {
                    entry.title = info.title;
                    entry.formats.extend(info.formats);
                }
            }
        }
        entries.extend(entry);
    }
    if !entries.is_empty() {
        return Ok(page.playlist(entries));
    }

    // A learningenglish-style video `div` handled by `BBCCoUkIE`.
    if let Some(found) = GROUP_ID.captures(webpage) {
        return Ok(super::follow_url(
            ctx.url,
            format!("https://www.bbc.co.uk/programmes/{}", &found[1]),
        ));
    }

    // Single video stories.
    if let Some(programme_id) = search_first(&*PAGE_VPID, webpage) {
        let (formats, subtitles) = ctx.media_selector(&programme_id)?;
        let digital_data = match DIGITAL_DATA.captures(webpage) {
            Some(found) => parse_json_optional(BBC_NAME, &found[1], &programme_id),
            None => Some(json!({})),
        };
        let Some(digital_data) = digital_data else {
            return Err(Error::Reported(
                "'NoneType' object has no attribute 'get'".into(),
            ));
        };
        let page_info = path(&digital_data, &["page", "pageInfo"])
            .cloned()
            .unwrap_or_else(|| json!({}));
        let title = nonempty(page_info.get("pageName")).or_else(|| page.og_title());
        let description = nonempty(page_info.get("description")).or_else(|| {
            let description = crate::html::og_search_property(webpage, "description");
            if description.is_none() {
                search_warning(BBC_NAME, "OpenGraph description");
            }
            description
        });
        let timestamp = parse_iso8601(page_info.get("publicationDate")).or(page.timestamp);
        return Ok(ExtractResult::Video(video(
            ctx,
            vec![
                ("id", programme_id.into()),
                ("title", opt_str(title)),
                ("description", opt_str(description)),
                ("timestamp", opt_int(timestamp)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
            Some(subtitles),
        )));
    }

    // BBC Reel.
    let reel = REEL_DATA
        .captures(webpage)
        .and_then(|found| found.get(1).or_else(|| found.get(2)))
        .map_or_else(
            || "{}".to_owned(),
            |found| crate::html::clean_html(found.as_str()).trim().to_owned(),
        );
    if let Some(initial_data) = parse_json_optional(BBC_NAME, &reel, playlist_id).filter(truthy) {
        let init_data = path(&initial_data, &["initData", "items", "0"])
            .filter(|data| data.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let smp_data = get(&init_data, "smpData")
            .filter(|data| truthy(data))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let clip_data = path(&smp_data, &["items", "0"])
            .filter(|data| data.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(version_id) = clip_data.get("versionID").filter(|id| truthy(id)) {
            let version_id = py_str(version_id);
            let title = smp_data
                .get("title")
                .cloned()
                .ok_or_else(|| key_error(playlist_id, "title"))?;
            let (formats, subtitles) = ctx.media_selector(&version_id)?;
            let thumbnail = nonempty(smp_data.get("holdingImageURL"))
                .map(|image| image.replace("$recipe", "raw"));
            let description = smp_data
                .get("summary")
                .filter(|summary| truthy(summary))
                .or_else(|| init_data.get("shortSummary"))
                .cloned()
                .unwrap_or(Value::Null);
            let upload_date =
                nonempty(init_data.get("displayDate")).map(|date| date.replace('-', ""));
            let categories = init_data
                .get("topicTitle")
                .filter(|title| truthy(title))
                .map_or(Value::Null, |title| json!([title]));
            return Ok(ExtractResult::Video(video(
                ctx,
                vec![
                    ("id", version_id.into()),
                    ("title", title),
                    ("formats", Value::Null),
                    (
                        "alt_title",
                        init_data.get("shortTitle").cloned().unwrap_or(Value::Null),
                    ),
                    ("thumbnail", opt_str(thumbnail)),
                    ("description", description),
                    ("upload_date", opt_str(upload_date)),
                    ("subtitles", Value::Null),
                    ("duration", opt_int(int_or_none(clip_data.get("duration")))),
                    ("categories", categories),
                ],
                formats,
                Some(subtitles),
            )));
        }
    }

    // Morph based embeds.
    if let Some(morph_payload) = morph_payload(webpage).filter(truthy)
        && let Some(result) = morph(page, &morph_payload)?
    {
        return Ok(result);
    }

    // `__PRELOADED_STATE__` JSON.
    let preload_state = super::search_json(&PRELOADED, webpage, true)
        .flatten()
        .unwrap_or_else(|| json!({}));
    if let Some(result) = preloaded(page, &preload_state)? {
        return Ok(result);
    }

    // BBC Three's config.
    let bbc3_config = match BBC3_CONFIG.captures(webpage) {
        Some(found) => parse_json_optional(
            BBC_NAME,
            &crate::js_to_json::js_to_json(&found[1]),
            playlist_id,
        ),
        None => Some(json!({})),
    }
    .filter(truthy)
    .unwrap_or_else(|| json!({}));
    let payload = get(&bbc3_config, "payload")
        .filter(|payload| truthy(payload))
        .cloned()
        .unwrap_or_else(|| json!({}));
    if truthy(&payload)
        && let Some(result) = bbc_three(page, &payload)?
    {
        return Ok(result);
    }

    // `__INITIAL_DATA__` articles and media experiences.
    let initial_data = match QUOTED_INITIAL_DATA.captures(webpage) {
        Some(found) => {
            let text = parse_json_optional(BBC_NAME, &found[1], playlist_id);
            match text {
                Some(Value::String(text)) => Some(text),
                Some(_) => return Err(unsupported("quoted initial data that is not a string")),
                None => {
                    return Err(Error::Reported(
                        "the JSON object must be str, bytes or bytearray, not NoneType".into(),
                    ));
                }
            }
        }
        None => Some(
            INITIAL_DATA
                .captures(webpage)
                .map_or_else(|| "{}".to_owned(), |found| found[1].to_owned()),
        ),
    };
    let initial_data = initial_data
        .and_then(|text| parse_json_optional(BBC_NAME, &text, playlist_id))
        .filter(truthy);
    if let Some(initial_data) = initial_data {
        return initial_data_result(page, &initial_data);
    }

    // `SIMORGH_DATA` hydration JSON.
    let simorgh = super::search_json(&SIMORGH, webpage, false)
        .flatten()
        .filter(truthy);
    if let Some(simorgh) = simorgh {
        let entries = simorgh_entries(page, &simorgh)?;
        if !entries.is_empty() {
            return Ok(page.playlist(entries));
        }
    }

    // Next.js articles (as served in the US).
    let next_data = super::search_nextjs_data(webpage, BBC_NAME, playlist_id)
        .ok()
        .and_then(|data| path(&data, &["props", "pageProps", "page"]).cloned());
    let pages = children(next_data.as_ref());
    let model = pages
        .iter()
        .flat_map(|page| of_type(page.get("contents"), &["video"]))
        .flat_map(|video| {
            of_type(path(video, &["model", "blocks"]), &["media"])
                .into_iter()
                .flat_map(|block| of_type(path(block, &["model", "blocks"]), &["mediaMetadata"]))
        })
        .find_map(|block| block.get("model").filter(|model| model.is_object()));
    if let Some((mut entry, has_timestamp)) = page.parse_model(model)? {
        if !has_timestamp {
            let timestamp = pages
                .iter()
                .flat_map(|page| of_type(page.get("contents"), &["timestamp"]))
                .find_map(|block| int_scaled(path(block, &["model", "timestamp"])));
            entry = with_timestamp(entry, timestamp);
        }
        return Ok(page.playlist(vec![entry]));
    }

    // SMP embeds and `setPlaylist` calls.
    let mut embeds = Vec::new();
    for found in SMP.captures_iter(webpage) {
        let Some(settings) = parse_json_optional(BBC_NAME, &found[1], playlist_id).filter(truthy)
        else {
            continue;
        };
        if let Some(embed_url) = path(&settings, &["playerSettings", "externalEmbedUrl"])
            .filter(|embed| truthy(embed))
            .and_then(Value::as_str)
            .filter(|embed| EMBED_URL_MATCH.is_match(embed))
        {
            embeds.push(embed_url.to_owned());
        }
    }
    embeds.extend(
        SET_PLAYLIST
            .captures_iter(webpage)
            .map(|found| found[1].to_owned()),
    );
    if !embeds.is_empty() {
        let entries = embeds
            .iter()
            .map(|embed| url_entry(embed, None, None))
            .collect::<Result<Vec<_>>>()?;
        return Ok(page.playlist(entries));
    }

    media_metas(page)
}

/// `entry['timestamp'] = ...` on a `parse_model` entry without one.
fn with_timestamp(entry: VideoInfo, timestamp: Option<i64>) -> VideoInfo {
    let mut entry = entry;
    if let Some(timestamp) = timestamp {
        crate::metadata_parser::set_field(&mut entry, "timestamp", timestamp.into());
    }
    let shape = entry
        .extra
        .get(crate::info_order::INFO_SHAPE)
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(shape) = shape
        && !shape.split([';', ',']).any(|key| key == "timestamp")
    {
        entry.extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("{shape},timestamp").into(),
        );
    }
    entry
}

/// `_search_json(r'\bMorph\s*\.\s*setPayload\s*\([^,]+,', ...,
/// contains_pattern=r'{(?s:(?:(?!</script>).)+(?:"leadMedia"|\\"videoData\\")\s*:.+)}',
/// default={})`: the first payload naming `leadMedia` or `videoData` before
/// its `</script>`, parsed up to the page's last `}`.
fn morph_payload(webpage: &str) -> Option<Value> {
    let last = webpage.rfind('}')?;
    for found in MORPH.find_iter(webpage) {
        let rest = &webpage[found.end()..];
        let start = found.end() + (rest.len() - rest.trim_start().len());
        if !webpage[start..].starts_with('{') || start >= last {
            continue;
        }
        let head_end = webpage[start..]
            .find("</script>")
            .map_or(webpage.len(), |end| start + end);
        let named = MORPH_KEY.find_iter(&webpage[start + 1..]).any(|key| {
            start + 1 + key.start() > start + 1
                && start + 1 + key.start() < head_end
                && start + 1 + key.end() <= last
        });
        if !named {
            continue;
        }
        return serde_json::Deserializer::from_str(&crate::json::replace_lone_surrogates(
            &webpage[start..=last],
        ))
        .into_iter::<Value>()
        .next()
        .and_then(std::result::Result::ok);
    }
    None
}

/// The Morph payload's lead media or article videos.
fn morph(page: &Page<'_>, morph_payload: &Value) -> Result<Option<ExtractResult>> {
    let ctx = &page.ctx;
    let components = children(path(morph_payload, &["body", "components"]));
    for lead_media in components
        .into_iter()
        .filter_map(|component| path(component, &["props", "leadMedia"]))
        .filter(|media| media.is_object())
    {
        let Some(programme_id) = ["vpid", "playablePid"]
            .into_iter()
            .find_map(|key| text(path(lead_media, &["identifiers", key])))
        else {
            continue;
        };
        if ["rawDuration", "formattedDuration", "spokenDuration"]
            .into_iter()
            .any(|key| !parse_duration(path(lead_media, &["duration", key])).is_null())
        {
            return Err(unsupported("Morph lead media durations (a list upstream)"));
        }
        let (formats, subtitles) = ctx.media_selector(&programme_id)?;
        let title = nonempty(lead_media.get("title")).or_else(|| page.og_title());
        let mut fields: Fields = vec![("id", programme_id.into()), ("title", opt_str(title))];
        for (key, source) in [
            ("description", "summary"),
            ("uploader", "masterBrand"),
            ("uploader_id", "mid"),
        ] {
            if let Some(value) = text(lead_media.get(source)) {
                fields.push((key, value.into()));
            }
        }
        fields.extend([("formats", Value::Null), ("subtitles", Value::Null)]);
        return Ok(Some(ExtractResult::Video(video(
            ctx,
            fields,
            formats,
            Some(subtitles),
        ))));
    }

    let body = match path(morph_payload, &["body", "content", "article", "body"]) {
        Some(Value::String(body)) => parse_json_optional(BBC_NAME, body, &page.playlist_id),
        None => {
            return Err(Error::Reported(
                "the JSON object must be str, bytes or bytearray, not NoneType".into(),
            ));
        }
        Some(_) => return Err(unsupported("Morph article bodies that are not strings")),
    };
    let timestamp = parse_iso8601(path(
        morph_payload,
        &["body", "content", "article", "dateTimeInfo", "dateTime"],
    ));
    let mut entries = Vec::new();
    for video_data in children(body.as_ref())
        .into_iter()
        .filter(|value| path(value, &["videoData", "pid"]).is_some_and(truthy))
        .filter_map(|value| value.get("videoData"))
    {
        let thumbnail = ["iChefImage", "image"]
            .into_iter()
            .find_map(|key| url_or_none(video_data.get(key)));
        let title = ["title", "caption"]
            .into_iter()
            .find_map(|key| text(video_data.get(key)));
        let duration = parse_duration(video_data.get("duration"));
        let (video_id, entry) = match video_data.get("vpid").filter(|vpid| truthy(vpid)) {
            Some(vpid) => {
                let video_id = py_str(vpid);
                let (formats, subtitles) = page.ctx.media_selector(&video_id)?;
                let mut fields: Fields = vec![
                    ("id", video_id.clone().into()),
                    ("formats", Value::Null),
                    ("subtitles", Value::Null),
                    ("timestamp", opt_int(timestamp)),
                ];
                for (key, value) in [
                    ("thumbnail", opt_str(thumbnail)),
                    ("title", opt_str(title)),
                    ("duration", duration),
                ] {
                    if !value.is_null() {
                        fields.push((key, value));
                    }
                }
                (video_id, video(ctx, fields, formats, Some(subtitles)))
            }
            None => {
                let video_id = py_str(video_data.get("pid").unwrap_or(&Value::Null));
                let mut fields = Map::new();
                fields.insert("timestamp".into(), opt_int(timestamp));
                for (key, value) in [
                    ("thumbnail", opt_str(thumbnail)),
                    ("title", opt_str(title)),
                    ("duration", duration),
                ] {
                    if !value.is_null() {
                        fields.insert(key.into(), value);
                    }
                }
                let entry = url_entry(
                    &format!("https://www.bbc.co.uk/programmes/{video_id}"),
                    Some(&video_id),
                    Some(fields),
                )?;
                (video_id, entry)
            }
        };
        if video_data.get("isLead").is_some_and(truthy) {
            // `_yes_playlist(playlist_id, video_id)`.
            if super::no_playlist() {
                crate::screen::screen(&format!(
                    "[{BBC_NAME}] Downloading just the video {video_id} because of --no-playlist"
                ));
                return Ok(Some(single(page, entry)));
            }
            crate::screen::screen(&format!(
                "[{BBC_NAME}] Downloading playlist {} - add --no-playlist to download just the video {video_id}",
                page.playlist_id
            ));
        }
        entries.push(entry);
    }
    if entries.is_empty() {
        return Ok(None);
    }
    let title = text(path(
        morph_payload,
        &["body", "content", "article", "headline"],
    ))
    .or_else(|| page.playlist_title.clone());
    Ok(Some(playlist_result(
        ctx,
        &page.playlist_id,
        title,
        page.playlist_description.clone(),
        entries,
    )))
}

/// A playlist entry returned as the result: a video, or a
/// `url_transparent` result followed with its fields.
fn single(page: &Page<'_>, entry: VideoInfo) -> ExtractResult {
    match entry.extra.get(crate::TRANSPARENT).cloned() {
        Some(marker) => {
            let mut follow =
                super::direct::build_at(page.ctx.url.clone(), page.ctx.url, String::new());
            follow.formats.clear();
            follow.direct = None;
            follow.extra.insert(
                crate::FOLLOW_TRANSPARENT.into(),
                json!({"url": marker["url"], "fields": marker["fields"]}),
            );
            ExtractResult::Video(follow)
        }
        None => ExtractResult::Video(entry),
    }
}

/// The `__PRELOADED_STATE__` programme or article asset.
fn preloaded(page: &Page<'_>, preload_state: &Value) -> Result<Option<ExtractResult>> {
    let ctx = &page.ctx;
    let current = path(preload_state, &["programmes", "current"]).filter(|value| value.is_object());
    if let Some(current) = current
        && let Some(programme_id) = text(current.get("id"))
        && current.get("type").and_then(Value::as_str) == Some("playable_item")
    {
        let title = ["tertiary", "secondary"]
            .into_iter()
            .find_map(|key| text(path(current, &["titles", key])))
            .filter(|title| !title.is_empty())
            .or_else(|| page.playlist_title.clone());
        let (formats, subtitles) = ctx.media_selector(&programme_id)?;
        let mut fields: Fields = vec![
            ("id", programme_id.into()),
            ("title", opt_str(title)),
            ("formats", Value::Null),
        ];
        let description = ["long", "medium", "short"]
            .into_iter()
            .find_map(|key| text(path(current, &["synopses", key])));
        let thumbnail = text(current.get("image_url"))
            .and_then(|image| super::json_ld::url_or_none(&image.replace("{recipe}", "raw")));
        let timestamp = [
            path(current, &["availability", "from"]),
            path(current, &["release", "date"]),
        ]
        .into_iter()
        .find_map(parse_iso8601);
        for (key, value) in [
            ("description", description.map(Value::from)),
            ("thumbnail", thumbnail.map(Value::from)),
            (
                "duration",
                int_or_none(path(current, &["duration", "value"])).map(Value::from),
            ),
            (
                "uploader",
                text(path(current, &["network", "short_title"])).map(Value::from),
            ),
            (
                "uploader_id",
                text(path(current, &["network", "id"])).map(Value::from),
            ),
            ("timestamp", timestamp.map(Value::from)),
            (
                "series",
                text(path(current, &["titles", "primary"])).map(Value::from),
            ),
        ] {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        fields.push(("subtitles", Value::Null));
        let mut chapters = Vec::new();
        for track in children(path(preload_state, &["tracklist", "tracks"])) {
            // `lambda _, v: float(v['offset']['start'])`: a nonzero start.
            let start = float_or_none(path(track, &["offset", "start"]), 1.0);
            if !start.is_some_and(|start| start != 0.0) {
                continue;
            }
            let mut chapter = Map::new();
            if let Some(titles) = track.get("titles").filter(|titles| titles.is_object()) {
                let title = ["primary", "secondary", "tertiary"]
                    .into_iter()
                    .filter_map(|key| titles.get(key).filter(|value| truthy(value)))
                    .map(py_str)
                    .collect::<Vec<_>>()
                    .join(" - ");
                chapter.insert("title".into(), title.into());
            }
            if let Some(start) = start {
                chapter.insert("start_time".into(), json!(start));
            }
            if let Some(end) = float_or_none(path(track, &["offset", "end"]), 1.0) {
                chapter.insert("end_time".into(), json!(end));
            }
            if !chapter.is_empty() {
                chapters.push(Value::Object(chapter));
            }
        }
        fields.push(("chapters", Value::Array(chapters)));
        let mut info = video(ctx, fields, formats, Some(subtitles));
        add_shape(&mut info, ";chapter=title,start_time,end_time");
        return Ok(Some(ExtractResult::Video(info)));
    }

    // `PWA_PRELOADED_STATE` with an article video asset.
    let articles = path(preload_state, &["entities", "articles"]);
    let asset_id = match articles {
        Some(Value::Object(articles)) => articles
            .iter()
            .filter(|(key, _)| key.rsplit('/').next() == Some(page.playlist_id.as_str()))
            .find_map(|(_, article)| text(path(article, &["assetVideo", "0"]))),
        _ => None,
    };
    let Some(asset_id) = asset_id else {
        return Ok(None);
    };
    let video_entity = path(preload_state, &["entities", "videos", &asset_id]);
    let Some(video_id) = video_entity.and_then(|video| text(video.get("vpid"))) else {
        return Ok(Some(super::follow_url(
            ctx.url,
            format!("https://www.bbc.co.uk/programmes/{asset_id}"),
        )));
    };
    let article = children(articles).into_iter().find(|article| {
        path(article, &["assetVideo", "0"]).and_then(Value::as_str) == Some(asset_id.as_str())
    });
    let (formats, subtitles) = ctx.media_selector(&video_id)?;
    let video_entity = video_entity.cloned().unwrap_or(Value::Null);
    let mut fields: Fields = vec![("id", video_id.into())];
    let image_url = |image_id: &Value| {
        let image_id = image_id.as_str()?;
        text(path(
            preload_state,
            &["entities", "images", image_id, "url"],
        ))
        .and_then(|url| super::json_ld::url_or_none(&url.replace("$recipe", "raw")))
    };
    for (key, value) in [
        ("title", text(video_entity.get("title")).map(Value::from)),
        (
            "description",
            ["synopsisLong", "synopsisMedium", "synopsisShort"]
                .into_iter()
                .find_map(|key| text(video_entity.get(key)))
                .map(Value::from),
        ),
        (
            "thumbnail",
            video_entity
                .as_array()
                .and_then(|items| items.first())
                .and_then(image_url)
                .map(Value::from),
        ),
        (
            "duration",
            int_or_none(video_entity.get("duration")).map(Value::from),
        ),
    ] {
        if let Some(value) = value {
            fields.push((key, value));
        }
    }
    fields.extend([
        ("formats", Value::Null),
        ("subtitles", Value::Null),
        (
            "timestamp",
            opt_int(parse_iso8601(
                article.and_then(|article| article.get("displayDate")),
            )),
        ),
    ]);
    Ok(Some(ExtractResult::Video(video(
        ctx,
        fields,
        formats,
        Some(subtitles),
    ))))
}

/// BBC Three's current clip or playlist.
fn bbc_three(page: &Page<'_>, payload: &Value) -> Result<Option<ExtractResult>> {
    let ctx = &page.ctx;
    let clip = get(payload, "currentClip")
        .filter(|clip| truthy(clip))
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let (Some(clip_vpid), Some(clip_title)) = (
        clip.get("vpid").filter(|vpid| truthy(vpid)),
        clip.get("title").filter(|title| truthy(title)),
    ) {
        let clip_vpid = py_str(clip_vpid);
        let (formats, subtitles) = ctx.media_selector(&clip_vpid)?;
        let thumbnail = ["poster", "imageUrl"]
            .into_iter()
            .filter_map(|key| clip.get(key))
            .find(|value| truthy(value))
            .cloned()
            .unwrap_or(Value::Null);
        return Ok(Some(ExtractResult::Video(video(
            ctx,
            vec![
                ("id", clip_vpid.into()),
                ("title", clip_title.clone()),
                ("thumbnail", thumbnail),
                (
                    "description",
                    clip.get("description").cloned().unwrap_or(Value::Null),
                ),
                ("duration", parse_duration(clip.get("duration"))),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
            Some(subtitles),
        ))));
    }
    let Some(bbc3_playlist) =
        path(payload, &["content", "bbcMedia", "playlist"]).filter(|value| value.is_object())
    else {
        return Ok(None);
    };
    let playlist_title =
        nonempty(bbc3_playlist.get("title")).or_else(|| page.playlist_title.clone());
    let thumbnail = bbc3_playlist
        .get("holdingImageURL")
        .cloned()
        .unwrap_or(Value::Null);
    let items = bbc3_playlist
        .get("items")
        .ok_or_else(|| key_error(&page.playlist_id, "items"))?;
    let mut entries = Vec::new();
    for item in children(Some(items)) {
        let Some(programme_id) = item.get("versionID").filter(|id| truthy(id)) else {
            continue;
        };
        let programme_id = py_str(programme_id);
        let (formats, subtitles) = ctx.media_selector(&programme_id)?;
        entries.push(video(
            ctx,
            vec![
                ("id", programme_id.into()),
                ("title", opt_str(playlist_title.clone())),
                ("thumbnail", thumbnail.clone()),
                ("timestamp", opt_int(page.timestamp)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
            Some(subtitles),
        ));
    }
    Ok(Some(playlist_result(
        ctx,
        &page.playlist_id,
        playlist_title,
        page.playlist_description.clone(),
        entries,
    )))
}

/// The `__INITIAL_DATA__` article videos, else its media experience and
/// article media (an empty playlist without any).
fn initial_data_result(page: &Page<'_>, initial_data: &Value) -> Result<ExtractResult> {
    let mut entries = Vec::new();
    for video_data in of_type(
        path(initial_data, &["stores", "article", "articleBodyContent"]),
        &["video"],
    ) {
        let model = nested_model(video_data, "aresMedia", "aresMediaMetadata");
        if let Some((entry, _)) = page.parse_model(model)? {
            entries.push(entry);
        }
    }
    if !entries.is_empty() {
        return Ok(page.playlist(entries));
    }
    for resp in children(initial_data.get("data"))
        .into_iter()
        .filter(|resp| resp.get("name").is_some_and(truthy))
    {
        match resp.get("name").and_then(Value::as_str) {
            Some("media-experience") => {
                let media = path(resp, &["data", "initialItem", "mediaItem"])
                    .filter(|media| media.is_object());
                parse_media(page, media, &mut entries)?;
            }
            Some("article") => {
                let data = resp.get("data");
                let blocks = [
                    data.and_then(|data| data.get("blocks")),
                    data.and_then(|data| path(data, &["content", "model", "blocks"])),
                ];
                for block in blocks
                    .into_iter()
                    .flat_map(|blocks| of_type(blocks, &["media", "video"]))
                {
                    let model = block.get("model").filter(|model| model.is_object());
                    parse_media(page, model, &mut entries)?;
                }
            }
            _ => {}
        }
    }
    Ok(page.playlist(entries))
}

/// `parse_media(media)`: a media block's items.
fn parse_media(page: &Page<'_>, media: Option<&Value>, entries: &mut Vec<VideoInfo>) -> Result<()> {
    let Some(media) = media.filter(|media| truthy(media)) else {
        return Ok(());
    };
    let items = match path(media, &["media", "items"]) {
        Some(Value::Array(items)) => items.as_slice(),
        _ => &[],
    };
    for item in items {
        if !item.is_object() {
            return Err(unsupported("media items that are not objects"));
        }
        let (Some(item_id), Some(item_title)) = (
            item.get("id").filter(|id| truthy(id)),
            item.get("title").filter(|title| truthy(title)),
        ) else {
            continue;
        };
        let item_id = py_str(item_id);
        let (formats, subtitles) = page.ctx.media_selector(&item_id)?;
        let summary: Vec<String> = match path(media, &["summary", "blocks"]) {
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter_map(|block| nonempty(path(block, &["model", "text"])))
                .collect(),
            _ => Vec::new(),
        };
        let description = (!summary.is_empty()).then(|| summary.join("\n\n").trim().to_owned());
        let item_time = match path(media, &["metadata", "items"]) {
            Some(Value::Array(metas)) => metas
                .iter()
                .find(|meta| meta.get("label").and_then(Value::as_str) == Some("Published"))
                .and_then(|meta| meta.get("timestamp"))
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_timestamp(date, true)),
            _ => None,
        };
        entries.push(video(
            &page.ctx,
            vec![
                ("id", item_id.into()),
                ("title", item_title.clone()),
                (
                    "thumbnail",
                    item.get("holdingImageUrl").cloned().unwrap_or(Value::Null),
                ),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("timestamp", opt_int(item_time)),
                ("description", opt_str(description)),
                ("duration", opt_int(int_or_none(item.get("duration")))),
            ],
            formats,
            Some(subtitles),
        ));
    }
    Ok(())
}

/// The `SIMORGH_DATA` video and legacy media blocks.
fn simorgh_entries(page: &Page<'_>, simorgh: &Value) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    for video_data in of_type(
        path(simorgh, &["pageData", "content", "model", "blocks"]),
        &["video", "legacyMedia"],
    ) {
        let model = nested_model(video_data, "aresMedia", "aresMediaMetadata");
        let mut done = false;
        let entry = if video_data.get("type").and_then(Value::as_str) == Some("video") {
            page.parse_model(model)?.map(|(entry, _)| entry)
        } else {
            // legacyMedia: no duration or subtitles.
            let block_id = model.and_then(|model| text(model.get("blockId")));
            let media_data = path(simorgh, &["pageData", "promo", "media"])
                .filter(|media| media.get("id").and_then(Value::as_str) == block_id.as_deref());
            let mut formats = Vec::new();
            for source in children(media_data.and_then(|media| media.get("playlist"))) {
                let Some(source_url) = url_or_none(source.get("url")) else {
                    continue;
                };
                let mut fields: Fields = vec![("url", source_url.into())];
                if let Some(ext) = text(source.get("format")) {
                    fields.push(("ext", ext.into()));
                }
                let tbr = int_scaled(source.get("bitrate"));
                if let Some(tbr) = tbr {
                    fields.push(("tbr", tbr.into()));
                }
                let mut format = dict_format(fields, BBC_NAME, &page.playlist_id)?;
                if tbr.is_some() {
                    crate::info_order::mark_int_rates(&mut format);
                }
                formats.push(format);
            }
            if formats.is_empty() {
                None
            } else {
                done = true;
                let model = model.cloned().unwrap_or(Value::Null);
                let mut fields: Fields = vec![
                    ("id", opt_str(block_id)),
                    ("display_id", page.playlist_id.clone().into()),
                    ("formats", Value::Null),
                    (
                        "description",
                        opt_str(text(path(simorgh, &["pageData", "promo", "summary"]))),
                    ),
                ];
                if let Some(title) = text(model.get("title")) {
                    fields.push(("title", title.into()));
                }
                if let Some(image) = text(model.get("imageUrl"))
                    && let Ok(thumbnail) = page.ctx.url.join(&image.replace("$recipe", "raw"))
                {
                    fields.push(("thumbnail", thumbnail.to_string().into()));
                }
                // A later `description` keeps the key's first position.
                if let Some(description) = ["long", "medium", "short"]
                    .into_iter()
                    .find_map(|key| text(path(&model, &["synopses", key])))
                    && let Some(slot) = fields.iter_mut().find(|(key, _)| *key == "description")
                {
                    slot.1 = description.into();
                }
                if let Some(timestamp) = int_scaled(model.get("firstPublished")) {
                    fields.push(("timestamp", timestamp.into()));
                }
                Some(video(&page.ctx, fields, formats, None))
            }
        };
        entries.extend(entry);
        if done {
            break;
        }
    }
    Ok(entries)
}

/// The `data-media-meta`, `mediaAssetPage` or `vxp-playlist-data` media.
fn media_metas(page: &Page<'_>) -> Result<ExtractResult> {
    let webpage = page.webpage.as_str();
    let playlist_id = page.playlist_id.as_str();
    let mut medias: Vec<Value> = MEDIA_META
        .captures_iter(webpage)
        .filter_map(|found| parse_json_optional(BBC_NAME, &found[1], playlist_id))
        .filter(truthy)
        .collect();
    if medias.is_empty()
        && let Some(found) = MEDIA_ASSET.captures(webpage)
    {
        let Some(asset_page) = parse_json_optional(BBC_NAME, &found[1], playlist_id) else {
            return Err(Error::Reported(
                "'NoneType' object has no attribute 'get'".into(),
            ));
        };
        for video in children(asset_page.get("videos")) {
            medias.extend(children(Some(video)).into_iter().cloned());
        }
    }
    if medias.is_empty() {
        let Some(found) = VXP.captures(webpage) else {
            return Err(Error::Reported(format!(
                "[{BBC_NAME}] {playlist_id}: Unable to extract playlist data; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let vxp_playlist = parse_json_fatal(&found[1], playlist_id)
            .map_err(|error| Error::Reported(format!("[{BBC_NAME}] {error}")))?;
        let mut playlist_medias = Vec::new();
        for item in children(Some(&vxp_playlist)) {
            let Some(media) = item.get("media").filter(|media| truthy(media)) else {
                continue;
            };
            playlist_medias.push(media.clone());
            if path(item, &["advert", "assetId"]).and_then(Value::as_str) == Some(playlist_id) {
                medias = vec![media.clone()];
                break;
            }
        }
        if medias.is_empty() {
            medias = playlist_medias;
        }
    }

    let count = medias.len();
    let mut entries = Vec::new();
    for (num, media_meta) in medias.iter().enumerate() {
        let num = num + 1;
        let (formats, subtitles, subtitles_list) = media_meta_formats(page, media_meta)?;
        // `self.get_param('ignore_no_formats')` is not a parameter upstream
        // sets, so formatless media are always skipped.
        if formats.is_empty() {
            continue;
        }
        let video_id = nonempty(media_meta.get("externalId")).unwrap_or_else(|| {
            if count == 1 {
                playlist_id.to_owned()
            } else {
                format!("{playlist_id}-{num}")
            }
        });
        let title = nonempty(media_meta.get("caption")).or_else(|| {
            if count == 1 {
                page.playlist_title.clone()
            } else {
                Some(format!(
                    "{} - Video {num}",
                    page.playlist_title.as_deref().unwrap_or("None")
                ))
            }
        });
        let duration = int_or_none(media_meta.get("durationInSeconds"))
            .map(Value::from)
            .filter(|duration| duration.as_i64() != Some(0))
            .unwrap_or_else(|| parse_duration(media_meta.get("duration")));
        let mut images: Vec<&Value> = children(media_meta.get("images"))
            .into_iter()
            .flat_map(|image| children(Some(image)))
            .collect();
        if let Some(image) = media_meta.get("image") {
            images.push(image);
        }
        let thumbnails: Vec<Value> = images
            .into_iter()
            .map(|image| {
                json!({
                    "url": image.get("href").cloned().unwrap_or(Value::Null),
                    "width": int_value(image.get("width")),
                    "height": int_value(image.get("height")),
                })
            })
            .collect();
        let mut entry = video(
            &page.ctx,
            vec![
                ("id", video_id.into()),
                ("title", opt_str(title)),
                ("thumbnails", Value::Array(thumbnails)),
                ("duration", duration),
                ("timestamp", opt_int(page.timestamp)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
            subtitles,
        );
        add_shape(
            &mut entry,
            if subtitles_list {
                ";thumbnail!=url,width,height;subtitles=list"
            } else {
                ";thumbnail!=url,width,height"
            },
        );
        entries.push(entry);
    }
    Ok(page.playlist(entries))
}

/// `_extract_from_media_meta(media_meta, video_id)`: the formats, the
/// subtitles and whether they are upstream's empty list.
fn media_meta_formats(
    page: &Page<'_>,
    media_meta: &Value,
) -> Result<(Vec<VideoFormat>, Option<Subtitles>, bool)> {
    if let Some(Value::Object(source_files)) =
        media_meta.get("sourceFiles").filter(|files| truthy(files))
    {
        let mut formats = Vec::new();
        for (format_id, file) in source_files {
            let Some(file_url) = file.get("url").filter(|url| truthy(url)) else {
                continue;
            };
            formats.push(dict_format(
                vec![
                    ("url", file_url.clone()),
                    ("format_id", format_id.clone().into()),
                    ("ext", file.get("encoding").cloned().unwrap_or(Value::Null)),
                    (
                        "tbr",
                        float_or_none(file.get("bitrate"), 1000.0)
                            .map_or(Value::Null, |tbr| json!(tbr)),
                    ),
                    ("filesize", int_value(file.get("filesize"))),
                ],
                BBC_NAME,
                &page.playlist_id,
            )?);
        }
        return Ok((formats, None, true));
    }
    if let Some(programme_id) = media_meta.get("externalId").filter(|id| truthy(id)) {
        let (formats, subtitles) = page.ctx.media_selector(&py_str(programme_id))?;
        return Ok((formats, Some(subtitles), false));
    }
    if let Some(href) = nonempty(media_meta.get("href")) {
        // `_download_legacy_playlist_url(href)` without a video ID.
        let xml = page
            .ctx
            .download_xml(&href, "", "Downloading legacy playlist XML")?;
        let programme = page
            .ctx
            .extract_from_legacy_playlist(&xml, &page.playlist_id)?;
        return Ok((programme.formats, programme.subtitles, false));
    }
    Ok((Vec::new(), None, true))
}

/// Appends `parts` to the info's `dict;KEYS` shape.
fn add_shape(info: &mut VideoInfo, parts: &str) {
    if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
        shape.push_str(parts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url_and_suitable() {
        for url in [
            "http://www.bbc.co.uk/programmes/b039g8p7",
            "http://www.bbc.co.uk/iplayer/episode/b054fn09/ad/natural-world-20152016-2-super-powered-owls",
            "http://www.bbc.co.uk/music/clips/p022h44b",
            "http://www.bbc.co.uk/radio/player/p03cchwf",
            "https://www.bbc.co.uk/events/eppp5c/play/boclqd/p0pb2tlv",
            "https://www.bbc.co.uk/programmes/w172w4dww1jqt5s",
            "http://www.bbc.co.uk/iplayer/playlist/p01dvks4",
        ] {
            assert!(couk_suitable(url), "{url}");
            assert!(!bbc_suitable(url), "{url}");
        }
        for url in [
            "http://www.bbc.co.uk/programmes/b00mfl7n/clips",
            "http://www.bbc.co.uk/programmes/articles/3jNQLTMrPlYGTBn0WV6M2MS/x",
            "https://www.bbc.co.uk/iplayer/episodes/b094m5t9/doctor-foster",
            "https://www.bbc.co.uk/iplayer/group/p081d7j7",
        ] {
            assert!(!couk_suitable(url), "{url}");
            assert!(!bbc_suitable(url), "{url}");
        }
        for url in [
            "http://www.bbc.com/news/world-europe-32668511",
            "https://www.bbc.co.uk/bbcthree/clip/73d0bbd0-abc3-4cea-b3c0-cdae21905eb1",
            "https://www.bbcnewsd73hkzno2ini43t4gblxvycyac5aw4gnv7t2rccijh7745uqd.onion/news/av/world-europe-63208576",
            "https://www.bbc.co.uk/sounds/play/w3ct5rgx",
        ] {
            assert!(bbc_suitable(url), "{url}");
        }
        assert_eq!(
            bbc_temp_id("http://www.bbc.com/news/world-europe-32668511?x=1").as_deref(),
            Some("world-europe-32668511")
        );
    }

    #[test]
    fn morph_payloads_need_lead_media_before_their_script_end() {
        let page = r#"<script>Morph.setPayload('/a', {"x": 1});</script>
<script>Morph.setPayload('/b', {"body": {"components": [{"props": {"leadMedia": {}}}]}});</script>"#;
        let payload = morph_payload(page).unwrap();
        assert!(payload["body"]["components"][0]["props"]["leadMedia"].is_object());
        assert!(morph_payload(r#"Morph.setPayload('/a', {"x": 1});"#).is_none());
    }

    #[test]
    fn page_titles_drop_the_bbc_suffix() {
        assert_eq!(
            TITLE_SUFFIX.replace("Pluto probe - BBC News", "$1").trim(),
            "Pluto probe"
        );
        assert_eq!(TITLE_SUFFIX.replace("Plain title", "$1"), "Plain title");
    }
}
