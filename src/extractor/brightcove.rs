//! Upstream `BrightcoveNewIE` (`yt_dlp/extractor/brightcove.py`):
//! `players.brightcove.net` player URLs through the player's policy key
//! (`config.json`, else `index.min.js`), the Playback API and
//! `_parse_brightcove_metadata`; plus `_extract_brightcove_urls` for pages
//! embedding the player (followed with the page as the smuggled
//! `referrer`). Playlists, TV Everywhere (Adobe Pass) videos and
//! `BrightcoveLegacyIE` URLs are explicit unsupported errors.
use super::Extractor;
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub struct BrightcoveNew;

pub(crate) const NAME: &str = "brightcove:new";
pub(crate) const KEY: &str = "BrightcoveNew";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://players\.brightcove\.net/(?P<account_id>\d+)/(?P<player_id>[^/]+)_(?P<embed>[^/]+)/index\.html\?.*(?P<content_type>video|playlist)Id=(?P<video_id>\d+|ref:[^&]+)").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`: the pattern has no `id` group.
pub(crate) fn temp_id(_url: &str) -> Option<String> {
    None
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Brightcove {what} are not supported natively"))
}

/// `smuggle_url(url, data)`.
pub(crate) fn smuggle(url: &str, data: &Value) -> String {
    format!(
        "{url}#__youtubedl_smuggle={}",
        super::html5::quote_plus(&crate::output::python_json_dumps(data))
    )
}

/// `unsmuggle_url(url, {})`: the URL before the last `#` and its data.
pub(crate) fn unsmuggle(url: &str) -> (String, Map<String, Value>) {
    if !url.contains("#__youtubedl_smuggle") {
        return (url.to_owned(), Map::new());
    }
    let (bare, fragment) = url.rsplit_once('#').unwrap_or((url, ""));
    let data = url::form_urlencoded::parse(fragment.as_bytes())
        .find(|(key, _)| key == "__youtubedl_smuggle")
        .and_then(|(_, value)| serde_json::from_str::<Value>(&value).ok());
    match data {
        Some(Value::Object(data)) => (bare.to_owned(), data),
        _ => (bare.to_owned(), Map::new()),
    }
}

/// `BrightcoveNewIE._extract_brightcove_urls(ie, webpage)`: player iframes
/// (protocol-relative ones as `https:`), then `<video data-video-id>` tags
/// with a player script (or `data-account`/`data-player`/`data-embed`).
/// Tags without a player script need upstream's `_is_valid_url` request
/// and are not detected.
pub(crate) fn brightcove_urls(webpage: &str) -> Vec<String> {
    static IFRAME: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"<iframe[^>]+src=(?:"((?:https?:)?//players\.brightcove\.net/\d+/[^/]+/index\.html.+?)"|'((?:https?:)?//players\.brightcove\.net/\d+/[^/]+/index\.html.+?)')"#,
        )
        .unwrap()
    });
    static VIDEO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?is)(<video(?:-js)?\s+[^>]*\bdata-video-id\s*=\s*['"]?[^>]+>)(?:.*?(<script[^>]+src=["'](?:https?:)?//players\.brightcove\.net/(\d+)/([^/]+)_([^/]+)/index(?:\.min)?\.js))?"#,
        )
        .unwrap()
    });
    let mut entries = Vec::new();
    for found in IFRAME.captures_iter(webpage) {
        let url = found
            .get(1)
            .or_else(|| found.get(2))
            .map_or("", |url| url.as_str());
        if url.starts_with("http:") || url.starts_with("https:") {
            entries.push(url.to_owned());
        } else {
            entries.push(format!("https:{url}"));
        }
    }
    for found in VIDEO.captures_iter(webpage) {
        let attrs = crate::html_elements::extract_attributes(&found[1]);
        let attr = |name: &str| {
            attrs
                .get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let Some(video_id) = attr("data-video-id") else {
            continue;
        };
        let group = |index: usize| found.get(index).map(|group| group.as_str().to_owned());
        let Some(account_id) = group(3).or_else(|| attr("data-account")) else {
            continue;
        };
        let player_id = group(4)
            .or_else(|| attr("data-player"))
            .unwrap_or_else(|| "default".into());
        let embed = group(5)
            .or_else(|| attr("data-embed"))
            .unwrap_or_else(|| "default".into());
        if found.get(2).is_none() {
            continue;
        }
        entries.push(format!(
            "https://players.brightcove.net/{account_id}/{player_id}_{embed}/index.html?videoId={video_id}"
        ));
    }
    entries
}

/// `_extract_from_webpage`: each player URL smuggled with the page as its
/// `referrer`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    brightcove_urls(webpage)
        .into_iter()
        .filter_map(|found| {
            Url::parse(&smuggle(
                &found,
                &serde_json::json!({ "referrer": url.as_str() }),
            ))
            .ok()
        })
        .collect()
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

fn float_or_none(value: Option<&Value>, scale: f64) -> Option<f64> {
    let number = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    Some(number / scale)
}

fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.into()),
        _ => None,
    }
}

/// `_download_json(url, video_id, fatal=False)`: HTTP errors are warnings.
fn download_json_optional(client: &Client, url: &Url, id: &str) -> Result<Option<Value>> {
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
    let mut response = match super::direct::request(client, url, None) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, response.headers());
    crate::json::from_str(&text)
        .map(Some)
        .map_err(|_| unsupported("player configurations that are not JSON"))
}

/// `extract_policy_key()`: the player's `config.json` `video_cloud`
/// policy key, else `index.min.js`'s `catalog({...})` `policyKey`, else
/// its `policyKey: "..."` (fatal).
fn extract_policy_key(
    client: &Client,
    account_id: &str,
    player_id: &str,
    embed: &str,
    id: &str,
) -> Result<String> {
    let base = format!("https://players.brightcove.net/{account_id}/{player_id}_{embed}/");
    let config = download_json_optional(client, &Url::parse(&format!("{base}config.json"))?, id)?;
    let key = config
        .as_ref()
        .and_then(|config| config.pointer("/video_cloud/policy_key"))
        .filter(|key| truthy(key));
    if let Some(key) = key {
        return key
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| unsupported("policy keys that are not strings"));
    }
    let script = super::download_webpage(
        client,
        &Url::parse(&format!("{base}index.min.js"))?,
        NAME,
        id,
    )?;
    static CATALOG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"catalog\((\{.+?\})\);").unwrap());
    if let Some(found) = CATALOG.captures(&script)
        && let Some(catalog) = crate::js_to_json::parse_js_value(&found[1])
        && let Some(key) = catalog.get("policyKey").filter(|key| truthy(key))
    {
        return key
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| unsupported("policy keys that are not strings"));
    }
    // `policyKey\s*:\s*(["\'])(?P<pk>.+?)\1`: the leftmost quote style.
    static KEYS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
        [
            Regex::new(r#"policyKey\s*:\s*"([^\n]+?)""#).unwrap(),
            Regex::new(r#"policyKey\s*:\s*'([^\n]+?)'"#).unwrap(),
        ]
    });
    KEYS.iter()
        .filter_map(|pattern| pattern.captures(&script))
        .min_by_key(|found| found.get(0).map_or(usize::MAX, |whole| whole.start()))
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] Unable to extract policy key; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

/// The Playback API JSON; 401/403 bodies are upstream's errors, except
/// that `None` asks for a fresh policy key (`INVALID_POLICY_KEY` for a
/// cached key, when `retry_key`).
fn call_api(
    client: &Client,
    api: &Url,
    id: &str,
    headers: &[(&str, &str)],
    retry_key: bool,
) -> Result<Option<Value>> {
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
    let mut response = super::direct::request_expecting(client, api, headers, &[401, 403])
        .map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: NAME,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            },
            error => error,
        })?;
    let status = response.status().as_u16();
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    if status == 401 || status == 403 {
        let text = String::from_utf8_lossy(&body);
        let error = crate::json::from_str(&text)
            .ok()
            .and_then(|data: Value| data.get(0).cloned())
            .and_then(|error| error.as_object().cloned())
            .ok_or_else(|| unsupported("API error responses that are not error lists"))?;
        let message = error
            .get("message")
            .filter(|message| truthy(message))
            .or_else(|| error.get("error_code"))
            .and_then(Value::as_str)
            .ok_or_else(|| unsupported("API errors without a message"))?;
        // `raise_geo_restricted(msg=message)` for `CLIENT_GEO`, else
        // `ExtractorError(message)`; a cached policy key the API rejects is
        // extracted again.
        if error.get("error_subcode").and_then(Value::as_str) == Some("CLIENT_GEO") {
            return Err(crate::geo::restricted(&format!("[{NAME}] {message}"), &[]));
        }
        if error.get("error_code").and_then(Value::as_str) == Some("INVALID_POLICY_KEY")
            && retry_key
        {
            return Ok(None);
        }
        return Err(Error::Reported(format!("[{NAME}] {message}")));
    }
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, response.headers());
    crate::json::from_str(&text).map(Some).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_parse_brightcove_metadata(json_data, video_id, headers)`.
fn parse_metadata(
    client: &Client,
    data: &Map<String, Value>,
    video_id: &str,
    headers: &[(String, String)],
    url: &Url,
) -> Result<VideoInfo> {
    let mut formats: Vec<crate::VideoFormat> = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    let empty = Vec::new();
    let sources = match data.get("sources") {
        Some(Value::Array(sources)) => sources,
        Some(value) if truthy(value) => return Err(unsupported("source lists that are not lists")),
        _ => &empty,
    };
    for source in sources {
        let Some(source) = source.as_object() else {
            return Err(unsupported("sources that are not objects"));
        };
        let text = |key: &str| source.get(key).and_then(Value::as_str);
        let container = text("container");
        let ext = text("type")
            .map(|kind| super::direct::mimetype2ext_or(kind, None))
            .filter(|ext| !ext.is_empty());
        let src = text("src").filter(|src| !src.is_empty());
        let mut fmts = Vec::new();
        if ext.as_deref() == Some("m3u8") || container == Some("M2TS") {
            let Some(src) = src else { continue };
            let manifest = Url::parse(src).map_err(|_| unsupported("relative source URLs"))?;
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                video_id,
                Some("hls"),
                true,
                false,
            ) {
                fmts = stream.formats;
                super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else if ext.as_deref() == Some("mpd") {
            let Some(src) = src else { continue };
            let manifest = Url::parse(src).map_err(|_| unsupported("relative source URLs"))?;
            if let Some(stream) = super::extract_mpd_optional_note(
                client,
                &manifest,
                NAME,
                video_id,
                Some("dash"),
                "Downloading MPD manifest",
            ) {
                fmts = stream.formats;
                super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else {
            let streaming_src = text("streaming_src").filter(|src| !src.is_empty());
            let stream_name = text("stream_name").filter(|name| !name.is_empty());
            let app_name = text("app_name").filter(|name| !name.is_empty());
            if src.is_none()
                && streaming_src.is_none()
                && (stream_name.is_none() || app_name.is_none())
            {
                continue;
            }
            let Some(target) = src.or(streaming_src) else {
                return Err(unsupported("RTMP sources"));
            };
            let tbr = float_or_none(source.get("avg_bitrate"), 1000.0);
            let height = int_or_none(source.get("height"));
            let width = int_or_none(source.get("width"));
            let ext = match (ext, container) {
                (Some(ext), _) => ext,
                (None, Some(container)) => container.to_lowercase(),
                (None, None) => return Err(unsupported("sources without a type or container")),
            };
            let mut map = Map::new();
            let number = |value: Option<f64>| value.map_or(Value::Null, Value::from);
            map.insert("tbr".into(), number(tbr));
            map.insert(
                "filesize".into(),
                int_or_none(source.get("size")).map_or(Value::Null, Value::from),
            );
            map.insert(
                "container".into(),
                container.map_or(Value::Null, Value::from),
            );
            map.insert("ext".into(), ext.into());
            let mut keys = vec!["tbr", "filesize", "container", "ext"];
            if width == Some(0) && height == Some(0) {
                map.insert("vcodec".into(), "none".into());
                keys.push("vcodec");
            } else {
                map.insert("width".into(), width.map_or(Value::Null, Value::from));
                map.insert("height".into(), height.map_or(Value::Null, Value::from));
                map.insert(
                    "vcodec".into(),
                    source.get("codec").cloned().unwrap_or(Value::Null),
                );
                keys.extend(["width", "height", "vcodec"]);
            }
            // `join_nonempty(kind, tbr and f'{int(tbr)}k', height and f'{height}p')`.
            let kind = if src.is_some() {
                "http"
            } else {
                "http-streaming"
            };
            let mut format_id = kind.to_owned();
            if let Some(tbr) = tbr.filter(|tbr| *tbr != 0.0) {
                format_id.push_str(&format!("-{}k", tbr.trunc() as i64));
            }
            if let Some(height) = height.filter(|height| *height != 0) {
                format_id.push_str(&format!("-{height}p"));
            }
            map.insert("url".into(), target.into());
            map.insert("format_id".into(), format_id.into());
            map.insert(
                "source_preference".into(),
                Value::from(if src.is_some() { 0 } else { -1 }),
            );
            keys.extend(["url", "format_id", "source_preference"]);
            // `None` values are assigned by the shape.
            map.retain(|_, value| !value.is_null());
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            fmts.push(format);
        }
        let drm = container == Some("WVM")
            || source.get("key_systems").is_some_and(truthy)
            || text("type")
                .map(|kind| super::direct::mimetype2ext_or(kind, None))
                .as_deref()
                == Some("ism");
        if drm {
            for format in &mut fmts {
                format.extra.insert("has_drm".into(), Value::Bool(true));
                crate::info_order::mark_format_update(format, "has_drm");
            }
        }
        formats.extend(fmts);
    }
    if formats.is_empty()
        && let Some(Value::Array(errors)) = data.get("errors")
        && let Some(error) = errors.first()
    {
        // `raise_no_formats(message or error_subcode or error_code, expected=True)`.
        let message = ["message", "error_subcode", "error_code"]
            .iter()
            .filter_map(|key| error.get(*key))
            .find(|value| truthy(value))
            .and_then(Value::as_str)
            .ok_or_else(|| unsupported("API errors without a message"))?;
        crate::no_formats::raise_expected(NAME, None, None, message)?;
    }
    for format in &mut formats {
        let own = format.http_headers.get_or_insert_with(Map::new);
        for (name, value) in headers {
            own.insert(name.clone(), Value::String(value.clone()));
        }
        crate::info_order::mark_format_update(format, "http_headers");
    }
    if let Some(Value::Array(tracks)) = data.get("text_tracks") {
        for track in tracks.iter().filter_map(Value::as_object) {
            if track.get("kind").and_then(Value::as_str) != Some("captions") {
                continue;
            }
            let Some(track_url) = track
                .get("src")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .and_then(|src| Url::parse(&src).ok())
            else {
                continue;
            };
            let language = str_or_none(track.get("srclang"))
                .filter(|language| !language.is_empty())
                .or_else(|| str_or_none(track.get("label")).filter(|label| !label.is_empty()))
                .unwrap_or_else(|| "en".into())
                .to_lowercase();
            let ext = super::direct::determine_ext(track_url.as_str())
                .unwrap_or_else(|| "unknown_video".into());
            super::html5::merge_subtitles(
                &mut subtitles,
                crate::Subtitles {
                    entries: vec![(
                        Some(language),
                        vec![crate::SubtitleFormat {
                            name: None,
                            url: track_url,
                            ext,
                            protocol: None,
                            fragments: Vec::new(),
                            extra: Default::default(),
                        }],
                    )],
                },
            );
        }
    }
    let duration = float_or_none(data.get("duration"), 1000.0);
    let is_live = duration.is_some_and(|duration| duration <= 0.0);
    const COMMON_RES: [(u32, u32); 9] = [
        (160, 90),
        (320, 180),
        (480, 720),
        (640, 360),
        (768, 432),
        (1024, 576),
        (1280, 720),
        (1366, 768),
        (1920, 1080),
    ];
    static RESOLUTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+x\d+").unwrap());
    // `dict_get(json_data, ('poster', 'thumbnail'))`.
    let thumbnail_base = ["poster", "thumbnail"]
        .iter()
        .filter_map(|key| data.get(*key))
        .find(|value| truthy(value));
    let thumbnails = match thumbnail_base {
        None => Value::Null,
        Some(Value::String(base)) => Value::Array(
            COMMON_RES
                .iter()
                .map(|(width, height)| {
                    serde_json::json!({
                        "url": RESOLUTION.replace_all(base, format!("{width}x{height}").as_str()),
                        "width": width,
                        "height": height,
                    })
                })
                .collect(),
        ),
        Some(_) => return Err(unsupported("posters that are not strings")),
    };

    let no_formats = formats.is_empty();
    let first = formats
        .first()
        .map_or_else(|| url.clone(), |format| format.url.clone());
    let mut info = super::direct::build_at(first.clone(), &first, String::new());
    info.formats = formats;
    info.direct = None;
    info.id = video_id.to_owned();
    info.subtitles = subtitles;
    info.webpage_url = url.clone();
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
    if no_formats {
        info.extra
            .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
    }
    let mut keys = vec![
        "id",
        "thumbnails",
        "duration",
        "formats",
        "subtitles",
        "is_live",
    ];
    info.extra.insert("thumbnails".into(), thumbnails);
    match duration {
        Some(duration) => {
            crate::metadata_parser::set_field(&mut info, "duration", Value::from(duration))
        }
        None => {
            info.extra.insert("duration".into(), Value::Null);
        }
    }
    crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(is_live));
    let clean = |key: &str| -> Result<Option<String>> {
        match data.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(crate::html::clean_html(text))),
            Some(_) => Err(unsupported("metadata fields that are not strings")),
        }
    };
    match clean("name")? {
        Some(title) => {
            info.title = title;
            keys.push("title");
        }
        None => {
            info.title = String::new();
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
    }
    if let Some(description) = clean("description")? {
        info.metadata.description = Some(description);
        keys.push("description");
    }
    let tags: Vec<Value> = data
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter(|tag| tag.as_str().is_some_and(|tag| !tag.is_empty()))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if !tags.is_empty() {
        crate::metadata_parser::set_field(&mut info, "tags", Value::Array(tags));
        keys.push("tags");
    }
    if let Some(timestamp) = data
        .get("published_at")
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
    {
        crate::metadata_parser::set_field(&mut info, "timestamp", Value::from(timestamp));
        keys.push("timestamp");
    }
    if let Some(uploader_id) = data.get("account_id").and_then(Value::as_str) {
        crate::metadata_parser::set_field(&mut info, "uploader_id", Value::from(uploader_id));
        keys.push("uploader_id");
    }
    let shape = format!(
        "dict;{};thumbnail=url,width,height;subtitle=url",
        keys.join(",")
    );
    Ok(crate::info_order::mark_info(info, &shape))
}

impl Extractor for BrightcoveNew {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let (bare, smuggled) = unsmuggle(url.as_str());
        let Some(found) = VALID_URL.captures(&bare) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let account_id = &found["account_id"];
        let player_id = &found["player_id"];
        let embed = &found["embed"];
        let content_type = &found["content_type"];
        let video_id = found["video_id"].to_owned();
        if content_type == "playlist" {
            return Err(unsupported("playlists"));
        }
        let smuggled_text = |key: &str| {
            smuggled
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let token = smuggled_text("token");
        let api = Url::parse(&format!(
            "https://{}.api.brightcove.com/playback/v1/accounts/{account_id}/{content_type}s/{video_id}",
            if token.is_some() { "edge-auth" } else { "edge" }
        ))?;
        let mut headers: Vec<(String, String)> = Vec::new();
        if let Some(token) = &token {
            headers.push(("Authorization".into(), format!("Bearer {token}")));
        }
        if let Some(referrer) = smuggled_text("referrer") {
            static ORIGIN: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r"https?://[^/]+").unwrap());
            let origin = ORIGIN
                .find(&referrer)
                .map(|found| found.as_str().to_owned())
                .ok_or_else(|| unsupported("referrers that are not HTTP URLs"))?;
            headers.push(("Referer".into(), referrer));
            headers.push(("Origin".into(), origin));
        }
        // The policy key is cached as `brightcove` / `ACCOUNT_PLAYER`; a
        // cached key the API rejects is cleared and extracted again.
        let cache_key = format!("{account_id}_{player_id}");
        let mut policy_key = crate::cache::load("brightcove", &cache_key, None).and_then(|key| {
            key.as_str()
                .filter(|key| !key.is_empty())
                .map(str::to_owned)
        });
        let mut extracted = false;
        let (data, policy_key) = loop {
            let key = match policy_key.take() {
                Some(key) => key,
                None => {
                    let key = extract_policy_key(client, account_id, player_id, embed, &video_id)?;
                    extracted = true;
                    crate::cache::store("brightcove", &cache_key, &Value::from(key.as_str()));
                    key
                }
            };
            headers.retain(|(name, _)| name != "Accept");
            headers.push(("Accept".into(), format!("application/json;pk={key}")));
            let request_headers: Vec<(&str, &str)> = headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            match call_api(client, &api, &video_id, &request_headers, !extracted)? {
                Some(data) => break (data, key),
                None => crate::cache::store("brightcove", &cache_key, &Value::Null),
            }
        };
        let Value::Object(data) = data else {
            return Err(unsupported("API responses that are not objects"));
        };
        if let Some(Value::Array(errors)) = data.get("errors")
            && errors
                .first()
                .and_then(|error| error.get("error_subcode"))
                .and_then(Value::as_str)
                == Some("TVE_AUTH")
        {
            let missing: Vec<&str> = ["source_url", "software_statement"]
                .into_iter()
                .filter(|key| smuggled.get(*key).is_none_or(|value| !truthy(value)))
                .collect();
            if !missing.is_empty() {
                return Err(Error::Reported(format!(
                    "[{NAME}] Missing fields in smuggled data: {}. This video can be only extracted from the webpage where it is embedded. Pass the URL of the embedding webpage instead of the Brightcove URL",
                    missing.join(", ")
                )));
            }
            // `get_temp_id` is None: errors have no video id.
            let key_error = |key: &str| {
                Error::Reported(format!(
                    "[{NAME}] An extractor error has occurred. (caused by KeyError('{key}')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            };
            let custom_fields = data
                .get("custom_fields")
                .ok_or_else(|| key_error("custom_fields"))?;
            let custom = |key: &str| -> Result<String> {
                match custom_fields.get(key) {
                    Some(Value::String(text)) => Ok(text.clone()),
                    Some(_) => Err(unsupported("Adobe Pass custom fields that are not strings")),
                    None => Err(key_error(key)),
                }
            };
            let source_url = smuggled_text("source_url")
                .ok_or_else(|| unsupported("smuggled source URLs that are not strings"))?;
            let tve_token = super::adobepass::extract_mvpd_auth(
                client,
                &super::adobepass::Caller {
                    extractor: NAME,
                    id: &video_id,
                    prefix: format!("[{NAME}] "),
                },
                &source_url,
                &custom("bcadobepassrequestorid")?,
                &custom("bcadobepassresourceid")?,
                &smuggled["software_statement"],
            )?;
            let accept = format!("application/json;pk={policy_key}");
            let data = super::download_json_headers(
                client,
                &super::update_url_query(&api, &[("tveToken", &tve_token)]),
                NAME,
                &video_id,
                &[("Accept", &accept)],
            )?;
            let Value::Object(data) = data else {
                return Err(unsupported("API responses that are not objects"));
            };
            headers.retain(|(name, _)| name != "Authorization");
            return parse_metadata(client, &data, &video_id, &headers, url).map(Some);
        }
        // `headers.pop('Authorization')` before the formats get them.
        headers.retain(|(name, _)| name != "Authorization");
        parse_metadata(client, &data, &video_id, &headers, url).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://players.brightcove.net/929656772001/e41d32dc-ec74-459e-a845-6c69f7b724ea_default/index.html?videoId=4463358922001"
        ));
        assert!(suitable(
            "https://players.brightcove.net/1752604059001/S13cJdUBz_default/index.html?playlistId=5718313430001"
        ));
        assert!(suitable(
            "http://players.brightcove.net/710858724001/default_default/index.html?videoId=ref:event-stream-356"
        ));
        assert!(!suitable(
            "https://players.brightcove.net/929656772001/default_default/index.html"
        ));
    }

    #[test]
    fn smuggling_round_trips() {
        let url = smuggle(
            "https://players.brightcove.net/1/default_default/index.html?videoId=2",
            &serde_json::json!({"referrer": "https://example.com/a"}),
        );
        assert_eq!(
            url,
            "https://players.brightcove.net/1/default_default/index.html?videoId=2#__youtubedl_smuggle=%7B%22referrer%22%3A+%22https%3A%2F%2Fexample.com%2Fa%22%7D"
        );
        let (bare, data) = unsmuggle(&url);
        assert_eq!(
            bare,
            "https://players.brightcove.net/1/default_default/index.html?videoId=2"
        );
        assert_eq!(data["referrer"], "https://example.com/a");
        assert!(unsmuggle("https://a.example/#x").1.is_empty());
    }

    #[test]
    fn embeds_follow_upstream_patterns() {
        let page = r#"<iframe src="//players.brightcove.net/1/abc_default/index.html?videoId=2"></iframe>
<video data-video-id="5" data-account="9" class="video-js"></video>
<video-js data-video-id="7" data-player="p"></video-js>
<script src="https://players.brightcove.net/8/xyz_default/index.min.js"></script>"#;
        assert_eq!(
            brightcove_urls(page),
            [
                "https://players.brightcove.net/1/abc_default/index.html?videoId=2",
                "https://players.brightcove.net/8/xyz_default/index.html?videoId=5",
            ]
        );
        assert!(
            brightcove_urls(r#"<video data-video-id="5" data-account="9"></video>"#).is_empty()
        );
    }
}
