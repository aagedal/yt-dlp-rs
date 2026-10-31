//! Upstream `KalturaIE` (`yt_dlp/extractor/kaltura.py`): `kaltura:PARTNER:ENTRY`
//! pseudo-URLs and `kwidget`/`mwEmbedFrame.php` player URLs through the
//! multirequest API (html5 and kwidget sessions), the flavor assets as
//! formats, the `playManifest` HLS playlist, caption assets as subtitles and
//! the entry metadata; `_extract_embed_urls` for generic pages. Player URLs
//! of `playlistAPI.kpl0Id` playlists remain an explicit native error.
use super::Extractor;
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Kaltura;

pub(crate) const NAME: &str = "Kaltura";
pub(crate) const KEY: &str = "Kaltura";
const SERVICE_URL: &str = "http://cdnapi.kaltura.com";
const SERVICE_BASE: &str = "/api_v3/service/multirequest";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?:kaltura:(?P<partner_id>\w+):(?P<id>\w+)(?::(?P<player_type>\w+))?|https?://(?:(?:www|cdnapi(?:sec)?)\.)?kaltura\.com(?::\d+)?/(?:(?:index\.php/(?:kwidget|extwidget/preview)|html5/html5lib/[^/]+/mwEmbedFrame\.php))(?:/(?P<path>[^?]+))?(?:\?(?P<query>.*))?)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`: the `id` group (pseudo-URLs only).
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL
        .captures(url)
        .and_then(|found| found.name("id"))
        .map(|id| id.as_str().to_owned())
}

/// `KalturaIE._extract_embed_urls`: `kWidget.embed` calls, else `cdnapi`
/// script URLs followed by an `entry_id`, else iframe/meta player URLs with
/// an `entry_id` query; each as `kaltura:PARTNER:ENTRY`, smuggling the
/// page's `embedIframeJs` service URL. The quote backreferences are spelled
/// out as per-quote alternatives.
pub(crate) fn embed_urls(_url: &Url, webpage: &str) -> Vec<Url> {
    static PATTERNS: LazyLock<[Regex; 3]> = LazyLock::new(|| {
        [
            Regex::new(
                r#"(?s)kWidget\.(?:thumb)?[Ee]mbed\(\{.*?(?:"wid"|'wid')\s*:\s*(?:"_?(?P<p1>[^"]+)"|'_?(?P<p2>[^']+)'),.*?(?:"entry_?[Ii]d"|'entry_?[Ii]d')\s*:\s*(?:"(?P<i1>[^"]+)"|'(?P<i2>[^']+)')(?:,|\s*\})"#,
            )
            .unwrap(),
            Regex::new(
                r#"(?s)(?:"(?:https?:)?//cdnapi(?:sec)?\.kaltura\.com(?::\d+)?/[^"]*\b(?:p|partner_id)/(?P<p1>\d+)[^"]*"|'(?:https?:)?//cdnapi(?:sec)?\.kaltura\.com(?::\d+)?/[^']*\b(?:p|partner_id)/(?P<p2>\d+)[^']*').*?(?:(?:entry_?[Ii]d|"entry_?[Ii]d"|'entry_?[Ii]d')\s*:\s*|\[\s*(?:"entry_?[Ii]d"|'entry_?[Ii]d')\s*\]\s*=\s*)(?:"(?P<i1>[^"]+)"|'(?P<i2>[^']+)')"#,
            )
            .unwrap(),
            Regex::new(
                r#"(?s)<(?:iframe[^>]+src|meta[^>]+\bcontent)=(?:"\s*(?:https?:)?//(?:(?:www|cdnapi(?:sec)?)\.)?kaltura\.com/[^"]*\b(?:p|partner_id)/(?P<p1>\d+)[^"]*[?&;]entry_id=(?P<i1>[^"&]+)[^"]*"|'\s*(?:https?:)?//(?:(?:www|cdnapi(?:sec)?)\.)?kaltura\.com/[^']*\b(?:p|partner_id)/(?P<p2>\d+)[^']*[?&;]entry_id=(?P<i2>[^'&]+)[^']*')"#,
            )
            .unwrap(),
        ]
    });
    let group = |found: &regex::Captures, a: &str, b: &str| {
        found
            .name(a)
            .or_else(|| found.name(b))
            .map(|value| value.as_str().trim().to_owned())
            .unwrap_or_default()
    };
    let mut matches = Vec::new();
    for pattern in PATTERNS.iter() {
        matches = pattern
            .captures_iter(webpage)
            .map(|found| (group(&found, "p1", "p2"), group(&found, "i1", "i2")))
            .collect();
        if !matches.is_empty() {
            break;
        }
    }
    let mut urls: Vec<Url> = Vec::new();
    for (partner_id, id) in matches {
        let mut embed = format!("kaltura:{partner_id}:{id}");
        let escaped = regex::escape(&partner_id);
        let service = Regex::new(&format!(
            r#"<script[^>]+src=(?:"(?P<s1>(?:https?:)?//[^"\n]+)|'(?P<s2>(?:https?:)?//[^'\n]+))/p/{escaped}/sp/{escaped}00/embedIframeJs"#
        ))
        .ok()
        .and_then(|pattern| {
            pattern.captures(webpage).map(|found| {
                found
                    .name("s1")
                    .or_else(|| found.name("s2"))
                    .map(|value| value.as_str().to_owned())
                    .unwrap_or_default()
            })
        });
        if let Some(service) = service {
            embed = super::brightcove::smuggle(&embed, &json!({ "service_url": service }));
        }
        // `extract_from_webpage`: `orderedSet`.
        if let Ok(url) = Url::parse(&embed)
            && !urls.contains(&url)
        {
            urls.push(url);
        }
    }
    urls
}

/// `url_result(TARGET, KalturaIE)` for an extractor delegating to Kaltura.
pub(crate) fn follow(url: &Url, target: String) -> crate::ExtractResult {
    let mut follow = super::direct::build_at(url.clone(), url, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
    crate::ExtractResult::Video(follow)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Kaltura {what} are not supported natively"))
}

/// An `ExtractorError` with `get_temp_id`'s prefix.
fn fail(id: Option<&str>, message: &str) -> Error {
    match id {
        Some(id) => Error::Reported(format!("[{NAME}] {id}: {message}")),
        None => Error::Reported(format!("[{NAME}] {message}")),
    }
}

/// An uncaught `KeyError`, wrapped without the extractor prefix.
fn bug(id: Option<&str>, cause: &str) -> Error {
    let message = format!(
        "An extractor error has occurred. (caused by {cause}); {}",
        crate::BUG_REPORTS_MESSAGE
    );
    Error::Reported(match id {
        Some(id) => format!("{id}: {message}"),
        None => message,
    })
}

fn key_error(id: Option<&str>, key: &str) -> Error {
    bug(id, &format!("KeyError('{key}')"))
}

/// Python `str()` of a JSON scalar.
fn py_str(value: &Value) -> Result<String> {
    Ok(match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(text) => text.clone(),
        Value::Number(number) if number.is_f64() => {
            let float = number.as_f64().unwrap_or_default();
            if float.fract() == 0.0 && float.abs() < 1e16 {
                format!("{float:.1}")
            } else {
                float.to_string()
            }
        }
        Value::Number(number) => number.to_string(),
        _ => return Err(unsupported("non-scalar flavor fields")),
    })
}

/// `int_or_none(value, invscale=INVSCALE)`.
fn int_or_none(value: Option<&Value>, invscale: i64) -> Option<i64> {
    let number = match value? {
        Value::Bool(flag) => i64::from(*flag),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float.trunc() as i64))?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    Some(number * invscale)
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

fn widget_id(partner_id: &str) -> String {
    if partner_id.contains('_') {
        partner_id.to_owned()
    } else {
        format!("_{partner_id}")
    }
}

/// `_kaltura_api_call`: the POSTed multirequest JSON, failing on an
/// `KalturaAPIException` result, with `data[1]` as its first object.
fn api_call(
    client: &Client,
    video_id: &str,
    actions: Vec<Value>,
    service_url: Option<&str>,
    note: &str,
    temp: Option<&str>,
) -> Result<Vec<Value>> {
    use std::io::Read;
    let mut params = match actions.first() {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    for (index, action) in actions.into_iter().enumerate().skip(1) {
        params.insert(index.to_string(), action);
    }
    let body = crate::output::python_json_dumps(&Value::Object(params));
    let url = Url::parse(&format!(
        "{}{SERVICE_BASE}",
        service_url.unwrap_or(SERVICE_URL)
    ))
    .map_err(|_| unsupported("service URLs that are not absolute URLs"))?;
    crate::screen::screen(&format!("[{NAME}] {video_id}: {note}"));
    let mut response =
        super::direct::post(client, &url, &[("Content-Type", "application/json")], body).map_err(
            |error| match error {
                Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                    extractor: NAME,
                    id: video_id.to_owned(),
                    what: "JSON metadata",
                    source: Box::new(error),
                },
                error => error,
            },
        )?;
    let mut raw = Vec::new();
    response
        .read_to_end(&mut raw)
        .map_err(Error::ExtractionRead)?;
    let raw = super::decode_content(response.headers(), raw).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&raw, response.headers());
    let data: Value = crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
        ))
    })?;
    let Value::Array(mut data) = data else {
        return Err(unsupported("multirequest responses that are not lists"));
    };
    for (index, status) in data.iter().enumerate() {
        if status.get("objectType").and_then(Value::as_str) == Some("KalturaAPIException") {
            let message = status
                .get("message")
                .ok_or_else(|| key_error(temp, "message"))?;
            return Err(bug_free(
                temp,
                &format!("{NAME} said: {} ({index})", py_str(message)?),
            ));
        }
    }
    if data.len() < 2 {
        return Err(unsupported(
            "multirequest responses with fewer than two results",
        ));
    }
    data[1] = data[1]
        .get("objects")
        .and_then(|objects| objects.get(0))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(data)
}

/// A non-expected `ExtractorError(message)`.
fn bug_free(id: Option<&str>, message: &str) -> Error {
    fail(id, &format!("{message}; {}", crate::BUG_REPORTS_MESSAGE))
}

/// `_get_video_info`: `(info, flavor_assets, captions)`.
fn video_info(
    client: &Client,
    video_id: &str,
    partner_id: &str,
    service_url: Option<&str>,
    player_type: &str,
    temp: Option<&str>,
) -> Result<(Value, Value, Value)> {
    let info_action = json!({
        "action": "list",
        "filter": {"redirectFromEntryId": video_id},
        "service": "baseentry",
        "ks": "{1:result:ks}",
        "responseProfile": {
            "type": 1,
            "fields": "createdAt,dataUrl,duration,name,plays,thumbnailUrl,userId",
        },
    });
    let flavor_action = json!({
        "action": "getbyentryid",
        "entryId": video_id,
        "service": "flavorAsset",
        "ks": "{1:result:ks}",
    });
    let caption_action = json!({
        "action": "list",
        "filter:entryIdEqual": video_id,
        "service": "caption_captionasset",
        "ks": "{1:result:ks}",
    });
    let session = json!({
        "expiry": 86400,
        "service": "session",
        "action": "startWidgetSession",
        "widgetId": widget_id(partner_id),
    });
    match player_type {
        "html5" => {
            let actions = vec![
                json!({
                    "apiVersion": "3.3.0",
                    "clientTag": "html5:v3.1.0",
                    "format": 1,
                    "ks": "",
                    "partnerId": partner_id,
                }),
                session,
                info_action,
                flavor_action,
                caption_action,
            ];
            let data = api_call(
                client,
                video_id,
                actions,
                service_url,
                "Downloading video info JSON (Kaltura html5 player)",
                temp,
            )?;
            let [_, info, flavors, captions]: [Value; 4] = data
                .try_into()
                .map_err(|_| unsupported("html5 responses without four results"))?;
            Ok((info, flavors, captions))
        }
        "kwidget" => {
            let actions = vec![
                json!({
                    "service": "multirequest",
                    "apiVersion": "3.1",
                    "expiry": 86400,
                    "clientTag": "kwidget:v2.89",
                    "format": 1,
                    "ignoreNull": 1,
                    "action": "null",
                }),
                session,
                json!({
                    "expiry": 86400,
                    "service": "session",
                    "action": "startwidgetsession",
                    "widgetId": widget_id(partner_id),
                    "format": 9,
                    "apiVersion": "3.1",
                    "clientTag": "kwidget:v2.89",
                    "ignoreNull": 1,
                    "ks": "{1:result:ks}",
                }),
                info_action,
                flavor_action,
                caption_action,
            ];
            let data = api_call(
                client,
                video_id,
                actions,
                service_url,
                "Downloading video info JSON (Kaltura kwidget player)",
                temp,
            )?;
            let [_, _, info, flavors, captions]: [Value; 5] = data
                .try_into()
                .map_err(|_| unsupported("kwidget responses without five results"))?;
            let info = info
                .get("objects")
                .and_then(|objects| objects.get(0))
                .cloned()
                .ok_or_else(|| unsupported("kwidget entries without objects"))?;
            Ok((info, flavors, captions))
        }
        // `assert player_type in ('html5', 'kwidget')`: a bare `AssertionError`.
        _ => Err(Error::Reported(String::new())),
    }
}

/// `parse_qs` (blank values dropped) as ordered `(key, values)`.
fn parse_qs(query: &str) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if value.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, values)) => values.push(value.into_owned()),
            None => groups.push((key.into_owned(), vec![value.into_owned()])),
        }
    }
    groups
}

impl Extractor for Kaltura {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let original = url.as_str();
        if !VALID_URL.is_match(original) {
            return Ok(None);
        }
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(original)
        ));
        let temp = temp_id(original);
        let temp = temp.as_deref();
        let (bare, smuggled) = super::brightcove::unsmuggle(original);
        let Some(found) = VALID_URL.captures(&bare) else {
            return Err(unsupported("smuggled URLs that do not match"));
        };
        let smuggled_str = |key: &str| match smuggled.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(unsupported("non-string smuggled values")),
        };
        let service_url = smuggled_str("service_url")?;
        let source_url = smuggled_str("source_url")?;
        let player_type = match found.name("player_type") {
            Some(kind) => kind.as_str().to_owned(),
            None if bare.contains("html5lib/v2") => "kwidget".into(),
            None => "html5".into(),
        };
        let mut ks = None;
        let (entry_id, info, flavors, captions) = if let (Some(partner), Some(entry)) =
            (found.name("partner_id"), found.name("id"))
        {
            let entry = entry.as_str().to_owned();
            let (info, flavors, captions) = video_info(
                client,
                &entry,
                partner.as_str(),
                service_url.as_deref(),
                &player_type,
                temp,
            )?;
            (entry, info, flavors, captions)
        } else {
            let path = found.name("path").map(|value| value.as_str());
            let query = found.name("query").map(|value| value.as_str());
            if path.is_none_or(str::is_empty) && query.is_none_or(str::is_empty) {
                return Err(fail(temp, "Invalid URL"));
            }
            let mut params = query.map(parse_qs).unwrap_or_default();
            if let Some(path) = path.filter(|path| !path.is_empty()) {
                let parts: Vec<&str> = path.split('/').collect();
                for pair in parts.chunks(2) {
                    if let [key, value] = pair {
                        let value = vec![(*value).to_owned()];
                        match params.iter_mut().find(|(existing, _)| existing == key) {
                            Some((_, values)) => *values = value,
                            None => params.push(((*key).to_owned(), value)),
                        }
                    }
                }
            }
            let param = |key: &str| {
                params
                    .iter()
                    .find(|(existing, _)| existing == key)
                    .and_then(|(_, values)| values.first().cloned())
            };
            let partner_id = if let Some(wid) = param("wid") {
                wid.strip_prefix('_').map(str::to_owned).unwrap_or(wid)
            } else if let Some(partner) = param("p").or_else(|| param("partner_id")) {
                partner
            } else {
                return Err(fail(temp, "Invalid URL"));
            };
            let result = if let Some(entry) = param("entry_id") {
                let (info, flavors, captions) =
                    video_info(client, &entry, &partner_id, None, &player_type, temp)?;
                (entry, info, flavors, captions)
            } else if param("uiconf_id").is_some()
                && let Some(reference) = param("flashvars[referenceId]")
            {
                let page = Url::parse(&bare)?;
                let webpage = super::download_webpage(client, &page, NAME, &reference)?;
                static PACKAGE: LazyLock<Regex> =
                    LazyLock::new(|| Regex::new(r"window\.kalturaIframePackageData\s*=").unwrap());
                let package = match super::search_json(&PACKAGE, &webpage, false) {
                    Some(Some(package)) => package,
                    Some(None) => {
                        return Err(fail(
                            Some(&reference),
                            &format!("{reference}: Failed to parse JSON kalturaIframePackageData"),
                        ));
                    }
                    None => {
                        return Err(fail(
                            Some(&reference),
                            &format!(
                                "Unable to extract kalturaIframePackageData; {}",
                                crate::BUG_REPORTS_MESSAGE
                            ),
                        ));
                    }
                };
                let entry_data = package
                    .get("entryResult")
                    .ok_or_else(|| key_error(temp, "entryResult"))?;
                let info = entry_data
                    .get("meta")
                    .cloned()
                    .ok_or_else(|| key_error(temp, "meta"))?;
                let flavors = entry_data
                    .get("contextData")
                    .ok_or_else(|| key_error(temp, "contextData"))?
                    .get("flavorAssets")
                    .cloned()
                    .ok_or_else(|| key_error(temp, "flavorAssets"))?;
                let entry = match info.get("id") {
                    Some(Value::String(id)) => id.clone(),
                    Some(_) => return Err(unsupported("non-string iframe entry IDs")),
                    None => return Err(key_error(temp, "id")),
                };
                // Captions through the regular API; its failure keeps
                // the page's data.
                match video_info(client, &entry, &partner_id, None, &player_type, temp) {
                    Ok((info, flavors, captions)) => (entry, info, flavors, captions),
                    Err(error @ Error::UnsupportedHtml5(_)) => return Err(error),
                    Err(_) => (entry, info, flavors, Value::Null),
                }
            } else if param("uiconf_id").is_some()
                && param("flashvars[playlistAPI.kpl0Id]").is_some()
            {
                return Err(unsupported("playlist player URLs (playlistAPI.kpl0Id)"));
            } else {
                return Err(fail(temp, "Invalid URL"));
            };
            ks = param("flashvars[ks]");
            result
        };
        per_video(
            client,
            url,
            source_url.as_deref(),
            &entry_id,
            &info,
            ks.as_deref(),
            &flavors,
            &captions,
            temp,
        )
        .map(Some)
    }
}

/// `_per_video_extract`.
#[allow(clippy::too_many_arguments)]
fn per_video(
    client: &Client,
    url: &Url,
    source_url: Option<&str>,
    entry_id: &str,
    info: &Value,
    ks: Option<&str>,
    flavors: &Value,
    captions: &Value,
    temp: Option<&str>,
) -> Result<VideoInfo> {
    use base64::Engine;
    let referrer = match source_url {
        Some(source) => {
            // `urlparse(source_url)[:2]`: the scheme and netloc as written.
            let (scheme, rest) = source.split_once(':').unwrap_or(("", source));
            let netloc = rest
                .strip_prefix("//")
                .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or_default())
                .unwrap_or_default();
            let origin = format!("{}://{netloc}", scheme.to_ascii_lowercase());
            Some(base64::engine::general_purpose::STANDARD.encode(origin))
        }
        None => None,
    };
    let sign = |unsigned: String| {
        let mut signed = unsigned;
        if let Some(ks) = ks {
            signed.push_str(&format!("/ks/{ks}"));
        }
        if let Some(referrer) = &referrer {
            signed.push_str(&format!("?referrer={referrer}"));
        }
        signed
    };
    // `info['dataUrl']` of a missing entry: an uncaught `TypeError`.
    if info.is_null() {
        return Err(Error::Reported(
            "'NoneType' object is not subscriptable".into(),
        ));
    }
    if !info.is_object() {
        return Err(unsupported("non-object entries"));
    }
    let mut data_url = match info.get("dataUrl") {
        Some(Value::String(text)) => text.clone(),
        Some(_) => return Err(unsupported("non-string dataUrl values")),
        None => return Err(key_error(temp, "dataUrl")),
    };
    static FLVCLIPPER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/flvclipper/.*").unwrap());
    if data_url.contains("/flvclipper/") {
        data_url = FLVCLIPPER
            .replace_all(&data_url, "/serveFlavor")
            .into_owned();
    }
    let Value::Array(flavors) = flavors else {
        return Err(unsupported("non-list flavor assets"));
    };
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for flavor in flavors {
        let Value::Object(flavor) = flavor else {
            return Err(unsupported("non-object flavor assets"));
        };
        if flavor.get("status").and_then(Value::as_f64) != Some(2.0)
            || flavor.get("status").is_some_and(Value::is_boolean)
        {
            continue;
        }
        let file_ext = flavor.get("fileExt");
        if file_ext.and_then(Value::as_str) == Some("chun")
            || file_ext.and_then(Value::as_str) == Some("wvm")
        {
            continue;
        }
        let file_ext = if truthy(file_ext) {
            file_ext.cloned().unwrap_or(Value::Null)
        } else if flavor.get("containerFormat").and_then(Value::as_str) == Some("qt") {
            Value::from("mov")
        } else {
            Value::from("mp4")
        };
        let flavor_id = flavor.get("id").ok_or_else(|| key_error(temp, "id"))?;
        let video_url = sign(format!("{data_url}/flavorId/{}", py_str(flavor_id)?));
        let bitrate = flavor
            .get("bitrate")
            .ok_or_else(|| key_error(temp, "bitrate"))?;
        let format_id = format!("{}-{}", py_str(&file_ext)?, py_str(bitrate)?);
        if flavor.get("isOriginal") == Some(&Value::Bool(true)) {
            let checked = super::proto_relative_http(&video_url);
            if checked.starts_with("http://") || checked.starts_with("https://") {
                crate::screen::screen(&format!("[{NAME}] {entry_id}: Checking {format_id} URL"));
                let target = Url::parse(&checked)?;
                if let Err(error) = super::direct::request(client, &target, None) {
                    crate::screen::screen(&format!(
                        "[{NAME}] {entry_id}: {format_id} URL is invalid, skipping: {}",
                        crate::upstream_error_text(&error)
                    ));
                    continue;
                }
            }
        }
        let frame_rate_zero = flavor
            .get("frameRate")
            .is_some_and(|rate| rate.as_f64() == Some(0.0) || rate == &Value::Bool(false));
        let vcodec = if !flavor.contains_key("videoCodecId") && frame_rate_zero {
            Value::from("none")
        } else {
            flavor.get("videoCodecId").cloned().unwrap_or(Value::Null)
        };
        let int = |key: &str, invscale: i64| {
            int_or_none(flavor.get(key), invscale).map_or(Value::Null, Value::from)
        };
        let mut map = Map::new();
        map.insert("format_id".into(), format_id.into());
        map.insert("ext".into(), file_ext);
        map.insert("tbr".into(), int("bitrate", 1));
        map.insert("fps".into(), int("frameRate", 1));
        map.insert("filesize_approx".into(), int("size", 1024));
        map.insert(
            "container".into(),
            flavor
                .get("containerFormat")
                .cloned()
                .unwrap_or(Value::Null),
        );
        map.insert("vcodec".into(), vcodec);
        map.insert("height".into(), int("height", 1));
        map.insert("width".into(), int("width", 1));
        map.insert("url".into(), video_url.into());
        let keys: Vec<String> = map.keys().cloned().collect();
        map.retain(|_, value| !value.is_null());
        if !map.get("ext").is_some_and(Value::is_string) {
            return Err(unsupported("non-string flavor extensions"));
        }
        if map.get("container").is_some_and(|value| !value.is_string())
            || map.get("vcodec").is_some_and(|value| !value.is_string())
        {
            return Err(unsupported("non-string flavor containers or codecs"));
        }
        let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
            .map_err(|error| fail(Some(entry_id), &error.to_string()))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    if data_url.contains("/playManifest/") {
        let manifest = sign(data_url.replace("format/url", "format/applehttp"));
        let manifest =
            Url::parse(&manifest).map_err(|_| unsupported("manifest URLs that are not URLs"))?;
        if let Some(stream) =
            super::extract_m3u8_optional(client, &manifest, NAME, entry_id, Some("hls"), true)
        {
            for mut format in stream.formats {
                format.ext = "mp4".into();
                formats.push(format);
            }
            super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
        }
    }
    if truthy(Some(captions)) {
        let objects = match captions.get("objects") {
            None => &Vec::new(),
            Some(Value::Array(objects)) => objects,
            Some(_) => return Err(unsupported("non-list caption objects")),
        };
        for caption in objects {
            if caption.get("status").and_then(Value::as_f64) != Some(2.0)
                || caption.get("status").is_some_and(Value::is_boolean)
            {
                continue;
            }
            if !truthy(caption.get("id")) {
                continue;
            }
            let language = if truthy(caption.get("languageCode")) {
                caption.get("languageCode")
            } else {
                caption.get("language")
            };
            let language = match language {
                None | Some(Value::Null) => None,
                Some(Value::String(text)) => Some(text.clone()),
                Some(_) => return Err(unsupported("non-string caption languages")),
            };
            let ext = if truthy(caption.get("fileExt")) {
                caption.get("fileExt").cloned().unwrap_or(Value::Null)
            } else {
                Value::from(match int_or_none(caption.get("format"), 1) {
                    Some(1) => "srt",
                    Some(2) => "ttml",
                    Some(3) => "vtt",
                    _ => "ttml",
                })
            };
            let track: crate::SubtitleFormat = serde_json::from_value(json!({
                "url": format!(
                    "{SERVICE_URL}/api_v3/service/caption_captionasset/action/serve/captionAssetId/{}",
                    py_str(caption.get("id").unwrap_or(&Value::Null))?
                ),
                "ext": ext,
            }))
            .map_err(|error| fail(Some(entry_id), &error.to_string()))?;
            match subtitles
                .entries
                .iter_mut()
                .find(|(existing, _)| *existing == language)
            {
                Some((_, tracks)) => tracks.push(track),
                None => subtitles.entries.push((language, vec![track])),
            }
        }
    }
    let title = match info.get("name") {
        Some(Value::String(title)) => Some(title.clone()),
        Some(Value::Null) => None,
        Some(_) => return Err(unsupported("non-string entry names")),
        None => return Err(key_error(temp, "name")),
    };
    let description = match info.get("description") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(text)) => Value::from(crate::html::clean_html(text)),
        Some(_) => return Err(unsupported("non-string descriptions")),
    };
    let uploader_id = match info.get("userId") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) if text == "None" => String::new(),
        Some(value) => py_str(value)?,
    };
    let no_formats = formats.is_empty();
    let first = formats
        .first()
        .map_or_else(|| url.clone(), |format| format.url.clone());
    let mut result = super::direct::build_at(first.clone(), &first, String::new());
    result.formats = formats;
    result.subtitles = subtitles;
    result.direct = None;
    result.id = entry_id.to_owned();
    match title {
        Some(title) => result.title = title,
        None => {
            result.title = String::new();
            result
                .extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
    }
    result.webpage_url = url.clone();
    result.extractor = NAME.into();
    result.extractor_key = KEY.into();
    if no_formats {
        result
            .extra
            .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
    }
    let raw = |key: &str| info.get(key).cloned().unwrap_or(Value::Null);
    for (name, value) in [
        ("description", description),
        ("thumbnail", raw("thumbnailUrl")),
        ("duration", raw("duration")),
        ("timestamp", raw("createdAt")),
        ("uploader_id", Value::from(uploader_id)),
        (
            "view_count",
            int_or_none(info.get("plays"), 1).map_or(Value::Null, Value::from),
        ),
    ] {
        crate::metadata_parser::set_field(&mut result, name, value);
    }
    Ok(crate::info_order::mark_info(
        result,
        "dict;id,title,formats,subtitles,description,thumbnail,duration,timestamp,uploader_id,view_count;subtitle=url,ext",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("kaltura:269692:1_1jc2y3e4"));
        assert!(suitable("kaltura:269692:1_1jc2y3e4:kwidget"));
        assert!(suitable(
            "http://www.kaltura.com/index.php/kwidget/cache_st/1300318621/wid/_269692/uiconf_id/3873291/entry_id/1_1jc2y3e4"
        ));
        assert!(suitable(
            "https://cdnapisec.kaltura.com/html5/html5lib/v2.30.2/mwEmbedFrame.php/p/1337/uiconf_id/20540612/entry_id/1_sf5ovm7u?wid=_243342"
        ));
        assert!(!suitable("https://www.kaltura.com/about"));
        assert_eq!(temp_id("kaltura:1:0_abc").as_deref(), Some("0_abc"));
        assert_eq!(
            temp_id("https://www.kaltura.com/index.php/kwidget/wid/_1/entry_id/0_x"),
            None
        );
    }

    #[test]
    fn embeds_follow_upstream_patterns() {
        let page = Url::parse("https://example.com/").unwrap();
        let urls = embed_urls(
            &page,
            r#"<script>kWidget.embed({"targetId": "x", "wid": "_1645161", "entry_id": "0_abc"});</script>"#,
        );
        assert_eq!(urls[0].as_str(), "kaltura:1645161:0_abc");
        let urls = embed_urls(
            &page,
            r#"<iframe src="https://cdnapisec.kaltura.com/p/1645161/sp/164516100/embedIframeJs/uiconf_id/1/partner_id/1645161?iframeembed=true&entry_id=1_def&x=y"></iframe>"#,
        );
        assert_eq!(urls[0].as_str(), "kaltura:1645161:1_def");
        let urls = embed_urls(
            &page,
            r#"<script src="https://api.example.org/p/42/sp/4200/embedIframeJs/uiconf_id/1"></script><script>kWidget.thumbEmbed({'wid': '_42', 'entry_id': '0_q'})</script>"#,
        );
        assert!(
            urls[0]
                .as_str()
                .starts_with("kaltura:42:0_q#__youtubedl_smuggle=")
        );
    }

    #[test]
    fn python_strings_and_ints() {
        assert_eq!(py_str(&json!(1000)).unwrap(), "1000");
        assert_eq!(py_str(&json!(1000.0)).unwrap(), "1000.0");
        assert_eq!(py_str(&Value::Null).unwrap(), "None");
        assert_eq!(int_or_none(Some(&json!(12)), 1024), Some(12288));
        assert_eq!(int_or_none(Some(&json!("x")), 1), None);
    }
}
