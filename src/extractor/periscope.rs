//! Upstream `PeriscopeBaseIE` and `PeriscopeIE` (`yt_dlp/extractor/periscope.py`):
//! a broadcast's metadata (`_parse_broadcast_data`, shared with
//! `TwitterBroadcastIE`) and its HLS formats with the Periscope `Referer`.
//! `PeriscopeUserIE` and the `_EMBED_REGEX` embeds are not ported.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Periscope;

const NAME: &str = "periscope";
const KEY: &str = "Periscope";
/// `PeriscopeBaseIE._M3U8_HEADERS`.
pub(super) const M3U8_REFERER: &str = "https://www.periscope.tv/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?(?:periscope|pscp)\.tv/[^/?#]+/(?P<id>[^/?#]+)").unwrap()
});

/// `PeriscopeIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none(value, scale=scale)`.
pub(super) fn int(value: Option<&Value>, scale: i64) -> Option<i64> {
    let value = value?;
    // `int(v) // scale`: a float is truncated, a text must be an integer.
    let number = match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|number| number.trunc() as i64))?,
        Value::String(text) => text.trim().parse::<i64>().ok()?,
        Value::Bool(flag) => i64::from(*flag),
        _ => return None,
    };
    Some(number.div_euclid(scale))
}

/// `(KEY, {clean_html}, filter)`: the cleaned text when not empty.
fn clean(value: Option<&Value>) -> Option<String> {
    Some(crate::html::clean_html(value?.as_str()?)).filter(|text| !text.is_empty())
}

/// `PeriscopeBaseIE._parse_broadcast_data(broadcast, video_id)`.
pub(super) fn parse_broadcast_data(broadcast: &Value, video_id: &str) -> Map<String, Value> {
    let mut info = Map::new();
    info.insert("display_id".into(), video_id.into());
    let state = broadcast
        .get("state")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    info.insert(
        "live_status".into(),
        match state.as_deref() {
            Some("running") => "is_live",
            Some("not_started") => "is_upcoming",
            _ => "was_live",
        }
        .into(),
    );
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            info.insert(key.into(), value);
        }
    };
    put(
        "id",
        match broadcast.get("id") {
            Some(Value::String(text)) => Some(text.clone().into()),
            Some(Value::Number(number)) => Some(number.to_string().into()),
            _ => None,
        },
    );
    put("title", clean(broadcast.get("status")).map(Value::from));
    put(
        "concurrent_view_count",
        int(broadcast.get("total_watching"), 1).map(Value::from),
    );
    put(
        "release_timestamp",
        ["scheduled_start_ms", "start_ms"]
            .iter()
            .find_map(|key| int(broadcast.get(*key), 1000))
            .map(Value::from),
    );
    let tags: Vec<Value> = broadcast
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tag| clean(Some(tag)))
        .map(Value::from)
        .collect();
    put("tags", (!tags.is_empty()).then_some(Value::Array(tags)));
    let thumbnails: Vec<Value> = ["image_url_small", "image_url_medium", "image_url"]
        .iter()
        .filter_map(|key| broadcast.get(*key)?.as_str())
        .filter_map(super::json_ld::url_or_none)
        .map(|url| serde_json::json!({"url": url}))
        .collect();
    put(
        "thumbnails",
        (!thumbnails.is_empty()).then_some(Value::Array(thumbnails)),
    );
    put(
        "timestamp",
        broadcast
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .or_else(|| int(broadcast.get("created_at_ms"), 1000))
            .map(Value::from),
    );
    put(
        "uploader",
        clean(broadcast.get("user_display_name")).map(Value::from),
    );
    put(
        "uploader_id",
        clean(broadcast.get("username")).map(Value::from),
    );
    put(
        "view_count",
        int(broadcast.get("total_watched"), 1).map(Value::from),
    );
    info
}

/// `_extract_common_format_info(broadcast)`: the lowercased state, width
/// and height.
pub(super) fn common_format_info(
    broadcast: &Value,
    extractor: &str,
    id: &str,
) -> Result<(String, Option<i64>, Option<i64>)> {
    let state = broadcast
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{extractor}] {id}: An extractor error has occurred. (caused by AttributeError(\"'NoneType' object has no attribute 'lower'\")); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    Ok((
        state.to_lowercase(),
        int(broadcast.get("width"), 1),
        int(broadcast.get("height"), 1),
    ))
}

/// `_extract_m3u8_formats(url, video_id, ext, entry_protocol=...,
/// m3u8_id=..., live=..., fatal=..., headers={'Referer': referer})`.
#[allow(clippy::too_many_arguments)]
pub(super) fn m3u8_formats(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    (ext, entry_protocol): (&str, &str),
    m3u8_id: Option<&str>,
    live: bool,
    fatal: bool,
    referer: &str,
) -> Result<Vec<VideoFormat>> {
    let referer = Url::parse(referer)?;
    let note = format!("[{extractor}] {id}: Downloading m3u8 information");
    crate::screen::expect_page(id, Some(note.clone()), None);
    let response = super::direct::request(client, manifest, Some(&referer));
    if response.is_err() {
        crate::screen::screen(&note);
    }
    let parsed = response.and_then(|response| {
        super::hls::from_response_with(
            client,
            response,
            manifest,
            Some(&referer),
            true,
            m3u8_id,
            live,
        )
    });
    crate::screen::clear_page();
    let mut stream =
        match parsed {
            Ok(Some(stream)) => stream,
            Ok(None) => return Ok(Vec::new()),
            Err(error) if fatal => {
                return Err(match error {
                    Error::Http(_) | Error::HttpRetryAfter { .. } => Error::Reported(
                        super::request_error_text(extractor, id, "m3u8 information", &error)
                            .replacen(": Unable to download ", ": Failed to download ", 1),
                    ),
                    error => error,
                });
            }
            Err(error) => {
                crate::screen::warning(&format!(
                    "[{extractor}] {id}: Failed to download m3u8 information: {}",
                    crate::upstream_error_text(&error)
                ));
                return Ok(Vec::new());
            }
        };
    super::ignore_subtitles(&mut stream, extractor, "HLS");
    if live {
        super::hls::live_format_ids(&mut stream.formats, m3u8_id);
    }
    for format in &mut stream.formats {
        format.ext = ext.to_owned();
        format.protocol = Some(entry_protocol.to_owned());
    }
    Ok(stream.formats)
}

/// `_extract_pscp_m3u8_formats(m3u8_url, video_id, format_id, state,
/// width, height, fatal)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn pscp_m3u8_formats(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    format_id: Option<&str>,
    (state, width, height): (&str, Option<i64>, Option<i64>),
    fatal: bool,
) -> Result<Vec<VideoFormat>> {
    let entry_protocol = if matches!(state, "ended" | "timed_out") {
        "m3u8_native"
    } else {
        "m3u8"
    };
    let mut formats = m3u8_formats(
        client,
        manifest,
        extractor,
        id,
        ("mp4", entry_protocol),
        format_id,
        false,
        fatal,
        M3U8_REFERER,
    )?;
    // `_add_width_and_height`: only a single format.
    if let [format] = formats.as_mut_slice() {
        for (key, value) in [("width", width), ("height", height)] {
            let slot = if key == "width" {
                &mut format.width
            } else {
                &mut format.height
            };
            if slot.is_none_or(|current| current == 0) {
                let update = if slot.is_none() {
                    format!("+{key}")
                } else {
                    key.to_owned()
                };
                *slot = value.and_then(|value| u32::try_from(value).ok());
                crate::info_order::mark_format_update(format, &update);
            }
        }
    }
    for format in &mut formats {
        let mut headers = format.http_headers.take().unwrap_or_default();
        headers.insert("Referer".into(), M3U8_REFERER.into());
        format.http_headers = Some(headers);
        crate::info_order::mark_format_update(format, "http_headers");
    }
    Ok(formats)
}

/// An info dict built from an extractor's dict in its key order.
pub(super) fn build(
    url: &Url,
    (extractor, key): (&'static str, &'static str),
    dict: Map<String, Value>,
    formats: Vec<VideoFormat>,
    thumbnail_keys: &str,
) -> VideoInfo {
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
    info.extractor = extractor.into();
    info.extractor_key = key.into();
    let mut keys = Vec::new();
    for (name, value) in dict {
        keys.push(name.clone());
        match name.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
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
                info.extra.insert(name, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &name, value),
        }
    }
    crate::info_order::mark_info(
        info,
        &format!("dict;{};thumbnail={thumbnail_keys}", keys.join(",")),
    )
}

impl Extractor for Periscope {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let token = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `PeriscopeBaseIE._call_api('accessVideoPublic', ...)`.
        let mut api = Url::parse("https://api.periscope.tv/api/v2/accessVideoPublic")?;
        api.query_pairs_mut().append_pair("broadcast_id", &token);
        let stream = super::download_json(client, &api, NAME, &token)?;
        let key_error = |key: &str| {
            Error::Reported(format!(
                "[{NAME}] {token}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let broadcast = stream
            .get("broadcast")
            .ok_or_else(|| key_error("broadcast"))?;
        let mut info = parse_broadcast_data(broadcast, &token);
        let common = common_format_info(broadcast, NAME, &token)?;
        let mut seen: Vec<String> = Vec::new();
        let mut formats = Vec::new();
        for format_id in ["replay", "rtmp", "hls", "https_hls", "lhls", "lhlsweb"] {
            let Some(video_url) = stream
                .get(format!("{format_id}_url"))
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
            else {
                continue;
            };
            if seen.iter().any(|known| known == video_url) {
                continue;
            }
            seen.push(video_url.to_owned());
            if format_id == "rtmp" {
                return Err(Error::UnsupportedHtml5(
                    "Periscope RTMP formats are not supported natively".into(),
                ));
            }
            let Ok(manifest) = Url::parse(video_url) else {
                continue;
            };
            formats.extend(pscp_m3u8_formats(
                client,
                &manifest,
                NAME,
                &token,
                Some(format_id),
                (&common.0, common.1, common.2),
                false,
            )?);
        }
        info.insert("formats".into(), Value::Null);
        Ok(Some(ExtractResult::Video(build(
            url,
            (NAME, KEY),
            info,
            formats,
            "url",
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.periscope.tv/LularoeHusbandMike/1mrGmgaXAVqxy"
        ));
        assert!(suitable("https://www.periscope.tv/w/1ZkKzPbMVggJv"));
        assert!(!suitable("https://www.periscope.tv/LularoeHusbandMike/"));
    }

    #[test]
    fn broadcast_data_follows_upstream_traversal() {
        let data = parse_broadcast_data(
            &serde_json::json!({
                "state": "RUNNING", "id": "x", "tags": ["a", ""], "image_url": "nope",
                "image_url_small": "https://a/b.jpg", "created_at": "2020-06-01T01:07:18.000Z",
                "created_at_ms": 5, "start_ms": "1590973647000",
            }),
            "x",
        );
        assert_eq!(
            Value::Object(data),
            serde_json::json!({
                "display_id": "x", "live_status": "is_live", "id": "x",
                "release_timestamp": 1590973647, "tags": ["a"],
                "thumbnails": [{"url": "https://a/b.jpg"}], "timestamp": 1590973638,
            })
        );
    }
}
