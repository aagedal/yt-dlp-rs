//! Upstream `ZDFIE` and `ZDFChannelIE` (`yt_dlp/extractor/zdf.py`, with
//! `ZDFBaseIE`): zdf.de video/play pages, legacy `.html` URLs and the
//! zdfheute.de/logo.de sister sites through the GraphQL `VideoByCanonical`
//! query (an API token from the Android app's token endpoint, cached like
//! upstream), the PTMD stream documents (progressive MP4/WebM with
//! `parse_codecs`, HLS masters, German Sign Language variants, language and
//! audio-description preferences, deduplicated captions), the deprecated
//! `mediathekV2/document` fallback, and a smart collection's seasons as a
//! lazily paged playlist of `zdf` URL results (or its featured video).
//!
//! Not supported natively: PTMD templates, aspect ratios, track classes or
//! languages of unexpected types, and API answers whose `data` is not an
//! object (explicit errors).
use super::{
    Extractor, dict_format, dict_info, download_webpage_handle, extract_m3u8_master_optional,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, Subtitles, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct Zdf;
pub struct ZdfChannel;

const NAME: &str = "zdf";
const KEY: &str = "ZDF";
const CHANNEL_NAME: &str = "zdf:channel";
const CHANNEL_KEY: &str = "ZDFChannel";
const PLAYER_ID: &str = "android_native_6";
const TOKEN_URL: &str = "https://zdf-prod-futura.zdf.de/mediathekV2/token";
const GRAPHQL_URL: &str = "https://api.zdf.de/graphql";
const COLLECTION_HASH: &str = "cb49420e133bd668ad895a8cea0e65cba6aa11ac1cacb02341ff5cf32a17cd02";
const SEASON_HASH: &str = "9412a0f4ac55dc37d46975d461ec64bfd14380d815df843a1492348f77b5c99a";
const PAGE_SIZE: i64 = 24;

static VIDEO_URLS: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        r"^https?://(?:www\.)?zdf\.de/(?:video|play)/(?:[^/?#]+/)*(?P<id>[^/?#]+)",
        // Legacy redirects from before the 2025-03 redesign.
        r"^https?://(?:www\.)?zdf\.de/(?:[^/?#]+/)*(?P<id>[^/?#]+)\.html",
        // Sister sites.
        r"^https?://(?:www\.)?(?:zdfheute|logo)\.de/(?:[^/?#]+/)*(?P<id>[^/?#]+)\.html",
    ]
    .map(|pattern| Regex::new(pattern).unwrap())
});
const CHANNEL_PATTERN: &str = r"https?://www\.zdf\.de/(?:[^/?#]+/)*(?P<id>[^/?#]+)";
static CHANNEL_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^{CHANNEL_PATTERN}")).unwrap());
/// `_search_regex(_VALID_URL, urlh.url, ...)`: unanchored.
static CHANNEL_SEARCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(CHANNEL_PATTERN).unwrap());
static PTMD_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/vod/ptmd/[^/?#]+/(\w+)").unwrap());
static TBR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_(\d+)k_").unwrap());
static ASPECT_RATIO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<width>\d+):(?P<height>\d+)").unwrap());
static THUMBNAIL_SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?P<width>\d+|auto)[Xx](?P<height>\d+|auto)").unwrap());

const GRAPHQL_QUERY: &str = "
query VideoByCanonical($canonical: String!) {
  videoByCanonical(canonical: $canonical) {
    canonical
    title
    leadParagraph
    editorialDate
    teaser {
      description
      image {
        list
      }
    }
    episodeInfo {
      episodeNumber
      seasonNumber
    }
    smartCollection {
      canonical
      title
    }
    currentMedia {
      nodes {
        ptmdTemplate
        ... on VodMedia {
          duration
          aspectRatio
          streamAnchorTags {
            nodes {
              anchorOffset
              anchorLabel
            }
          }
          vodMediaType
          label
        }
        ... on LiveMedia {
          start
          stop
          encryption
          liveMediaType
          label
        }
        id
      }
    }
  }
}
    ";

/// `ZDFIE._match_valid_url`: the URL's ID.
fn video_id(url: &str) -> Option<String> {
    VIDEO_URLS
        .iter()
        .find_map(|pattern| pattern.captures(url))
        .map(|found| found["id"].to_owned())
}

/// `ZDFIE.suitable`.
pub fn suitable(url: &str) -> bool {
    video_id(url).is_some()
}

/// `ZDFChannelIE.suitable`: not a `ZDFIE` URL.
pub fn channel_suitable(url: &str) -> bool {
    !suitable(url) && CHANNEL_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

pub(crate) fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("ZDF {what} are not supported natively"))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
pub(crate) fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// An `ExtractorError(msg)` raised in `_real_extract`.
fn extractor_error(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{name}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

pub(crate) fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// Python iteration of a `traverse_obj` `...` branch: list items or dict
/// values (`None` dropped).
fn branch(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().filter(|item| !item.is_null()).collect(),
        Some(Value::Object(map)) => map.values().filter(|item| !item.is_null()).collect(),
        _ => Vec::new(),
    }
}

/// A `traverse_obj` path of dict keys.
pub(crate) fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .try_fold(value, |value, key| value.get(*key))
        .filter(|value| !value.is_null())
}

/// `{str}`.
pub(crate) fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

/// `int_or_none`: Python's `int()` of numbers, booleans and integer text.
pub(crate) fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().replace('_', "").parse().ok(),
        _ => None,
    }
}

/// `float_or_none`: Python's `float()` of numbers, booleans and text.
fn float_or_none(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Bool(flag) => Some(f64::from(u8::from(*flag))),
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `url_or_none` of a string.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `(('leadParagraph', ('teaser', 'description')), any, {str})`.
fn description(data: &Value) -> Option<String> {
    [
        path(data, &["leadParagraph"]),
        path(data, &["teaser", "description"]),
    ]
    .into_iter()
    .flatten()
    .find(|value| *value != &Value::Object(Map::new()))
    .and_then(|value| value.as_str().map(str::to_owned))
}

fn timestamp(data: &Value) -> Option<i64> {
    path(data, &["editorialDate"])
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
}

/// `join_nonempty(*values, delim=delim)`.
fn join_nonempty(values: &[Option<&str>], delim: &str) -> String {
    values
        .iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(delim)
}

/// The JSON of an API request: `[NAME] ID: NOTE` (no ID for `None`), the
/// request with `headers=` (a POST with `body`), `ERRNOTE: ...` for HTTP
/// errors.
#[allow(clippy::too_many_arguments)]
pub(crate) fn request_json(
    client: &Client,
    url: &Url,
    name: &'static str,
    id: Option<&str>,
    note: &str,
    errnote: &str,
    headers: &[(&str, &str)],
    body: Option<String>,
) -> Result<Value> {
    use std::io::Read;
    let prefix = match id {
        Some(id) => format!("[{name}] {id}: "),
        None => format!("[{name}] "),
    };
    crate::screen::screen(&format!("{prefix}{note}"));
    let response = match body {
        Some(body) => super::direct::post(client, url, headers, body),
        None => super::direct::request_with_headers(client, url, None, headers),
    };
    let mut response = response.map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::Reported(match id {
            Some(id) => super::request_errnote_text(name, id, errnote, &error),
            None => super::request_errnote_text(name, "", errnote, &error).replacen(
                &format!("[{name}] : "),
                &format!("[{name}] "),
                1,
            ),
        }),
        error => error,
    })?;
    let page_url = response.url().clone();
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    let bytes = super::decode_content(response.headers(), bytes).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id.unwrap_or("None"), None, None);
    crate::screen::page_read(&page_url, &bytes)?;
    let text = super::decode_webpage(&bytes, response.headers());
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!("{prefix}Failed to parse JSON  (caused by {error})"))
    })
}

/// `ZDFBaseIE._token_cache`, shared by the ZDF extractors.
static TOKEN_CACHE: Mutex<Option<Map<String, Value>>> = Mutex::new(None);

/// `ZDFBaseIE._get_api_token`: the cached token unless expired, else a new
/// one from the Android app's endpoint (stored in the cache).
/// `temp_id` is the URL's ID that a `KeyError` carries.
fn api_token(client: &Client, (name, temp_id): (&'static str, &str)) -> Result<String> {
    let mut cache = TOKEN_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let token = cache.get_or_insert_with(Map::new);
    if token.is_empty()
        && let Some(Value::Object(stored)) = crate::cache::load("zdf", "api-token", None)
    {
        token.extend(stored);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    if int_or_none(token.get("expires")).unwrap_or(0) < now {
        let url = Url::parse(TOKEN_URL)?;
        let fresh = request_json(
            client,
            &url,
            name,
            None,
            "Downloading API token",
            "Failed to download API token",
            &[],
            None,
        )?;
        let Value::Object(fresh) = fresh else {
            return Err(unsupported("API token answers that are not objects"));
        };
        token.extend(fresh);
        crate::cache::store("zdf", "api-token", &Value::Object(token.clone()));
    }
    let part = |key: &str| -> Result<String> {
        match token.get(key) {
            None => Err(key_error(temp_id, key)),
            Some(Value::String(text)) => Ok(text.clone()),
            Some(_) => Err(unsupported("API token fields that are not strings")),
        }
    };
    Ok(format!("{} {}", part("type")?, part("token")?))
}

/// Python's `urlencode` value (`quote_plus`).
fn quote_plus(text: &str) -> String {
    const PLUS: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'_')
        .remove(b'.')
        .remove(b'-')
        .remove(b'~')
        .remove(b' ');
    percent_encoding::utf8_percent_encode(text, PLUS)
        .to_string()
        .replace(' ', "+")
}

/// Python's `json.dumps` (ASCII, `, `/`: ` separators).
fn dumps(value: &Value) -> String {
    let bytes = crate::info_json::to_python_json(value).unwrap_or_default();
    String::from_utf8(bytes).unwrap_or_default()
}

/// `ZDFBaseIE._download_graphql(item_id, data_desc, query=..., body=...)`.
fn download_graphql(
    client: &Client,
    (name, temp_id): (&'static str, &str),
    item_id: &str,
    data_desc: &str,
    query: Option<&[(&str, String)]>,
    body: Option<&Value>,
) -> Result<Value> {
    let token = api_token(client, (name, temp_id))?;
    let mut url = GRAPHQL_URL.to_owned();
    if let Some(query) = query {
        let encoded: Vec<String> = query
            .iter()
            .map(|(key, value)| format!("{key}={}", quote_plus(value)))
            .collect();
        url = format!("{url}?{}", encoded.join("&"));
    }
    let url = Url::parse(&url)?;
    let mut headers = vec![
        ("Api-Auth", token.as_str()),
        ("Apollo-Require-Preflight", "True"),
    ];
    if body.is_some() {
        headers.push(("Content-Type", "application/json"));
    }
    request_json(
        client,
        &url,
        name,
        Some(item_id),
        &format!("Downloading {data_desc}"),
        &format!("Failed to download {data_desc}"),
        &headers,
        body.map(dumps),
    )
}

/// `[...]['data'][field]` of a GraphQL answer.
fn graphql_data<'a>(response: &'a Value, field: &str, temp_id: &str) -> Result<&'a Value> {
    let data = match response.get("data") {
        None => return Err(key_error(temp_id, "data")),
        Some(data @ Value::Object(_)) => data,
        Some(_) => return Err(unsupported("GraphQL answers whose data is not an object")),
    };
    data.get(field).ok_or_else(|| key_error(temp_id, field))
}

/// `ZDFBaseIE._extract_subtitles`: the captions' URLs (first seen) by
/// language (`deu` without one).
fn extract_subtitles(captions: &[&Value]) -> Result<Subtitles> {
    let mut seen: Vec<String> = Vec::new();
    let mut subtitles = Subtitles::default();
    for caption in captions {
        let Some(subtitle_url) = url_or_none(caption.get("uri")) else {
            continue;
        };
        if seen.contains(&subtitle_url) {
            continue;
        }
        seen.push(subtitle_url.clone());
        // `caption.get('language') or 'deu'`.
        let language = match caption.get("language") {
            Some(value) if truthy(value) => match value {
                Value::String(text) => text.clone(),
                _ => return Err(unsupported("caption languages that are not strings")),
            },
            _ => "deu".to_owned(),
        };
        let parsed = Url::parse(&subtitle_url)
            .map_err(|_| unsupported("caption URLs that are not absolute"))?;
        let ext = super::direct::determine_ext(&subtitle_url)
            .map_or_else(|| "unknown_video".to_owned(), |ext| ext.to_lowercase());
        let track = crate::SubtitleFormat {
            name: None,
            url: parsed,
            ext,
            protocol: None,
            fragments: Vec::new(),
            extra: Map::new(),
        };
        let key = Some(language);
        match subtitles
            .entries
            .iter_mut()
            .find(|(known, _)| *known == key)
        {
            Some((_, tracks)) => tracks.push(track),
            None => subtitles.entries.push((key, vec![track])),
        }
    }
    Ok(subtitles)
}

/// A PTMD URL and whether it is a German Sign Language variant
/// (`smuggle_url(ptmd_url, {'vod_media_type': 'DGS'})`).
pub(crate) struct PtmdUrl {
    pub(crate) url: String,
    pub(crate) is_dgs: bool,
}

/// `ZDFBaseIE._extract_ptmd`'s result.
pub(crate) struct Ptmd {
    pub(crate) content_id: Option<String>,
    pub(crate) duration: Option<f64>,
    pub(crate) formats: Vec<VideoFormat>,
    pub(crate) subtitles: Subtitles,
}

/// A track's string field (`variant.get(key)`).
fn track_text(variant: &Value, key: &str, what: &str) -> Result<Option<String>> {
    match variant.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(unsupported(&format!("track {what} that are not strings"))),
    }
}

/// `parse_codecs(mime_codec)` as format dict fields in upstream's order:
/// `vcodec`, `acodec`, `dynamic_range` (and `scodec`), just the two codecs
/// for the two-unknown-codecs fallback, or nothing.
fn codec_fields(mime_codec: Option<&Value>) -> Vec<(&'static str, Value)> {
    let text = mime_codec.and_then(Value::as_str);
    let mut warnings = 0;
    let codecs = crate::codecs::parse_codecs(text, &mut |warning| {
        warnings += 1;
        crate::screen::stderr(warning);
    });
    let Some(codecs) = codecs else {
        return Vec::new();
    };
    let listed = text
        .unwrap_or_default()
        .split(',')
        .filter(|codec| !codec.trim().is_empty())
        .count();
    let mut fields = vec![
        ("vcodec", Value::from(codecs.vcodec)),
        ("acodec", Value::from(codecs.acodec)),
    ];
    if warnings == 2 && listed == 2 {
        return fields;
    }
    fields.push((
        "dynamic_range",
        codecs.dynamic_range.map_or(Value::Null, Value::from),
    ));
    if let Some(scodec) = codecs.scodec {
        fields.push(("scodec", scodec.into()));
    }
    fields
}

/// The PTMD format fields upstream adds to every format of a track.
struct TrackInfo<'a> {
    class: Option<&'a str>,
    language: Option<&'a str>,
    is_dgs: bool,
}

impl TrackInfo<'_> {
    /// `(format_id, format_note, preference, language, language_preference)`
    /// for a format with these `format_id`, `vcodec`, `language` and
    /// `format_note`.
    fn fields(
        &self,
        format_id: &str,
        vcodec: Option<&str>,
        language: Option<&str>,
        note: Option<&str>,
    ) -> (String, String, i64, Option<String>, i64) {
        let lowered = language
            .or(self.language)
            .unwrap_or_default()
            .to_lowercase();
        let f_lang = crate::embed_subtitles::short_to_long(&lowered).map(str::to_owned);
        let is_audio_only = vcodec == Some("none");
        let format_id = join_nonempty(&[Some(format_id), self.is_dgs.then_some("dgs")], "-");
        let format_note = join_nonempty(
            &[
                if is_audio_only { None } else { self.class },
                self.is_dgs.then_some("German Sign Language"),
                note,
            ],
            ", ",
        );
        let preference = if self.is_dgs { -2 } else { -1 };
        let is_deu = f_lang.as_deref() == Some("deu");
        let is_main = self.class == Some("main");
        let language_preference = if (is_audio_only && note == Some("Audiodeskription"))
            || (!is_audio_only && self.class == Some("ad"))
        {
            -10
        } else if is_deu && is_main {
            10
        } else if is_deu {
            5
        } else if is_main {
            1
        } else {
            -1
        };
        (
            format_id,
            format_note,
            preference,
            f_lang,
            language_preference,
        )
    }
}

/// `ZDFBaseIE._extract_ptmd(ptmd_urls, video_id, api_token, aspect_ratio)`.
pub(crate) fn extract_ptmd(
    client: &Client,
    name: &'static str,
    ptmd_urls: &[PtmdUrl],
    video_id: &str,
    token: &str,
    aspect_ratio: Option<f64>,
) -> Result<Ptmd> {
    let mut content_id: Option<String> = None;
    let mut duration: Option<f64> = None;
    let mut formats = Vec::new();
    let mut captions: Vec<Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for ptmd_url in ptmd_urls {
        let url = Url::parse(&ptmd_url.url)?;
        let ptmd = request_json(
            client,
            &url,
            name,
            Some(video_id),
            "Downloading PTMD data",
            "Failed to download PTMD data",
            &[("Api-Auth", token)],
            None,
        )?;
        let basename = match ptmd.get("basename") {
            Some(value) if truthy(value) => Some(
                value
                    .as_str()
                    .ok_or_else(|| unsupported("PTMD basenames that are not strings"))?
                    .to_owned(),
            ),
            _ => PTMD_ID
                .captures(&ptmd_url.url)
                .map(|found| found[1].to_owned()),
        };
        // DGS variants only add formats: the old archive ID is the main one.
        if content_id.as_deref().is_none_or(str::is_empty) && !ptmd_url.is_dgs {
            content_id = basename;
        }
        if duration.is_none_or(|duration| duration == 0.0) {
            duration = float_or_none(path(&ptmd, &["attributes", "duration", "value"]))
                .map(|value| value / 1000.0);
        }
        captions.extend(
            branch(ptmd.get("captions"))
                .into_iter()
                .filter(|caption| caption.is_object())
                .cloned(),
        );

        for priority in branch(ptmd.get("priorityList")) {
            for stream in branch(priority.get("formitaeten")) {
                if !stream.is_object() {
                    continue;
                }
                for quality in branch(stream.get("qualities")) {
                    if !quality.is_object() {
                        continue;
                    }
                    let tracks = quality.get("audio").and_then(|audio| audio.get("tracks"));
                    for variant in branch(tracks) {
                        let Some(format_url) = variant
                            .get("uri")
                            .filter(|_| url_or_none(variant.get("uri")).is_some())
                            .and_then(Value::as_str)
                        else {
                            continue;
                        };
                        if seen.iter().any(|known| known == format_url) {
                            continue;
                        }
                        seen.push(format_url.to_owned());
                        let class = track_text(variant, "class", "classes")?;
                        let language = track_text(variant, "language", "languages")?;
                        let track = TrackInfo {
                            class: class.as_deref(),
                            language: language.as_deref(),
                            is_dgs: ptmd_url.is_dgs,
                        };
                        let ext = super::direct::determine_ext(format_url)
                            .unwrap_or_else(|| "unknown_video".into());
                        match ext.as_str() {
                            "m3u8" => {
                                let manifest = Url::parse(format_url)
                                    .map_err(|_| unsupported("relative HLS track URLs"))?;
                                let Some(stream) = extract_m3u8_master_optional(
                                    client,
                                    &manifest,
                                    name,
                                    video_id,
                                    Some("hls"),
                                    false,
                                    false,
                                ) else {
                                    continue;
                                };
                                for mut format in stream.formats {
                                    let (format_id, note, preference, f_lang, lang_pref) = track
                                        .fields(
                                            &format.format_id,
                                            format.vcodec.as_deref(),
                                            format.language.as_deref(),
                                            format.format_note.as_deref(),
                                        );
                                    format.format_id = format_id;
                                    format.format_note = Some(note);
                                    format.extra.insert("preference".into(), preference.into());
                                    format.language = f_lang.clone();
                                    if f_lang.is_none() {
                                        format.extra.insert("language".into(), Value::Null);
                                    }
                                    format
                                        .extra
                                        .insert("language_preference".into(), lang_pref.into());
                                    for key in [
                                        "format_id",
                                        "format_note",
                                        "preference",
                                        "language",
                                        "language_preference",
                                    ] {
                                        crate::info_order::mark_format_update(&mut format, key);
                                    }
                                    formats.push(format);
                                }
                            }
                            "mp4" | "webm" => {
                                let height = int_or_none(quality.get("highestVerticalResolution"));
                                let width = match (aspect_ratio, height) {
                                    (Some(ratio), Some(height)) if ratio != 0.0 && height != 0 => {
                                        Some((ratio * height as f64).round_ties_even() as i64)
                                    }
                                    _ => None,
                                };
                                let codecs = codec_fields(quality.get("mimeCodec"));
                                let vcodec = codecs
                                    .iter()
                                    .find(|(key, _)| *key == "vcodec")
                                    .and_then(|(_, value)| value.as_str().map(str::to_owned));
                                let stream_type = match stream.get("type") {
                                    None | Some(Value::Null) => None,
                                    Some(Value::String(text)) => Some(text.clone()),
                                    Some(_) => {
                                        return Err(unsupported(
                                            "stream types that are not strings",
                                        ));
                                    }
                                };
                                let base_id =
                                    join_nonempty(&[Some("http"), stream_type.as_deref()], "-");
                                let (format_id, note, preference, f_lang, lang_pref) =
                                    track.fields(&base_id, vcodec.as_deref(), None, None);
                                let tbr = TBR
                                    .captures(format_url)
                                    .and_then(|found| found[1].parse::<i64>().ok());
                                let mut fields = vec![("url", Value::from(format_url))];
                                fields.extend(codecs);
                                fields.extend([
                                    ("height", height.map_or(Value::Null, Value::from)),
                                    ("width", width.map_or(Value::Null, Value::from)),
                                    (
                                        "filesize",
                                        int_or_none(variant.get("filesize"))
                                            .map_or(Value::Null, Value::from),
                                    ),
                                    ("format_id", format_id.into()),
                                    ("tbr", tbr.map_or(Value::Null, Value::from)),
                                    ("format_note", note.into()),
                                    ("preference", preference.into()),
                                    ("language", f_lang.map_or(Value::Null, Value::from)),
                                    ("language_preference", lang_pref.into()),
                                ]);
                                let mut format = dict_format(fields, name, video_id)?;
                                if tbr.is_some() {
                                    crate::info_order::mark_int_rates(&mut format);
                                }
                                formats.push(format);
                            }
                            _ => crate::screen::warning(&format!(
                                "[{name}] {video_id}: Skipping unsupported extension \"{ext}\""
                            )),
                        }
                    }
                }
            }
        }
    }
    let captions: Vec<&Value> = captions.iter().collect();
    Ok(Ptmd {
        content_id: content_id.filter(|id| !id.is_empty()),
        duration,
        formats,
        subtitles: extract_subtitles(&captions)?,
    })
}

/// `ZDFBaseIE._extract_thumbnails(source)`: a dict's URL values by key.
pub(crate) fn extract_thumbnails(source: &Value) -> Value {
    let Value::Object(map) = source else {
        return Value::Array(Vec::new());
    };
    let mut thumbnails = Vec::new();
    for (format_id, url) in map {
        let Some(url) = url
            .as_str()
            .filter(|url| super::json_ld::url_or_none(url).is_some())
        else {
            continue;
        };
        let mut thumbnail = Map::new();
        thumbnail.insert("id".into(), format_id.clone().into());
        thumbnail.insert("url".into(), url.into());
        thumbnail.insert(
            "preference".into(),
            i64::from(format_id == "original").into(),
        );
        if let Some(found) = THUMBNAIL_SIZE.captures(format_id) {
            for key in ["width", "height"] {
                if let Ok(number) = found[key].parse::<i64>() {
                    thumbnail.insert(key.into(), number.into());
                }
            }
        }
        thumbnails.push(Value::Object(thumbnail));
    }
    Value::Array(thumbnails)
}

/// `ZDFBaseIE._extract_chapters(data)`: anchors with a truthy offset.
pub(crate) fn extract_chapters(data: Option<&Value>) -> Option<Value> {
    let chapters: Vec<Value> = branch(data)
        .into_iter()
        .filter(|anchor| anchor.get("anchorOffset").is_some_and(truthy))
        .filter_map(|anchor| {
            let mut chapter = Map::new();
            if let Some(start) = float_or_none(anchor.get("anchorOffset")) {
                chapter.insert("start_time".into(), start.into());
            }
            if let Some(Value::String(title)) = anchor.get("anchorLabel") {
                chapter.insert("title".into(), title.clone().into());
            }
            (!chapter.is_empty()).then_some(Value::Object(chapter))
        })
        .collect();
    (!chapters.is_empty()).then_some(Value::Array(chapters))
}

/// `ZDFBaseIE._parse_aspect_ratio`.
pub(crate) fn parse_aspect_ratio(value: Option<&Value>) -> Result<Option<f64>> {
    let Some(Value::String(text)) = value else {
        return Ok(None);
    };
    let Some(found) = ASPECT_RATIO.captures(text) else {
        return Ok(None);
    };
    let width: f64 = found["width"].parse().unwrap_or(0.0);
    let height: f64 = found["height"].parse().unwrap_or(0.0);
    if height == 0.0 {
        return Err(unsupported("aspect ratios with a zero height"));
    }
    Ok(Some(width / height))
}

/// `ZDFIE._extract_ptmd`'s `_old_archive_ids` (`make_archive_id`).
fn archive_ids(ptmd: &Ptmd, video_id: &str) -> Value {
    json!([format!(
        "zdf {}",
        ptmd.content_id.as_deref().unwrap_or(video_id)
    )])
}

/// Extend the info dict's shape (chapter and thumbnail key orders).
pub(crate) fn mark_shape(info: &mut VideoInfo, suffix: &str) {
    if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
        shape.push_str(suffix);
    }
}

/// `ZDFIE._extract_fallback(document_id)`: the deprecated mediathekV2
/// document API.
fn extract_fallback(client: &Client, page: &Url, document_id: &str) -> Result<VideoInfo> {
    let url = Url::parse(&format!(
        "https://zdf-prod-futura.zdf.de/mediathekV2/document/{document_id}"
    ))?;
    let video = request_json(
        client,
        &url,
        NAME,
        Some(document_id),
        "Downloading fallback metadata",
        "Failed to download fallback metadata",
        &[],
        None,
    )?;
    let document = video
        .get("document")
        .ok_or_else(|| key_error(document_id, "document"))?;
    // `(('streamApiUrlAndroid', ('streams', 0, 'streamApiUrlAndroid')),
    // {url_or_none}, any, {require('PTMD URL')})`.
    let first_stream = document
        .get("streams")
        .and_then(Value::as_array)
        .and_then(|streams| streams.first());
    let ptmd_url = [
        document.get("streamApiUrlAndroid"),
        first_stream.and_then(|stream| stream.get("streamApiUrlAndroid")),
    ]
    .into_iter()
    .find_map(url_or_none)
    .ok_or_else(|| extractor_error(NAME, document_id, "Unable to extract PTMD URL"))?;

    let mut thumbnails = Vec::new();
    if let Some(Value::Object(images)) = document.get("teaserBild") {
        for (key, thumbnail) in images {
            let Some(thumbnail_url) = url_or_none(thumbnail.get("url")) else {
                continue;
            };
            thumbnails.push(json!({
                "url": thumbnail_url,
                "id": key,
                "width": int_or_none(thumbnail.get("width")),
                "height": int_or_none(thumbnail.get("height")),
            }));
        }
    }
    let mut fields = vec![("thumbnails", Value::Array(thumbnails))];
    if let Some(title) = text(path(&video, &["document", "titel"])) {
        fields.push(("title", title.into()));
    }
    if let Some(description) = text(path(&video, &["document", "beschreibung"])) {
        fields.push(("description", description.into()));
    }
    let timestamp = [
        path(&video, &["document", "date"]),
        path(&video, &["meta", "editorialDate"]),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .find_map(|date| crate::dates::unified_timestamp(date, false));
    if let Some(timestamp) = timestamp {
        fields.push(("timestamp", timestamp.into()));
    }
    // The document's captions keep the key's position; the PTMD's replace
    // them.
    let captions = branch(path(&video, &["document", "captions"]));
    if !extract_subtitles(&captions)?.is_empty() {
        fields.push(("subtitles", Value::Null));
    }
    let token = api_token(client, (NAME, document_id))?;
    let ptmd = extract_ptmd(
        client,
        NAME,
        &[PtmdUrl {
            url: ptmd_url,
            is_dgs: false,
        }],
        document_id,
        &token,
        None,
    )?;
    fields.push(("duration", ptmd.duration.map_or(Value::Null, Value::from)));
    fields.push(("formats", Value::Null));
    if !fields.iter().any(|(key, _)| *key == "subtitles") {
        fields.push(("subtitles", Value::Null));
    }
    fields.push(("_old_archive_ids", archive_ids(&ptmd, document_id)));
    fields.push(("id", document_id.into()));
    let mut info = dict_info(page, (NAME, KEY), fields, ptmd.formats);
    info.subtitles = ptmd.subtitles;
    mark_shape(&mut info, ";thumbnail!=url,id,width,height");
    Ok(info)
}

impl Extractor for Zdf {
    fn key(&self) -> &'static str {
        KEY
    }

    /// `_GEO_COUNTRIES = ['DE']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["DE"])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(video_id) = video_id(url.as_str()) else {
            return Ok(None);
        };
        announce(NAME, url);
        let body = json!({
            "operationName": "VideoByCanonical",
            "query": GRAPHQL_QUERY,
            "variables": {"canonical": video_id},
        });
        let response = download_graphql(
            client,
            (NAME, &video_id),
            &video_id,
            "video metadata",
            None,
            Some(&body),
        )?;
        let video_data = graphql_data(&response, "videoByCanonical", &video_id)?;
        if !truthy(video_data) {
            return extract_fallback(client, url, &video_id).map(Some);
        }

        let mut aspect_ratio = None;
        let mut is_live = false;
        let mut ptmd_urls = Vec::new();
        let nodes = path(video_data, &["currentMedia", "nodes"]);
        for node in branch(nodes) {
            let Some(template) = node.get("ptmdTemplate").filter(|value| truthy(value)) else {
                continue;
            };
            let Value::String(template) = template else {
                return Err(unsupported("PTMD templates that are not strings"));
            };
            let expanded = template.replace("{playerId}", PLAYER_ID);
            let ptmd_url = Url::parse("https://api.zdf.de")?
                .join(&expanded)
                .map_err(|_| unsupported("PTMD templates that are not URL paths"))?;
            let is_dgs = match node.get("vodMediaType") {
                Some(Value::String(kind)) => kind == "DGS",
                _ => false,
            };
            is_live = node.get("liveMediaType").is_some();
            ptmd_urls.push(PtmdUrl {
                url: ptmd_url.into(),
                is_dgs,
            });
            if aspect_ratio.is_none_or(|ratio| ratio == 0.0) {
                aspect_ratio = parse_aspect_ratio(node.get("aspectRatio"))?;
            }
        }

        let mut fields = Vec::new();
        if let Some(title) = text(path(video_data, &["title"])) {
            fields.push(("title", title.into()));
        }
        if let Some(description) = description(video_data) {
            fields.push(("description", description.into()));
        }
        if let Some(timestamp) = timestamp(video_data) {
            fields.push(("timestamp", timestamp.into()));
        }
        if let Some(list) = path(video_data, &["teaser", "image", "list"]) {
            fields.push(("thumbnails", extract_thumbnails(list)));
        }
        let episode = |key: &str| int_or_none(path(video_data, &["episodeInfo", key]));
        if let Some(number) = episode("episodeNumber") {
            fields.push(("episode_number", number.into()));
        }
        if let Some(number) = episode("seasonNumber") {
            fields.push(("season_number", number.into()));
        }
        if let Some(series) = text(path(video_data, &["smartCollection", "title"])) {
            fields.push(("series", series.into()));
        }
        if let Some(series_id) = text(path(video_data, &["smartCollection", "canonical"])) {
            fields.push(("series_id", series_id.into()));
        }
        let first_node = nodes
            .and_then(Value::as_array)
            .and_then(|nodes| nodes.first());
        if let Some(chapters) =
            extract_chapters(first_node.and_then(|node| path(node, &["streamAnchorTags", "nodes"])))
        {
            fields.push(("chapters", chapters));
        }
        let token = api_token(client, (NAME, &video_id))?;
        let ptmd = extract_ptmd(client, NAME, &ptmd_urls, &video_id, &token, aspect_ratio)?;
        fields.push(("duration", ptmd.duration.map_or(Value::Null, Value::from)));
        fields.push(("formats", Value::Null));
        fields.push(("subtitles", Value::Null));
        fields.push(("_old_archive_ids", archive_ids(&ptmd, &video_id)));
        fields.push(("is_live", is_live.into()));
        fields.push(("id", video_id.clone().into()));
        let mut info = dict_info(url, (NAME, KEY), fields, ptmd.formats);
        info.subtitles = ptmd.subtitles;
        mark_shape(
            &mut info,
            ";chapter=start_time,title;thumbnail=id,url,preference,width,height",
        );
        Ok(Some(info))
    }
}

/// A top-level `url_result(url, ZDFIE, video_id)`, followed.
fn follow(page: &Url, target: &str) -> ExtractResult {
    let mut info = super::direct::build_at(page.clone(), page, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(crate::FOLLOW_URL.into(), target.into());
    ExtractResult::Video(info)
}

/// `url_result(episode['sharingUrl'], ZDFIE, **fields)`: a playlist entry.
fn episode_entry(episode: &Value, target: &str) -> Result<VideoInfo> {
    let mut entry = Map::new();
    if let Some(id) = text(path(episode, &["canonical"])) {
        entry.insert("id".into(), id.into());
    }
    if let Some(title) = text(path(episode, &["teaser", "title"])) {
        entry.insert("title".into(), title.into());
    }
    if let Some(description) = description(episode) {
        entry.insert("description".into(), description.into());
    }
    if let Some(timestamp) = timestamp(episode) {
        entry.insert("timestamp".into(), timestamp.into());
    }
    for (key, field) in [
        ("episode_number", "episodeNumber"),
        ("season_number", "seasonNumber"),
    ] {
        if let Some(number) = int_or_none(path(episode, &["episodeInfo", field])) {
            entry.insert(key.into(), number.into());
        }
    }
    entry.insert("ie_key".into(), KEY.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // `get_temp_id(url)` without an ID.
    if let Some(id) = entry
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| video_id(target))
    {
        info.id = id;
    }
    if let Some(title) = entry.get("title").and_then(Value::as_str) {
        info.title = title.to_owned();
    }
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

/// One `ZDFChannelIE._fetch_page` of the pending seasons (`state`): its
/// entries, then the next page's cursor or the next season.
fn season_page(
    client: &Client,
    state: &mut Map<String, Value>,
    entries: &mut Vec<VideoInfo>,
) -> Result<()> {
    let playlist_id = state["playlist_id"].as_str().unwrap_or_default().to_owned();
    let canonical = state["canonical"].clone();
    let season_index = state["season_index"].as_u64().unwrap_or_default();
    let season_number = state["season_number"].clone();
    let page_number = state["page"].as_u64().unwrap_or(1);
    let mut variables = Map::new();
    variables.insert("seasonIndex".into(), season_index.into());
    variables.insert("canonical".into(), canonical);
    variables.insert("episodesPageSize".into(), PAGE_SIZE.into());
    if !state["cursor"].is_null() {
        variables.insert("episodesAfter".into(), state["cursor"].clone());
    }
    let extensions = json!({"persistedQuery": {"version": 1, "sha256Hash": SEASON_HASH}});
    let query = [
        ("operationName", "seasonByCanonical".to_owned()),
        ("variables", dumps(&Value::Object(variables))),
        ("extensions", dumps(&extensions)),
    ];
    let temp_id = state["temp_id"].as_str().unwrap_or_default().to_owned();
    let response = download_graphql(
        client,
        (CHANNEL_NAME, &temp_id),
        &playlist_id,
        &format!("season {season_number} page {page_number} JSON"),
        Some(&query),
        None,
    )?;
    let page = graphql_data(&response, "smartCollectionByCanonical", &temp_id)?;
    let nodes = branch(page.get("seasons").and_then(|seasons| seasons.get("nodes")));
    for node in &nodes {
        let episodes = node
            .get("episodes")
            .and_then(|episodes| episodes.get("nodes"));
        for episode in branch(episodes) {
            let Some(target) = url_or_none(episode.get("sharingUrl")) else {
                continue;
            };
            let target = episode
                .get("sharingUrl")
                .and_then(Value::as_str)
                .map_or(target, str::to_owned);
            entries.push(episode_entry(episode, &target)?);
        }
    }
    let page_info = nodes
        .last()
        .and_then(|node| path(node, &["episodes", "pageInfo"]))
        .filter(|info| info.is_object());
    let has_next = page_info
        .and_then(|info| info.get("hasNextPage"))
        .is_some_and(truthy);
    let cursor = page_info
        .and_then(|info| info.get("endCursor"))
        .filter(|cursor| truthy(cursor))
        .cloned();
    match cursor.filter(|_| has_next) {
        Some(cursor) => {
            state.insert("cursor".into(), cursor);
            state.insert("page".into(), (page_number + 1).into());
        }
        None => {
            state.insert("cursor".into(), Value::Null);
            state.insert("page".into(), 1.into());
            state.insert("position".into(), (season_index + 1).into());
            state.shift_remove("season_index");
        }
    }
    Ok(())
}

/// `ZDFChannelIE._entries`: the pending seasons' pages until `limit`
/// entries are known; the remaining state, if any.
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
        if limit.is_some_and(|limit| entries.len() >= limit) {
            return Ok(Some(Value::Object(state)));
        }
        if !state.contains_key("season_index") {
            // The next season to fetch (the requested one only).
            let seasons: Vec<i64> = state["seasons"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_i64)
                .collect();
            let requested = state["requested"].as_i64();
            let start = state["position"].as_u64().unwrap_or_default() as usize;
            let Some(index) = (start..seasons.len())
                .find(|index| requested.is_none_or(|number| number == seasons[*index]))
            else {
                return Ok(None);
            };
            state.insert("season_index".into(), index.into());
            state.insert("season_number".into(), seasons[index].into());
        }
        season_page(client, &mut state, entries)?;
    }
}

/// Whether a playlist's pending state is a ZDF channel's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == "zdf"
}

/// `parse_qs(url)['staffel'][-1]` as `int_or_none` (blank values ignored).
fn requested_season(url: &Url) -> Option<i64> {
    url.query_pairs()
        .filter(|(key, value)| key == "staffel" && !value.is_empty())
        .last()
        .and_then(|(_, value)| int_or_none(Some(&Value::from(value.into_owned()))))
}

impl Extractor for ZdfChannel {
    fn key(&self) -> &'static str {
        CHANNEL_KEY
    }

    /// `_GEO_COUNTRIES = ['DE']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["DE"])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !channel_suitable(url.as_str()) {
            return Ok(None);
        }
        let temp_id = CHANNEL_URL
            .captures(url.as_str())
            .map(|found| found["id"].to_owned())
            .unwrap_or_default();
        announce(CHANNEL_NAME, url);
        // Make sure to get the correct ID in case of redirects.
        let (_, final_url) = download_webpage_handle(client, url, CHANNEL_NAME, &temp_id)?;
        let canonical_id = CHANNEL_SEARCH
            .captures(final_url.as_str())
            .map(|found| found["id"].to_owned())
            .ok_or_else(|| {
                extractor_error(CHANNEL_NAME, &temp_id, "Unable to extract channel id")
            })?;
        let season_number = requested_season(url);
        let playlist_id = match season_number {
            Some(number) if number != 0 => format!("{canonical_id}-s{number}"),
            _ => canonical_id.clone(),
        };

        let variables = json!({"canonical": canonical_id, "videoPageSize": 100});
        let extensions = json!({"persistedQuery": {"version": 1, "sha256Hash": COLLECTION_HASH}});
        let query = [
            ("operationName", "GetSmartCollectionByCanonical".to_owned()),
            ("variables", dumps(&variables)),
            ("extensions", dumps(&extensions)),
        ];
        let response = download_graphql(
            client,
            (CHANNEL_NAME, &temp_id),
            &playlist_id,
            "smart collection data",
            Some(&query),
            None,
        )?;
        let collection = graphql_data(&response, "smartCollectionByCanonical", &temp_id)?;
        if !collection.is_object() {
            return Err(unsupported("smart collections that are not objects"));
        }
        let empty = Value::Object(Map::new());
        let video_data = collection
            .get("video")
            .filter(|video| video.is_object())
            .unwrap_or(&empty);
        let season_numbers: Vec<i64> = branch(path(collection, &["seasons", "seasons"]))
            .into_iter()
            .filter_map(|season| int_or_none(season.get("number")))
            .collect();

        // `_yes_playlist(season_numbers and playlist_id, url_or_none(...) and
        // video_data.get('canonical'))`.
        let video_id = match url_or_none(video_data.get("sharingUrl")) {
            Some(_) => match video_data.get("canonical") {
                Some(value) if truthy(value) => Some(
                    value
                        .as_str()
                        .ok_or_else(|| unsupported("video canonicals that are not strings"))?
                        .to_owned(),
                ),
                _ => None,
            },
            None => None,
        };
        let yes_playlist = match (&video_id, season_numbers.is_empty()) {
            (None, _) => true,
            (Some(_), true) => false,
            (Some(video_id), false) => {
                if super::no_playlist() {
                    crate::screen::screen(&format!(
                        "[{CHANNEL_NAME}] Downloading just the video {video_id} because of --no-playlist"
                    ));
                    false
                } else {
                    crate::screen::screen(&format!(
                        "[{CHANNEL_NAME}] Downloading playlist {playlist_id} - add --no-playlist to download just the video {video_id}"
                    ));
                    true
                }
            }
        };
        if !yes_playlist {
            let target = video_data
                .get("sharingUrl")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !suitable(target) {
                return Err(Error::Reported(format!("Unsupported URL: {target}")));
            }
            return Ok(Some(follow(url, target)));
        }

        if let Some(number) = season_number
            && !season_numbers.contains(&number)
        {
            return Err(extractor_error(
                CHANNEL_NAME,
                &temp_id,
                &format!("Season {number} was not found in the collection data"),
            ));
        }

        let collection_title = text(collection.get("title"));
        let season_title = season_number
            .filter(|number| *number != 0)
            .map(|number| format!("Season {number}"));
        let title = join_nonempty(
            &[collection_title.as_deref(), season_title.as_deref()],
            " - ",
        );
        let description = text(collection.get("infoText"));
        let mut keys = vec!["id"];
        if !title.is_empty() {
            keys.push("title");
        }
        let mut extra = Map::new();
        if let Some(description) = description {
            keys.push("description");
            extra.insert("description".into(), description.into());
        }
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
            "kind": "zdf",
            "playlist_id": playlist_id,
            "temp_id": temp_id,
            "canonical": canonical_id,
            "seasons": season_numbers,
            "requested": season_number,
            "position": 0,
            "page": 1,
            "cursor": null,
        });
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title,
            webpage_url: url.clone(),
            extractor: CHANNEL_NAME.into(),
            extractor_key: CHANNEL_KEY.into(),
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
        for (url, id) in [
            (
                "https://www.zdf.de/video/dokus/ein-tag-im-juli---ahrtalflut-2021-movie-100/terra-x-history-ein-tag-im-juli-ahrtalflut-2021-100",
                "terra-x-history-ein-tag-im-juli-ahrtalflut-2021-100",
            ),
            (
                "https://www.zdf.de/play/live-tv/sender/zdf-live-beitrag-100",
                "zdf-live-beitrag-100",
            ),
            (
                "https://www.zdf.de/politik/phoenix-sendungen/die-gesten-der-maechtigen-100.html",
                "die-gesten-der-maechtigen-100",
            ),
            (
                "https://www.zdfheute.de/video/heute-journal/heute-journal-vom-19-dezember-2025-100.html",
                "heute-journal-vom-19-dezember-2025-100",
            ),
            (
                "https://www.logo.de/logo-vom-freitag-19-dezember-2025-102.html",
                "logo-vom-freitag-19-dezember-2025-102",
            ),
        ] {
            assert_eq!(video_id(url).as_deref(), Some(id), "{url}");
            assert!(!channel_suitable(url), "{url}");
        }
        for url in [
            "https://www.zdf.de/sport/das-aktuelle-sportstudio",
            "https://www.zdf.de/magazine/heute-journal-104?staffel=2025",
            "https://www.zdf.de/dokus/ein-tag-im-juli---ahrtalflut-2021-movie-100",
        ] {
            assert!(!suitable(url), "{url}");
            assert!(channel_suitable(url), "{url}");
        }
        assert!(!channel_suitable("https://zdf.de/sport/x"));
        assert!(!suitable("https://www.logo.de/kinder"));
    }

    #[test]
    fn track_fields_follow_upstream_preferences() {
        let main = TrackInfo {
            class: Some("main"),
            language: Some("deu"),
            is_dgs: false,
        };
        assert_eq!(
            main.fields("http-x", Some("avc1"), None, None),
            ("http-x".into(), "main".into(), -1, Some("deu".into()), 10)
        );
        assert_eq!(
            main.fields(
                "hls-aud",
                Some("none"),
                Some("de"),
                Some("Audiodeskription")
            ),
            (
                "hls-aud".into(),
                "Audiodeskription".into(),
                -1,
                Some("deu".into()),
                -10
            )
        );
        let dgs = TrackInfo {
            class: Some("ad"),
            language: None,
            is_dgs: true,
        };
        assert_eq!(
            dgs.fields("hls-1", None, None, None),
            (
                "hls-1-dgs".into(),
                "ad, German Sign Language".into(),
                -2,
                None,
                -10
            )
        );
    }

    #[test]
    fn graphql_values_encode_like_python() {
        assert_eq!(
            dumps(&json!({"canonical": "a", "videoPageSize": 100})),
            r#"{"canonical": "a", "videoPageSize": 100}"#
        );
        assert_eq!(quote_plus(r#"{"a": 1}"#), "%7B%22a%22%3A+1%7D");
    }
}
