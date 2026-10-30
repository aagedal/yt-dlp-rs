//! Upstream `XiaoHongShuIE` (`yt_dlp/extractor/xiaohongshu.py`, release
//! 2026.10.21): the note page's `window.__INITIAL_STATE__` streams (every
//! codec's `masterUrl` and `backupUrls`), the original upload when
//! `sns-video-bd.xhscdn.com` serves it, the note's images as thumbnails and
//! its title, description, tags and uploader ID.
use super::{Extractor, download_webpage, search_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct XiaoHongShu;

const NAME: &str = "XiaoHongShu";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://www\.xiaohongshu\.com/(?:explore|discovery/item)/(?P<id>[\da-f]+)",
    )
    .unwrap()
});

static INITIAL_STATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__INITIAL_STATE__\s*=").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn report(id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("XiaoHongShu {what} are not supported natively"))
}

/// `traverse_obj`'s discarded values: `None` and `{}`.
fn present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Object(map) => !map.is_empty(),
        _ => true,
    }
}

/// `traverse_obj(obj, (key, ...))` from a mapping: `obj[key]`'s values (a
/// mapping's values, a list's items, nothing for anything else).
fn branch(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Object(map)) => map.values().collect(),
        Some(Value::Array(items)) => items.iter().collect(),
        _ => Vec::new(),
    }
}

/// Python `int(v)` of a JSON value.
fn py_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(value) => Some(i64::from(*value)),
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|value| value.is_finite() && value.abs() < 9.2e18)
                .map(|value| value.trunc() as i64)
        }),
        Value::String(text) => {
            let text = text.trim();
            let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
            if digits.is_empty()
                || digits.starts_with('_')
                || digits.ends_with('_')
                || digits.contains("__")
                || !digits.chars().all(|c| c.is_ascii_digit() || c == '_')
            {
                return None;
            }
            text.replace('_', "").parse().ok()
        }
        _ => None,
    }
}

/// `int_or_none(v, scale=scale)`: `int(v) // scale`.
fn int_or_none(value: Option<&Value>, scale: i64) -> Option<i64> {
    value.and_then(py_int).map(|value| value.div_euclid(scale))
}

/// `float_or_none(v, scale=scale)`.
fn float_or_none(value: Option<&Value>, scale: f64) -> Option<f64> {
    let value = match value? {
        Value::Bool(value) => f64::from(u8::from(*value)),
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    Some(value / scale)
}

/// Upstream `url_or_none` (`https?`, `rtmp`-family, `ftps?`, `wss?` or
/// protocol-relative URLs).
fn url_or_none(value: &Value) -> Option<String> {
    static URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:(?:https?|rtm(?:pt?[es]?|fp)|ftps?|wss?):)?//").unwrap());
    let text = value.as_str()?.trim();
    URL.is_match(text).then(|| text.to_owned())
}

/// A format URL as processing's `sanitize_url` leaves it: only HTTP(S)
/// downloads are supported natively.
fn format_url(url: &str) -> Result<Url> {
    let url = Url::parse(&super::proto_relative_http(url.trim()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(unsupported("non-HTTP stream URLs"));
    }
    Ok(url)
}

/// One `{'url': u, **format_info}` dict as a format.
fn stream_format(id: &str, dict: Map<String, Value>) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = dict.keys().cloned().collect();
    let mut format = dict;
    let url = format_url(format["url"].as_str().unwrap_or_default())?;
    let ext = super::direct::determine_ext(url.as_str()).unwrap_or_else(|| "unknown_video".into());
    format.insert("url".into(), url.as_str().into());
    format.insert("ext".into(), ext.into());
    format.insert("format_id".into(), "".into());
    let mut format: crate::VideoFormat =
        serde_json::from_value(Value::Object(format)).map_err(|error| {
            Error::UnsupportedHtml5(format!(
                "XiaoHongShu stream values out of range are not supported natively ({id}: {error})"
            ))
        })?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_rates(&mut format);
    Ok(format)
}

impl Extractor for XiaoHongShu {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        let state = match search_json(&INITIAL_STATE, &webpage, true) {
            None => return Err(report(&id, "Unable to extract initial state")),
            Some(None) => {
                return Err(report(
                    &id,
                    "Unable to extract initial state - Failed to parse JSON",
                ));
            }
            Some(Some(state)) => state,
        };
        // `traverse_obj(initial_state, ('note', 'noteDetailMap', display_id, 'note'))`.
        let note = state
            .get("note")
            .and_then(|note| note.get("noteDetailMap"))
            .and_then(|map| map.get(&id))
            .and_then(|detail| detail.get("note"))
            .and_then(Value::as_object)
            .filter(|note| !note.is_empty());
        let note_get = |key: &str| note.and_then(|note| note.get(key));

        // `traverse_obj(note_info, ('video', 'media', 'stream', ..., ...))`.
        let streams: Vec<&Value> = branch(
            note_get("video")
                .and_then(|video| video.get("media"))
                .and_then(|media| media.get("stream")),
        )
        .into_iter()
        .filter(|value| present(value))
        .flat_map(|codec| branch(Some(codec)))
        .filter(|value| present(value))
        .collect();

        let mut formats = Vec::new();
        for stream in streams {
            let Value::Object(stream) = stream else {
                continue;
            };
            let get = |key: &str| stream.get(key);
            let text = |key: &str| get(key).and_then(Value::as_str).map(Value::from);
            let int = |key: &str, scale: i64| int_or_none(get(key), scale).map(Value::from);
            let mut info = Map::new();
            for (key, value) in [
                ("fps", int("fps", 1)),
                ("width", int("width", 1)),
                ("height", int("height", 1)),
                ("vcodec", text("videoCodec")),
                ("acodec", text("audioCodec")),
                ("abr", int("audioBitrate", 1000)),
                ("vbr", int("videoBitrate", 1000)),
                ("audio_channels", int("audioChannels", 1)),
                ("tbr", int("avgBitrate", 1000)),
                ("format", text("qualityType")),
                ("filesize", int("size", 1)),
                (
                    "duration",
                    float_or_none(get("duration"), 1000.0).map(Value::from),
                ),
            ] {
                if let Some(value) = value {
                    info.insert(key.into(), value);
                }
            }
            // `(('masterUrl', ('backupUrls', ...)), {lambda u: url_or_none(u) and ...})`.
            let mut urls = vec![get("masterUrl")];
            urls.extend(branch(get("backupUrls")).into_iter().map(Some));
            for candidate in urls.into_iter().flatten() {
                if !present(candidate) || url_or_none(candidate).is_none() {
                    continue;
                }
                let mut dict = Map::new();
                dict.insert("url".into(), candidate.clone());
                dict.extend(info.clone());
                formats.push(stream_format(&id, dict)?);
            }
        }

        // `consumer.originVideoKey`: the original upload, if it is served.
        if let Some(origin_key) = note_get("video")
            .and_then(|video| video.get("consumer"))
            .and_then(|consumer| consumer.get("originVideoKey"))
            .and_then(Value::as_str)
        {
            let origin = Url::parse(&format!("https://sns-video-bd.xhscdn.com/{origin_key}"))?;
            crate::screen::screen(&format!(
                "[{NAME}] {id}: Checking original video availability"
            ));
            match super::direct::request(client, &origin, None) {
                Ok(response) => {
                    let ext = super::direct::urlhandle_detect_ext(&response, Some("mp4"));
                    let filesize = response
                        .headers()
                        .get(reqwest::header::CONTENT_LENGTH)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| py_int(&Value::from(value)));
                    let final_url = response.url().clone();
                    drop(response);
                    let filesize = match filesize {
                        Some(size) => Value::from(
                            u64::try_from(size)
                                .map_err(|_| unsupported("negative Content-Lengths"))?,
                        ),
                        None => Value::Null,
                    };
                    let mut format: crate::VideoFormat =
                        serde_json::from_value(serde_json::json!({
                            "format_id": "direct",
                            "ext": ext,
                            "filesize": filesize,
                            "url": final_url.as_str(),
                            "quality": 1,
                        }))
                        .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
                    crate::info_order::mark_format(
                        &mut format,
                        "dict:format_id,ext,filesize,url,quality",
                    );
                    crate::info_order::mark_int_quality(&mut format);
                    formats.push(format);
                }
                Err(error) => crate::screen::warning(&format!(
                    "[{NAME}] {id}: Original video is not available: {}",
                    crate::upstream_error_text(&error)
                )),
            }
        }

        let mut thumbnails = Vec::new();
        for image in branch(note_get("imageList"))
            .into_iter()
            .filter(|value| present(value))
        {
            let Value::Object(image) = image else {
                continue;
            };
            let mut sizes = Map::new();
            for key in ["height", "width"] {
                if let Some(size) = int_or_none(image.get(key), 1) {
                    sizes.insert(key.into(), size.into());
                }
            }
            for key in ["urlDefault", "urlPre"] {
                if let Some(thumb_url) = image.get(key).and_then(url_or_none) {
                    let mut thumbnail = Map::new();
                    thumbnail.insert("url".into(), thumb_url.into());
                    thumbnail.extend(sizes.clone());
                    thumbnails.push(Value::Object(thumbnail));
                }
            }
        }

        // `_html_search_meta(['og:title'], webpage, default=None)`, then the
        // note's own fields.
        let mut dict = Map::new();
        dict.insert("id".into(), id.clone().into());
        dict.insert("formats".into(), Value::Null);
        dict.insert("thumbnails".into(), Value::Array(thumbnails));
        dict.insert(
            "title".into(),
            crate::html::html_search_meta(&webpage, "og:title").map_or(Value::Null, Value::from),
        );
        for (key, value) in [
            ("title", note_get("title").filter(|value| value.is_string())),
            (
                "description",
                note_get("desc").filter(|value| value.is_string()),
            ),
        ] {
            if let Some(value) = value {
                dict.insert(key.into(), value.clone());
            }
        }
        let tags: Vec<Value> = branch(note_get("tagList"))
            .into_iter()
            .filter_map(|tag| tag.get("name"))
            .filter(|name| name.is_string())
            .cloned()
            .collect();
        if !tags.is_empty() {
            dict.insert("tags".into(), Value::Array(tags));
        }
        if let Some(uploader_id) = note_get("user")
            .and_then(|user| user.get("userId"))
            .filter(|value| value.is_string())
        {
            dict.insert("uploader_id".into(), uploader_id.clone());
        }

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
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let mut keys = Vec::new();
        for (key, value) in dict {
            keys.push(key.clone());
            match key.as_str() {
                "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
                "title" => match value.as_str() {
                    Some(title) => info.title = title.to_owned(),
                    None => {
                        info.title = String::new();
                        info.extra
                            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
                    }
                },
                "formats" => {}
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("dict;{};thumbnail=url,height,width", keys.join(",")),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.xiaohongshu.com/explore/6411cf99000000001300b6d9"
        ));
        assert!(suitable(
            "https://www.xiaohongshu.com/discovery/item/674051740000000007027a15?xsec_token=x"
        ));
        assert!(!suitable("https://xiaohongshu.com/explore/6411cf99"));
        assert!(!suitable("https://www.xiaohongshu.com/user/profile/5c31"));
    }

    #[test]
    fn numbers_follow_python_conversions() {
        assert_eq!(int_or_none(Some(&json!("64000")), 1000), Some(64));
        assert_eq!(int_or_none(Some(&json!(" 1_000 ")), 1), Some(1000));
        assert_eq!(int_or_none(Some(&json!("1.5")), 1), None);
        assert_eq!(int_or_none(Some(&json!(25.9)), 1), Some(25));
        assert_eq!(int_or_none(Some(&json!(-1500)), 1000), Some(-2));
        assert_eq!(int_or_none(Some(&json!(null)), 1), None);
        assert_eq!(float_or_none(Some(&json!("28073")), 1000.0), Some(28.073));
    }

    #[test]
    fn urls_follow_upstream_url_or_none() {
        assert!(url_or_none(&json!("//cdn.test/a.mp4")).is_some());
        assert!(url_or_none(&json!(" https://cdn.test/a.mp4 ")).is_some());
        assert!(url_or_none(&json!("rtsp://cdn.test/a")).is_none());
        assert!(url_or_none(&json!("not a url")).is_none());
        assert!(url_or_none(&json!(5)).is_none());
    }
}
