//! Upstream `LBRYIE` (`yt_dlp/extractor/lbry.py`, `lbry`/odysee.com): the
//! API proxy's `resolve` and `get` (streaming URL), the original file when
//! the v3 API serves one, the streaming redirect's HLS formats, livestreams
//! and the claim's metadata. `LBRYChannelIE` (`lbry:channel`) and
//! `LBRYPlaylistIE` (`lbry:playlist`): the channel's `resolve` or the
//! playlist's `claim_search`, then `claim_search` pages (`OnDemandPagedList`,
//! 50 per page) of `url` entries, filtered by the URL's query.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Lbry;

const NAME: &str = "lbry";
const KEY: &str = "LBRY";
const CLAIM: &str = "[0-9a-f]{1,40}";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    let opt = format!(r"[^$@:/?#&]+(?:[:#]{CLAIM})?");
    crate::extractor::url_regex(&format!(
        r"^(?:https?://(?:www\.)?(?:lbry\.tv|odysee\.com)/|lbry://)(?:\$/(?:download|embed)/)?(?P<id>[^$@:/?#]+/{CLAIM}|(?:@{opt}/)?{opt})"
    ))
    .unwrap()
});
static API_VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/api/v\d+/").unwrap());

/// `LBRYIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// `extract`'s wrapping of a `KeyError(key)`: `ExtractorError(..., video_id=temp_id)`
/// (no extractor prefix).
fn key_error(temp_id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{temp_id}: An extractor error has occurred. (caused by KeyError({})); {}",
        crate::output::traverse::py_repr_str(key),
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn int(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
        .map(Value::from)
}

/// `_call_api_proxy(method, display_id, params, resource)`.
/// Errors carry `temp_id`, extract's temporary ID (the URL's raw match).
/// `temp_id` is `None` for a playlist page read after extraction (the error
/// then carries no extractor/ID prefix).
fn call_api(
    client: &Client,
    (name, method): (&'static str, &str),
    id: &str,
    temp_id: Option<&str>,
    params: Value,
    resource: &str,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!(
        "[{name}] {id}: Downloading {resource} JSON metadata"
    ));
    let mut headers = vec![("Content-Type", "application/json-rpc".to_owned())];
    if let Some(token) = crate::cookies::get_cookie("https://odysee.com", "auth_token") {
        headers.push(("x-lbry-auth-token", token));
    }
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let body = crate::output::python_json_dumps(&json!({"method": method, "params": params}));
    let url = Url::parse("https://api.lbry.tv/api/v1/proxy")?;
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
    let mut text = Vec::new();
    response
        .read_to_end(&mut text)
        .map_err(Error::ExtractionRead)?;
    let response: Value = serde_json::from_slice(&text).map_err(|error| {
        Error::Reported(format!(
            "[{name}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })?;
    if let Some(error) = response
        .get("error")
        .filter(|error| !error.is_null() && error.as_object().is_none_or(|e| !e.is_empty()))
    {
        let field = |key: &str| match error.get(key) {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Null) | None => "None".into(),
            Some(other) => other.to_string(),
        };
        let message = format!("{name} said: {} - {}", field("code"), field("message"));
        return Err(Error::Reported(match temp_id {
            Some(temp_id) => format!("[{name}] {temp_id}: {message}"),
            None => message,
        }));
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}

/// `_permanent_url(url, claim_name, claim_id)`.
fn permanent_url(url: &str, name: &str, claim: &str) -> Option<String> {
    let base = Url::parse(&url.replace("lbry://", "https://lbry.tv/")).ok()?;
    base.join(&format!("/{name}:{claim}"))
        .ok()
        .map(String::from)
}

/// `_parse_stream(stream, url)`.
fn parse_stream(stream: &Value, url: &str) -> Map<String, Value> {
    let stream_type = stream.pointer("/value/stream_type").and_then(Value::as_str);
    let text = |path: &str| {
        stream
            .pointer(path)
            .filter(|value| value.is_string())
            .cloned()
    };
    let mut info = Map::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            info.insert(key.to_owned(), value);
        }
    };
    put("title", text("/value/title"));
    put(
        "thumbnail",
        stream
            .pointer("/value/thumbnail/url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map(Value::from),
    );
    put("description", text("/value/description"));
    put("license", text("/value/license"));
    put("timestamp", int(stream.get("timestamp")));
    put(
        "release_timestamp",
        int(stream.pointer("/value/release_time")),
    );
    let tags: Vec<Value> = stream
        .pointer("/value/tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|tag| match tag {
            Value::String(text) => !text.is_empty(),
            Value::Null => false,
            _ => true,
        })
        .cloned()
        .collect();
    put("tags", (!tags.is_empty()).then_some(Value::Array(tags)));
    put(
        "duration",
        stream_type.and_then(|kind| int(stream.pointer(&format!("/value/{kind}/duration")))),
    );
    put("channel", text("/signing_channel/value/title"));
    put("channel_id", text("/signing_channel/claim_id"));
    put("uploader_id", text("/signing_channel/name"));
    if let (Some(uploader), Some(channel)) = (
        info.get("uploader_id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty()),
        info.get("channel_id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty()),
    ) && let Some(channel_url) = permanent_url(url, uploader, channel)
    {
        info.insert("channel_url".into(), channel_url.into());
    }
    info
}

/// `urlhandle_detect_ext(urlh)`.
fn response_ext(response: &reqwest::blocking::Response) -> Option<String> {
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    if let Some(disposition) = header("content-disposition") {
        static FILENAME: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"filename\s*=\s*"?([^";]+)"?"#).unwrap());
        if let Some(ext) = FILENAME
            .captures(&disposition)
            .and_then(|found| super::direct::determine_ext(&found[1]))
        {
            return Some(ext);
        }
    }
    if let Some(ext) =
        header("x-amz-meta-name").and_then(|name| super::direct::determine_ext(&name))
    {
        return Some(ext);
    }
    if let Some(kind) = header("x-amz-meta-file-type") {
        return Some(kind);
    }
    header("content-type")
        .map(|mime| super::direct::mimetype2ext_or(&mime, None))
        .filter(|ext| !ext.is_empty())
}

impl Extractor for Lbry {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, page: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        let mut display_id = temp_id.clone();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        display_id = if display_id.starts_with('@') {
            display_id.replace(':', "#")
        } else {
            display_id.replace('/', ":")
        };
        display_id = percent_encoding::percent_decode_str(&display_id)
            .decode_utf8_lossy()
            .into_owned();
        let uri = format!("lbry://{display_id}");
        let resolved = call_api(
            client,
            (NAME, "resolve"),
            &display_id,
            Some(&temp_id),
            json!({"urls": uri}),
            "stream",
        )?;
        let result = resolved
            .get(&uri)
            .cloned()
            .ok_or_else(|| key_error(&temp_id, &uri))?;
        let headers = [("Referer", "https://odysee.com/")];
        let stream_type = result.pointer("/value/stream_type").and_then(Value::as_str);
        let mut formats = Vec::new();
        let mut final_url: Option<String> = None;
        let (claim_id, is_live) = if matches!(stream_type, Some("video" | "audio")) {
            let claim_id = result
                .get("claim_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let mut params = Map::new();
            params.insert("uri".into(), uri.clone().into());
            for key in ["signature", "signature_ts"] {
                if let Some((_, value)) = page
                    .query_pairs()
                    .find(|(name, value)| name == key && !value.is_empty())
                {
                    params.insert(key.into(), value.into_owned().into());
                }
            }
            let streaming = call_api(
                client,
                (NAME, "get"),
                &claim_id,
                Some(&temp_id),
                Value::Object(params),
                "streaming url",
            )?;
            let streaming_url = streaming
                .get("streaming_url")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    fail(
                        &claim_id,
                        "An extractor error has occurred. (caused by KeyError('streaming_url'))",
                    )
                })?
                .to_owned();
            // The v3 API answers with the original file when available.
            let direct_url = API_VERSION.replace(&streaming_url, "/api/v3/").into_owned();
            crate::screen::screen(&format!(
                "[{NAME}] {display_id}: Checking for original quality"
            ));
            let original = Url::parse(&direct_url).ok().and_then(|direct| {
                super::direct::request_with_headers(client, &direct, None, &headers).ok()
            });
            if let Some(response) = original
                && response_ext(&response).as_deref() != Some("m3u8")
            {
                let value = result.get("value").cloned().unwrap_or(Value::Null);
                let ext = value
                    .pointer("/source/name")
                    .and_then(Value::as_str)
                    .and_then(super::direct::determine_ext)
                    .or_else(|| {
                        value
                            .pointer("/source/media_type")
                            .and_then(Value::as_str)
                            .map(|mime| super::direct::mimetype2ext_or(mime, None))
                            .filter(|ext| !ext.is_empty())
                    });
                let mut format = Map::new();
                format.insert("url".into(), direct_url.clone().into());
                format.insert("format_id".into(), "original".into());
                format.insert("quality".into(), 1.into());
                let mut keys = vec!["url", "format_id", "quality"];
                if let Some(ext) = &ext {
                    format.insert("ext".into(), ext.clone().into());
                    keys.push("ext");
                }
                for (key, path) in [
                    ("filesize", "/source/size"),
                    ("width", "/video/width"),
                    ("height", "/video/height"),
                ] {
                    if let Some(number) = int(value.pointer(path)) {
                        format.insert(key.into(), number);
                        keys.push(key);
                    }
                }
                if stream_type == Some("audio") {
                    format.insert("vcodec".into(), "none".into());
                }
                keys.push("vcodec");
                if !format.contains_key("ext") {
                    format.insert(
                        "ext".into(),
                        super::direct::determine_ext(&direct_url)
                            .unwrap_or_else(|| "unknown_video".into())
                            .into(),
                    );
                }
                let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
                    .map_err(|error| fail(&display_id, &error.to_string()))?;
                crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
                crate::info_order::mark_int_rates(&mut format);
                formats.push(format);
            }
            crate::screen::screen(&format!(
                "[{NAME}] {display_id}: Downloading streaming redirect url info"
            ));
            if let Ok(response) = Url::parse(&streaming_url)
                .map_err(Error::from)
                .and_then(|stream| super::direct::head_with_headers(client, &stream, &headers))
            {
                final_url = Some(response.url().to_string());
            }
            (claim_id, false)
        } else if result.get("value_type").and_then(Value::as_str) == Some("stream")
            && stream_type != Some("binary")
        {
            let claim_id = result
                .pointer("/signing_channel/claim_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let mut api = Url::parse("https://api.odysee.live/livestream/is_live")?;
            api.query_pairs_mut()
                .append_pair("channel_claim_id", &claim_id);
            let live = super::download_json_note(
                client,
                &api,
                NAME,
                &claim_id,
                "Downloading livestream JSON metadata",
            )?;
            let data = live.get("data").cloned().unwrap_or(Value::Null);
            if data.get("Live").and_then(Value::as_bool) != Some(true) {
                return Err(fail(&claim_id, "This stream is not live"));
            }
            final_url = data
                .get("VideoURL")
                .and_then(Value::as_str)
                .map(str::to_owned);
            (claim_id, true)
        } else {
            return Err(Error::Reported(format!("Unsupported URL: {page}")));
        };
        if let Some(final_url) = &final_url
            && super::direct::determine_ext(final_url).as_deref() == Some("m3u8")
        {
            let manifest = Url::parse(final_url)?;
            let mut stream = super::extract_m3u8_fatal(
                client,
                &manifest,
                NAME,
                &display_id,
                Some("hls"),
                false,
            )?;
            if is_live {
                super::hls::live_format_ids(&mut stream.formats, Some("hls"));
            }
            formats.extend(stream.formats);
        }
        let mut dict = parse_stream(&result, page.as_str());
        dict.insert("id".into(), claim_id.into());
        dict.insert("formats".into(), json!([]));
        dict.insert("is_live".into(), is_live.into());
        dict.insert(
            "http_headers".into(),
            json!({"Referer": "https://odysee.com/"}),
        );
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
        info.extractor_key = KEY.into();
        let titled = dict.contains_key("title");
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
                "formats" => {}
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!(
                "dict{};{}",
                if titled { "" } else { "-untitled" },
                keys.join(",")
            ),
        )))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

pub struct LbryChannel;
pub struct LbryPlaylist;

const CHANNEL_NAME: &str = "lbry:channel";
const CHANNEL_KEY: &str = "LBRYChannel";
const PLAYLIST_NAME: &str = "lbry:playlist";
const PLAYLIST_KEY: &str = "LBRYPlaylist";
/// `LBRYBaseIE._PAGE_SIZE`.
const PAGE_SIZE: usize = 50;

static CHANNEL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:https?://(?:www\.)?(?:lbry\.tv|odysee\.com)/|lbry://)(?P<id>@[^$@:/?#&]+(?:[:#]{CLAIM})?)/?(?:[?&]|$)"
    ))
    .unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:https?://(?:www\.)?(?:lbry\.tv|odysee\.com)/|lbry://)\$/(?:play)?list/(?P<id>[0-9a-f-]+)",
    )
    .unwrap()
});

/// `LBRYChannelIE.suitable`.
pub fn channel_suitable(url: &str) -> bool {
    CHANNEL_URL.is_match(url)
}

/// `LBRYPlaylistIE.suitable`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("LBRY {what} are not supported natively"))
}

/// A `claim_search` item's `{..._parse_stream(item, url), '_type': 'url',
/// 'id', 'url', 'ie_key': 'LBRY'}` entry.
fn url_entry(raw: Map<String, Value>) -> Result<VideoInfo> {
    let target = raw
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("https://lbry.tv/None")
        .to_owned();
    let url = super::target_url(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = raw
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    info.title = raw
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": Value::Object(raw),
        }),
    );
    Ok(info)
}

/// Whether a playlist's pending state is an LBRY channel's or playlist's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == "lbry"
}

/// `_fetch_page(display_id, url, params, page)`: the page's `claim_search`
/// items as `url` entries.
fn fetch_page(client: &Client, state: &Value, page: usize) -> Result<Vec<VideoInfo>> {
    let name = if state["name"] == PLAYLIST_NAME {
        PLAYLIST_NAME
    } else {
        CHANNEL_NAME
    };
    let url = state["url"].as_str().unwrap_or_default();
    let display_id = state["display_id"].as_str().unwrap_or_default();
    let page = page + 1;
    let mut page_params = Map::new();
    page_params.insert("no_totals".into(), true.into());
    page_params.insert("page".into(), page.into());
    page_params.insert("page_size".into(), PAGE_SIZE.into());
    if let Some(params) = state["params"].as_object() {
        page_params.extend(params.clone());
    }
    let result = call_api(
        client,
        (name, "claim_search"),
        display_id,
        None,
        Value::Object(page_params),
        &format!("page {page}"),
    )?;
    let mut entries = Vec::new();
    // `('items', lambda _, v: v['name'] and v['claim_id'])`.
    for item in result
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(claim_name), Some(claim_id)) = (item.get("name"), item.get("claim_id")) else {
            continue;
        };
        if !super::nuxt::truthy(claim_name) || !super::nuxt::truthy(claim_id) {
            continue;
        }
        let (Some(claim_name), Some(claim_id)) = (claim_name.as_str(), claim_id.as_str()) else {
            return Err(unsupported("claim names and IDs that are not strings"));
        };
        let mut raw = parse_stream(item, url);
        raw.insert("_type".into(), "url".into());
        raw.insert("id".into(), claim_id.into());
        raw.insert(
            "url".into(),
            permanent_url(url, claim_name, claim_id).map_or(Value::Null, Value::from),
        );
        raw.insert("ie_key".into(), KEY.into());
        entries.push(url_entry(raw)?);
    }
    Ok(entries)
}

/// `OnDemandPagedList(fetch_page, 50)` read by upstream's `PlaylistEntries`
/// (see [`super::paged`]).
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let page_state = state.clone();
    let mut fetch = |page| fetch_page(client, &page_state, page);
    let placeholder = || url_entry(Map::new());
    super::paged::fetch_requested(state, limit, entries, PAGE_SIZE, &mut fetch, &placeholder)
}

/// `_playlist_entries(url, display_id, claim_param, metadata)`: the
/// `claim_search` parameters from the URL's query and the pending playlist.
fn playlist_entries(
    page: &Url,
    (name, key, temp_id): (&'static str, &'static str, &str),
    display_id: String,
    claim_param: (&str, Value),
    metadata: Option<&Value>,
) -> Result<ExtractResult> {
    // `parse_qs(url)`'s first non-blank value.
    let first = |param: &str| {
        page.query_pairs()
            .find(|(name, value)| name == param && !value.is_empty())
            .map(|(_, value)| value.into_owned())
    };
    let content = first("content");
    let mut params = Map::new();
    params.insert(
        "fee_amount".into(),
        first("fee_amount").unwrap_or_else(|| ">=0".into()).into(),
    );
    let order = first("order").unwrap_or_else(|| "new".into());
    let order_by = match order.as_str() {
        "new" => json!(["release_time"]),
        "top" => json!(["effective_amount"]),
        "trending" => json!(["trending_group", "trending_mixed"]),
        other => return Err(key_error(temp_id, other)),
    };
    params.insert("order_by".into(), order_by);
    params.insert("claim_type".into(), "stream".into());
    params.insert(
        "stream_types".into(),
        match content.as_deref() {
            Some(content @ ("audio" | "video")) => json!([content]),
            _ => json!(["video", "audio"]),
        },
    );
    params.insert(claim_param.0.into(), claim_param.1);
    if let Some(duration) = first("duration") {
        let duration = match duration.as_str() {
            "long" => ">=1200",
            "short" => "<=240",
            other => return Err(key_error(temp_id, other)),
        };
        params.insert("duration".into(), duration.into());
    }
    let language = first("language").unwrap_or_else(|| "all".into());
    if language != "all" {
        let mut languages = vec![Value::from(language.clone())];
        if language == "en" {
            languages.push("none".into());
        }
        params.insert("any_languages".into(), languages.into());
    }

    // `traverse_obj(metadata, ('value', {'title': 'title', 'description':
    // 'description'}))` as `playlist_result` keyword fields.
    let mut keys = Vec::new();
    let mut title = String::new();
    let mut extra = Map::new();
    for field in ["title", "description"] {
        let Some(value) = metadata
            .and_then(|metadata| metadata.get("value"))
            .and_then(|value| value.get(field))
            .filter(|value| !value.is_null())
        else {
            continue;
        };
        keys.push(field);
        match (field, value) {
            ("title", Value::String(text)) => title = text.clone(),
            _ => {
                extra.insert(field.into(), value.clone());
            }
        }
    }
    keys.push("id");
    // `OnDemandPagedList`, whose full count, like a generator's, is unknown
    // unless a selection reads past its end.
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
        "kind": "lbry",
        "name": name,
        "url": page.as_str(),
        "display_id": display_id,
        "params": params,
        "random_access": true,
    });
    Ok(ExtractResult::Playlist(crate::PlaylistInfo {
        id: display_id,
        title,
        webpage_url: page.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: Some(crate::PendingEntries { state }),
        entries: Vec::new(),
        extra,
    }))
}

impl Extractor for LbryChannel {
    fn key(&self) -> &'static str {
        CHANNEL_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CHANNEL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        announce(CHANNEL_NAME, url);
        let display_id = temp_id.replace(':', "#");
        let uri = format!("lbry://{display_id}");
        let resolved = call_api(
            client,
            (CHANNEL_NAME, "resolve"),
            &display_id,
            Some(&temp_id),
            json!({"urls": uri}),
            "channel",
        )?;
        let result = resolved
            .get(&uri)
            .ok_or_else(|| key_error(&temp_id, &uri))?;
        let claim_id = match result.get("claim_id") {
            Some(Value::String(claim_id)) => claim_id.clone(),
            Some(_) => return Err(unsupported("channel claim IDs that are not strings")),
            None => return Err(key_error(&temp_id, "claim_id")),
        };
        playlist_entries(
            url,
            (CHANNEL_NAME, CHANNEL_KEY, &temp_id),
            claim_id.clone(),
            ("channel_ids", json!([claim_id])),
            Some(result),
        )
        .map(Some)
    }
}

impl Extractor for LbryPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        announce(PLAYLIST_NAME, url);
        let mut params = Map::new();
        params.insert("claim_ids".into(), json!([display_id]));
        params.insert("no_totals".into(), true.into());
        params.insert("page".into(), 1.into());
        params.insert("page_size".into(), PAGE_SIZE.into());
        let search = call_api(
            client,
            (PLAYLIST_NAME, "claim_search"),
            &display_id,
            Some(&display_id),
            Value::Object(params),
            "playlist",
        )?;
        // `traverse_obj(..., ('items', 0))`.
        let result = search
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .filter(|item| !item.is_null());
        // `('value', 'claims', ..., {str})`.
        let claims: Vec<Value> = result
            .and_then(|result| result.pointer("/value/claims"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|claim| claim.is_string())
            .cloned()
            .collect();
        playlist_entries(
            url,
            (PLAYLIST_NAME, PLAYLIST_KEY, &display_id),
            display_id.clone(),
            ("claim_ids", Value::Array(claims)),
            result,
        )
        .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://lbry.tv/@Mantega:1/First-day-LBRY:1"));
        assert!(suitable(
            "https://odysee.com/@gardeningincanada:b/plants-i-will-never-grow-again.-the:e"
        ));
        assert!(suitable(
            "https://odysee.com/$/embed/@Mantega:1/First-day-LBRY:1"
        ));
        assert!(suitable("lbry://@lbry#3f/odysee#7"));
        assert!(suitable(
            "https://odysee.com/Episode-1:e7d93d772bd87e2b62d5ab993c1c3ced86ebb396"
        ));
        assert!(!suitable("https://example.com/@x"));
    }

    #[test]
    fn playlist_urls_follow_upstream_valid_url() {
        for url in [
            "https://lbry.tv/@LBRYFoundation:0",
            "https://lbry.tv/@LBRYFoundation",
            "lbry://@lbry#3f",
            "https://odysee.com/@LBRYFoundation:0/?order=top",
            "https://odysee.com/@LBRYFoundation?content=audio",
        ] {
            assert!(channel_suitable(url), "{url}");
            assert!(!suitable(url), "{url}");
        }
        assert!(!channel_suitable(
            "https://lbry.tv/@Mantega:1/First-day-LBRY:1"
        ));
        for url in [
            "https://odysee.com/$/playlist/ffef782f27486f0ac138bde8777f72ebdd0548c2",
            "https://odysee.com/$/list/9c6658b3dd21e4f2a0602d523a13150e2b48b770",
            "https://odysee.com/$/playlist/938fb11d-215f-4d1c-ad64-723954df2184",
        ] {
            assert!(playlist_suitable(url), "{url}");
            assert!(!suitable(url), "{url}");
        }
    }
}
