//! Upstream `HuyaLiveIE` and `HuyaVideoIE` (`yt_dlp/extractor/huya.py`): a
//! live room's embedded `stream` JSON as FLV formats, and a video moment's
//! HLS definitions. Live streams that need the signed-secret (`encrypt`)
//! path crash upstream on Python 3 and are reported as unsupported here.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct HuyaLive;
pub struct HuyaVideo;

const LIVE_NAME: &str = "huya:live";
const LIVE_KEY: &str = "HuyaLive";
const VIDEO_NAME: &str = "huya:video";
const VIDEO_KEY: &str = "HuyaVideo";

/// `(?!(?:video/play/))` (no look-ahead in the regex crate) is applied in
/// [`live_suitable`].
static LIVE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.|m\.)?huya\.com/(?P<id>[^/#?&]+)(?:\D|$)").unwrap()
});
static VIDEO_PLAY_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.|m\.)?huya\.com/video/play/").unwrap());
static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?huya\.com/video/play/(?P<id>\d+)\.html").unwrap()
});
static STREAM_START: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"stream:\s").unwrap());
static DISPLAY_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?s)(.+?)(?:(\d+)M)?$").unwrap());

/// `HuyaLiveIE._match_valid_url`.
pub fn live_suitable(url: &str) -> bool {
    LIVE_URL.is_match(url) && !VIDEO_PLAY_PREFIX.is_match(url)
}

/// `HuyaVideoIE._match_valid_url`.
pub fn video_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Huya {what} are not supported natively"))
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|v| v != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

/// `str_or_none` for a JSON value.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(other) => Some(other.to_string()),
    }
}

/// `int_or_none` for an integer or numeric-string JSON value.
fn int_value(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|v| v as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `dict.update` for a parsed format, keeping the upstream field positions
/// for keys the format already holds.
fn update_format(format: &mut VideoFormat, key: &str, value: Value) {
    let in_place = match key {
        "format_id" => true,
        "height" => format.height.is_some(),
        "width" => format.width.is_some(),
        "quality" => format.quality.is_some(),
        "filesize" => format.filesize.is_some(),
        _ => false,
    };
    if in_place {
        crate::info_order::mark_format_update_in_place(format, key);
    } else {
        crate::info_order::mark_format_update(format, key);
    }
    match key {
        "format_id" => format.format_id = value.as_str().unwrap_or_default().to_owned(),
        "height" => format.height = value.as_u64().and_then(|v| u32::try_from(v).ok()),
        "width" => format.width = value.as_u64().and_then(|v| u32::try_from(v).ok()),
        "quality" => {
            format.quality = value.as_f64();
            crate::info_order::mark_int_quality(format);
        }
        "filesize" => format.filesize = value.as_u64(),
        _ => {}
    }
}

/// Upstream `parse_qsl(unescapeHTML(...))` as an ordered dict: blank values
/// are dropped, a repeated key keeps its first position and last value.
fn parse_params(text: &str) -> Vec<(String, String)> {
    let mut params: Vec<(String, String)> = Vec::new();
    for (name, value) in url::form_urlencoded::parse(text.as_bytes()) {
        if value.is_empty() {
            continue;
        }
        match params.iter_mut().find(|(key, _)| *key == name) {
            Some(slot) => slot.1 = value.into_owned(),
            None => params.push((name.into_owned(), value.into_owned())),
        }
    }
    params
}

fn set_param(params: &mut Vec<(String, String)>, key: &str, value: String) {
    match params.iter_mut().find(|(name, _)| name == key) {
        Some(slot) => slot.1 = value,
        None => params.push((key.to_owned(), value)),
    }
}

fn remove_param(params: &mut Vec<(String, String)>, key: &str) {
    params.retain(|(name, _)| name != key);
}

impl Extractor for HuyaLive {
    fn key(&self) -> &'static str {
        LIVE_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !live_suitable(url.as_str()) {
            return Ok(None);
        }
        let video_id = LIVE_URL
            .captures(url.as_str())
            .map(|found| found["id"].to_owned())
            .unwrap_or_default();
        crate::screen::set_extractor(LIVE_NAME);
        crate::screen::screen(&format!(
            "[{LIVE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, LIVE_NAME, &video_id)?;
        let stream_data = super::search_json(&STREAM_START, &webpage, false)
            .flatten()
            .filter(|value| value.is_object())
            .unwrap_or(Value::Null);
        let Some(room_info) = stream_data
            .pointer("/data/0/gameLiveInfo")
            .filter(|room| truthy(Some(*room)))
        else {
            return Err(Error::Reported(format!(
                "[{LIVE_NAME}] {video_id}: Can not extract the room info"
            )));
        };
        let title = room_info
            .get("roomName")
            .filter(|value| truthy(Some(*value)))
            .or_else(|| {
                room_info
                    .get("introduction")
                    .filter(|value| truthy(Some(*value)))
            })
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| crate::html::html_extract_title(&webpage));
        let screen_type = room_info.get("screenType");
        let live_source_type = int_value(room_info.get("liveSourceType"));

        let stream_info_list = stream_data
            .pointer("/data/0")
            .ok_or_else(|| key_error(&video_id, "data"))?
            .get("gameStreamInfoList")
            .ok_or_else(|| key_error(&video_id, "gameStreamInfoList"))?;
        if !truthy(Some(stream_info_list)) {
            return Err(Error::Reported(format!(
                "[{LIVE_NAME}] {video_id}: Video is offline"
            )));
        }
        let mut formats = Vec::new();
        for stream_info in stream_info_list.as_array().into_iter().flatten() {
            let Some(stream_url) = stream_info
                .get("sFlvUrl")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            let stream_name = stream_info
                .get("sStreamName")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let re_secret =
                !truthy(screen_type) && matches!(live_source_type, Some(0) | Some(8) | Some(13));
            if re_secret {
                return Err(unsupported("signed-secret live streams"));
            }
            let anti_code = stream_info
                .get("sFlvAntiCode")
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(&video_id, "sFlvAntiCode"))?;
            let mut params = parse_params(&crate::html::unescape_html(anti_code));
            let suffix = stream_info
                .get("sFlvUrlSuffix")
                .and_then(Value::as_str)
                .ok_or_else(|| unsupported("streams without a URL suffix"))?
                .to_owned();
            let line_index = str_or_none(stream_info.get("iLineIndex"));
            let display_list = stream_data
                .get("vMultiStreamInfo")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            for stream in display_list {
                let display = stream
                    .get("sDisplayName")
                    .and_then(Value::as_str)
                    .ok_or_else(|| unsupported("display names that are not strings"))?;
                let found = DISPLAY_NAME
                    .captures(display)
                    .ok_or_else(|| unsupported("display names"))?;
                let display_name = found.get(1).map_or("", |m| m.as_str()).to_owned();
                let bitrate = found.get(2).map(|m| m.as_str().to_owned());
                let mut rate = int_value(stream.get("iBitRate"));
                if rate.is_some_and(|rate| rate != 0) {
                    set_param(&mut params, "ratio", rate.unwrap_or_default().to_string());
                } else {
                    remove_param(&mut params, "ratio");
                    if let Some(bitrate) = bitrate.and_then(|text| text.parse::<i64>().ok()) {
                        rate = Some(bitrate * 1000);
                    }
                }
                let base = Url::parse(&format!("{stream_url}/{stream_name}.{suffix}"))?;
                let query: Vec<(&str, &str)> = params
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str()))
                    .collect();
                let format_url = super::update_url_query(&base, &query);
                let (width, height) = match display_name.as_str() {
                    "蓝光" => (Some(1920), Some(1080)),
                    "超清" => (Some(1280), Some(720)),
                    "流畅" => (Some(800), Some(480)),
                    _ => (None, None),
                };
                let mut fields = vec![
                    ("ext", json!(suffix)),
                    (
                        "format_id",
                        line_index.clone().map_or(Value::Null, Value::from),
                    ),
                    ("tbr", rate.map_or(Value::Null, Value::from)),
                    ("url", json!(format_url.as_str())),
                ];
                if let (Some(width), Some(height)) = (width, height) {
                    fields.push(("width", json!(width)));
                    fields.push(("height", json!(height)));
                }
                let mut format = dict_format(fields, LIVE_NAME, &video_id)?;
                crate::info_order::mark_int_rates(&mut format);
                formats.push(format);
            }
        }

        Ok(Some(dict_info(
            url,
            (LIVE_NAME, LIVE_KEY),
            vec![
                ("id", video_id.clone().into()),
                ("title", title.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
                (
                    "view_count",
                    int_value(room_info.get("totalCount")).map_or(Value::Null, Value::from),
                ),
                (
                    "thumbnail",
                    room_info.get("screenshot").cloned().unwrap_or(Value::Null),
                ),
                (
                    "description",
                    room_info
                        .get("contentIntro")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "http_headers",
                    json!({"Origin": "https://www.huya.com", "Referer": "https://www.huya.com/"}),
                ),
            ],
            formats,
        )))
    }
}

impl Extractor for HuyaVideo {
    fn key(&self) -> &'static str {
        VIDEO_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(VIDEO_NAME);
        crate::screen::screen(&format!(
            "[{VIDEO_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = super::update_url_query(
            &Url::parse("https://liveapi.huya.com/moment/getMomentContent")?,
            &[("videoId", &video_id)],
        );
        let response = download_json(client, &api, VIDEO_NAME, &video_id)?;
        let moment = response
            .get("data")
            .ok_or_else(|| key_error(&video_id, "data"))?
            .get("moment")
            .ok_or_else(|| key_error(&video_id, "moment"))?;
        let video_info = moment.get("videoInfo");

        let mut formats = Vec::new();
        let definitions = video_info
            .and_then(|info| info.get("definitions"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for definition in definitions {
            let Some(manifest) = definition
                .get("m3u8")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .and_then(|text| Url::parse(&text).ok())
            else {
                continue;
            };
            let Some(parsed) =
                super::extract_m3u8_optional(client, &manifest, VIDEO_NAME, &video_id, None, false)
            else {
                continue;
            };
            for mut format in parsed.formats {
                if let Some(size) = definition.get("size") {
                    update_format(&mut format, "filesize", json!(int_value(Some(size))));
                }
                if let Some(name) = str_or_none(definition.get("defName")) {
                    update_format(&mut format, "format_id", json!(name));
                }
                if let Some(height) = definition.get("height") {
                    update_format(&mut format, "height", json!(int_value(Some(height))));
                }
                if let Some(quality) = definition.get("definition") {
                    update_format(&mut format, "quality", json!(int_value(Some(quality))));
                }
                if let Some(width) = definition.get("width") {
                    update_format(&mut format, "width", json!(int_value(Some(width))));
                }
                formats.push(format);
            }
        }

        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
        let description = moment
            .get("content")
            .and_then(Value::as_str)
            .map(crate::html::clean_html)
            .filter(|text| !text.is_empty());
        let categories = text(video_info.and_then(|info| info.get("category")))
            .filter(|text| !text.is_empty())
            .map(|category| vec![category]);
        let duration = video_info
            .and_then(|info| info.get("videoDuration"))
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
            .map(|duration| match duration {
                crate::info::PyNumber::Int(value) => Value::from(value),
                crate::info::PyNumber::Float(value) => Value::from(value),
            });
        let tags = video_info
            .and_then(|info| info.get("tags"))
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            });
        let thumbnail = ["videoBigCover", "videoCover"].iter().find_map(|key| {
            video_info
                .and_then(|info| info.get(*key))
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .and_then(|text| Url::parse(&text).ok())
                .map(|mut cover| {
                    cover.set_query(None);
                    cover.to_string()
                })
        });
        let uploader_id = str_or_none(video_info.and_then(|info| info.get("uid")));

        let mut fields = vec![("id", json!(video_id)), ("formats", Value::Null)];
        let mut push = |key: &'static str, value: Option<Value>| {
            if let Some(value) = value {
                fields.push((key, value));
            }
        };
        push(
            "comment_count",
            int_value(moment.get("commentCount")).map(Value::from),
        );
        push("description", description.map(Value::from));
        push(
            "like_count",
            int_value(moment.get("favorCount")).map(Value::from),
        );
        push("timestamp", int_value(moment.get("cTime")).map(Value::from));
        push(
            "title",
            text(video_info.and_then(|info| info.get("videoTitle"))).map(Value::from),
        );
        push("categories", categories.map(|list| json!(list)));
        push("duration", duration);
        push("tags", tags.map(|list| json!(list)));
        push("thumbnail", thumbnail.map(Value::from));
        push(
            "uploader",
            text(video_info.and_then(|info| info.get("nickName"))).map(Value::from),
        );
        push("uploader_id", uploader_id.map(Value::from));
        push(
            "view_count",
            int_value(video_info.and_then(|info| info.get("videoPlayNum"))).map(Value::from),
        );

        Ok(Some(dict_info(
            url,
            (VIDEO_NAME, VIDEO_KEY),
            fields,
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(live_suitable("https://www.huya.com/572329"));
        assert!(live_suitable("https://m.huya.com/xiaoyugame"));
        assert!(!live_suitable(
            "https://www.huya.com/video/play/1002412640.html"
        ));
        assert!(video_suitable(
            "https://www.huya.com/video/play/556054543.html"
        ));
        assert!(!video_suitable("https://www.huya.com/572329"));
    }
}
