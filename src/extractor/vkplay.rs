//! Upstream `VKPlayIE` and `VKPlayLiveIE` (`yt_dlp/extractor/vk.py`): VK
//! Play Live records and live streams from `api.vkplay.live`'s
//! `public_video_stream` JSON, else the page's inline `initial-state`
//! script, with their `playerUrls` HLS, DASH and progressive formats.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct VkPlay;
pub struct VkPlayLive;

const RECORD: &str = "VKPlay";
const LIVE: &str = "VKPlayLive";
const BASE_URL: &str = r"^https?://(?:vkplay\.live|live\.vk(?:play|video)\.ru)/";

static RECORD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{BASE_URL}(?P<username>[^/#?]+)/record/(?P<id>[\da-f-]+)"
    ))
    .unwrap()
});
static LIVE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{BASE_URL}(?P<id>[^/#?]+)/?(?:[#?]|$)")).unwrap());
static INITIAL_STATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script[^>]+\bid="initial-state"[^>]*>"#).unwrap());

/// `VKPlayBaseIE._RESOLUTIONS`.
const RESOLUTIONS: &[(&str, i64, i64)] = &[
    ("tiny", 256, 144),
    ("lowest", 426, 240),
    ("low", 640, 360),
    ("medium", 852, 480),
    ("high", 1280, 720),
    ("full_hd", 1920, 1080),
    ("quad_hd", 2560, 1440),
];

/// `VKPlayIE.suitable`.
pub fn suitable(url: &str) -> bool {
    RECORD_URL.is_match(url)
}

/// `VKPlayLiveIE.suitable`.
pub fn live_suitable(url: &str) -> bool {
    LIVE_URL.is_match(url)
}

fn start(name: &'static str, page: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(page.as_str())
    ));
}

/// `_download_json(url, video_id, fatal=False)`: a warning and `None` on
/// failure.
fn optional_json(
    client: &Client,
    name: &'static str,
    url: &Url,
    id: &str,
) -> Result<Option<Value>> {
    match super::download_json(client, url, name, id) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{name}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(Error::Reported(message)) if message.contains(": Failed to parse JSON ") => {
            crate::screen::warning(&message);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// `VKPlayBaseIE._extract_from_initial_state(url, video_id, path)`.
fn from_initial_state(
    client: &Client,
    name: &'static str,
    page: &Url,
    id: &str,
    path: &[&str],
) -> Result<Value> {
    let webpage = super::download_webpage(client, page, name, id)?;
    let state = match super::search_json(&INITIAL_STATE, &webpage, false) {
        Some(Some(state)) => state,
        Some(None) => {
            return Err(Error::UnsupportedHtml5(
                "VK Play initial states that do not parse are not supported natively".into(),
            ));
        }
        None => {
            return Err(Error::Reported(format!(
                "[{name}] {id}: Unable to extract initial state; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
    };
    path.iter()
        .try_fold(&state, |value, key| value.get(key))
        .filter(|info| info.as_object().is_some_and(|info| !info.is_empty()))
        .cloned()
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{name}] {id}: Unable to extract video info from html inline initial state; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
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

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.into()),
        _ => None,
    }
}

/// `int_or_none`.
fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_bool().map(i64::from))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn format_from(mut format: Map<String, Value>, id: &str, name: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = format.keys().cloned().collect();
    format.retain(|_, value| !value.is_null());
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| Error::Reported(format!("[{name}] {id}: {error}")))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    Ok(format)
}

/// `VKPlayBaseIE._extract_formats(stream_info, video_id)`.
fn extract_formats(
    client: &Client,
    name: &'static str,
    info: &Value,
    id: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let mut formats = Vec::new();
    let streams = info
        .pointer("/data/0/playerUrls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|stream| {
            stream
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .is_some()
                && stream.get("type").is_some_and(truthy)
        });
    for stream in streams {
        let url = stream["url"].as_str().unwrap_or_default();
        let format_id = str_or_none(stream.get("type")).unwrap_or_default();
        if ["hls", "live_hls", "live_playback_hls"].contains(&format_id.as_str())
            || url.contains(".m3u8")
        {
            // A live stream's media playlists have no fixed fragments.
            let live = info.get("isOnline").and_then(Value::as_bool) == Some(true);
            if let Some(stream) = Url::parse(url).ok().and_then(|manifest| {
                if live {
                    super::extract_m3u8_live_master_optional(
                        client,
                        &manifest,
                        name,
                        id,
                        Some(&format_id),
                    )
                } else {
                    super::extract_m3u8_optional(
                        client,
                        &manifest,
                        name,
                        id,
                        Some(&format_id),
                        false,
                    )
                }
            }) {
                formats.extend(stream.formats);
            }
        } else if format_id == "dash" {
            if let Some(mut stream) = Url::parse(url).ok().and_then(|manifest| {
                super::extract_mpd_optional_note(
                    client,
                    &manifest,
                    name,
                    id,
                    Some(&format_id),
                    "Downloading MPD manifest",
                )
            }) {
                super::ignore_subtitles(&mut stream, name, "DASH");
                formats.extend(stream.formats);
            }
        } else if ["live_dash", "live_playback_dash"].contains(&format_id.as_str()) {
            // `write_debug(f'Not extracting unsupported format "{format_id}"')`.
        } else {
            let mut format = Map::new();
            format.insert("url".into(), url.into());
            format.insert("ext".into(), "mp4".into());
            format.insert("format_id".into(), format_id.clone().into());
            if let Some((_, width, height)) = RESOLUTIONS
                .iter()
                .find(|(quality, _, _)| *quality == format_id)
            {
                format.insert("width".into(), (*width).into());
                format.insert("height".into(), (*height).into());
            }
            formats.push(format_from(format, id, name)?);
        }
    }
    Ok(formats)
}

/// `VKPlayBaseIE._extract_common_meta(stream_info)` (`get_all=False`).
fn common_meta(info: &Value) -> Map<String, Value> {
    let at = |path: &str| info.pointer(path);
    let mut meta = Map::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            meta.insert(key.into(), value);
        }
    };
    put("id", str_or_none(at("/id")).map(Value::from));
    put(
        "title",
        at("/title").and_then(Value::as_str).map(Value::from),
    );
    put("release_timestamp", int(at("/startTime")).map(Value::from));
    put(
        "thumbnail",
        at("/previewUrl")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map(Value::from),
    );
    put("view_count", int(at("/count/views")).map(Value::from));
    put("like_count", int(at("/count/likes")).map(Value::from));
    put(
        "categories",
        at("/category/title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .map(|title| json!([title])),
    );
    put(
        "uploader",
        ["/user/nick", "/blog/owner/nick"]
            .iter()
            .find_map(|path| at(path).and_then(Value::as_str))
            .map(Value::from),
    );
    put(
        "uploader_id",
        ["/user/id", "/blog/owner/id"]
            .iter()
            .find_map(|path| str_or_none(at(path)))
            .map(Value::from),
    );
    put("duration", int(at("/duration")).map(Value::from));
    put(
        "is_live",
        at("/isOnline").and_then(Value::as_bool).map(Value::from),
    );
    put(
        "concurrent_view_count",
        int(at("/count/viewers")).map(Value::from),
    );
    meta
}

fn video(
    page: &Url,
    (extractor, key): (&str, &str),
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
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
    info.webpage_url = page.clone();
    info.extractor = extractor.into();
    info.extractor_key = key.into();
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
    crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{}",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    )
}

impl Extractor for VkPlay {
    fn key(&self) -> &'static str {
        RECORD
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    /// `VKPlayIE._real_extract`.
    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = RECORD_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let (username, video_id) = (found["username"].to_owned(), found["id"].to_owned());
        start(RECORD, page);
        let api = Url::parse(&format!(
            "https://api.vkplay.live/v1/blog/{username}/public_video_stream/record/{video_id}"
        ))?;
        let record = optional_json(client, RECORD, &api, &video_id)?
            .and_then(|json| json.pointer("/data/record").cloned())
            .filter(|record| record.is_object() && truthy(record));
        let record = match record {
            Some(record) => record,
            None => from_initial_state(
                client,
                RECORD,
                page,
                &video_id,
                &["record", "currentRecord", "data"],
            )?,
        };
        let mut dict = common_meta(&record);
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("formats".into(), json!([]));
        let formats = extract_formats(client, RECORD, &record, &video_id)?;
        Ok(Some(ExtractResult::Video(video(
            page,
            (RECORD, RECORD),
            dict,
            formats,
        ))))
    }
}

impl Extractor for VkPlayLive {
    fn key(&self) -> &'static str {
        LIVE
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    /// `VKPlayLiveIE._real_extract`.
    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = LIVE_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let username = found["id"].to_owned();
        start(LIVE, page);
        let api = Url::parse(&format!(
            "https://api.vkplay.live/v1/blog/{username}/public_video_stream"
        ))?;
        let stream = optional_json(client, LIVE, &api, &username)?.filter(truthy);
        let stream = match stream {
            Some(stream) => stream,
            None => from_initial_state(
                client,
                LIVE,
                page,
                &username,
                &["stream", "stream", "data", "stream"],
            )?,
        };
        let formats = extract_formats(client, LIVE, &stream, &username)?;
        if formats.is_empty() && stream.get("isOnline").and_then(Value::as_bool) != Some(true) {
            return Err(Error::Reported(format!(
                "[{LIVE}] {username}: The channel is not currently live"
            )));
        }
        let mut dict = common_meta(&stream);
        if !dict.contains_key("id") {
            // Upstream's result then lacks its required `id`.
            return Err(Error::UnsupportedHtml5(
                "VK Play live streams without an ID are not supported natively".into(),
            ));
        }
        dict.insert("formats".into(), json!([]));
        Ok(Some(ExtractResult::Video(video(
            page,
            (LIVE, LIVE),
            dict,
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert!(suitable(
            "https://vkplay.live/zitsmann/record/f5e6e3b5-dc52-4d14-965d-0680dd2882da"
        ));
        assert!(suitable(
            "https://live.vkvideo.ru/lebwa/record/33a4e4ce-e3ef-49db-bb14-f006cc6fabc9/records"
        ));
        assert!(!live_suitable(
            "https://live.vkplay.ru/lebwa/record/33a4e4ce-e3ef-49db-bb14-f006cc6fabc9/records"
        ));
        assert!(live_suitable("https://vkplay.live/bayda"));
        assert!(live_suitable("https://live.vkvideo.ru/panterka?x=1"));
        assert!(!live_suitable("https://vkplay.live/bayda/other"));
    }
}
