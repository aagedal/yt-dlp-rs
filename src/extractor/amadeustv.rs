//! Upstream `AmadeusTVIE` (`yt_dlp/extractor/amadeustv.py`): the library
//! page's `window.__NUXT__` fetch data (`_search_nuxt_data`), its Tencent
//! Cloud VOD play info as HTTP formats, and the item's metadata.
use super::nuxt::{Step, search_nuxt_data};
use super::{Extractor, download_json_headers, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct AmadeusTv;

const NAME: &str = "AmadeusTV";
const REFERER: &str = "http://www.amadeus.tv/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?amadeus\.tv/library/(?P<id>[\da-f]+)")
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `traverse_obj(value, path)` over plain keys and indices (a `None`
/// value ends the path).
fn path<'a>(mut value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    for key in keys {
        value = match value {
            Value::Object(map) => map.get(*key)?,
            Value::Array(items) => items.get(key.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    (!value.is_null()).then_some(value)
}

/// `float_or_none`.
fn float_or_none(value: Option<&Value>) -> Option<Value> {
    let number = match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    serde_json::Number::from_f64(number).map(Value::Number)
}

/// `int_or_none` (`None` dropped).
fn int_or_none(value: Option<&Value>) -> Option<Value> {
    Some(super::streamable::int_or_none(value)).filter(|value| !value.is_null())
}

/// Python's `str(x)` for `f'http-{x or "0"}'`.
fn definition_id(value: &Value) -> String {
    let text = match value {
        Value::Null | Value::Bool(false) => "0".into(),
        Value::Bool(true) => "True".into(),
        Value::Number(number) if number.as_f64() == Some(0.0) => "0".into(),
        Value::Number(number) => number.to_string(),
        Value::String(text) if text.is_empty() => "0".into(),
        Value::String(text) => text.clone(),
        Value::Array(items) if items.is_empty() => "0".into(),
        Value::Object(map) if map.is_empty() => "0".into(),
        other => other.to_string(),
    };
    format!("http-{text}")
}

impl Extractor for AmadeusTv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let nuxt = search_nuxt_data(
            &webpage,
            NAME,
            Some(&display_id),
            "__NUXT__",
            true,
            &[Step::Key("fetch"), Step::Key("0")],
        )?;
        let video_id = path(&nuxt, &["item", "video"])
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract actual video ID; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let api = Url::parse(&format!(
            "http://playvideo.qcloud.com/getplayinfo/v2/1253584441/{video_id}"
        ))?;
        let video_data =
            download_json_headers(client, &api, NAME, &video_id, &[("Referer", REFERER)])?;
        // `('videoInfo', ('sourceVideo', ('transcodeList', ...)), {dict})`.
        let mut sources: Vec<&Map<String, Value>> = Vec::new();
        if let Some(info) = video_data.get("videoInfo") {
            if let Some(source) = info.get("sourceVideo").and_then(Value::as_object) {
                sources.push(source);
            }
            match info.get("transcodeList") {
                Some(Value::Array(items)) => {
                    sources.extend(items.iter().filter_map(Value::as_object));
                }
                Some(Value::Object(map)) => {
                    sources.extend(map.values().filter_map(Value::as_object));
                }
                _ => {}
            }
        }
        let mut formats = Vec::new();
        for video in sources {
            let Some(f_url) = video
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let video = Value::Object(video.clone());
            let mut map = Map::new();
            map.insert("url".into(), video["url"].clone());
            // Upstream's lambda also sees a missing `definition` (`None`).
            let definition = video.get("definition").unwrap_or(&Value::Null);
            map.insert("format_id".into(), definition_id(definition).into());
            for (key, value) in [
                ("width", int_or_none(video.get("width"))),
                ("height", int_or_none(video.get("height"))),
                (
                    "filesize",
                    int_or_none(video.get("totalSize")).or_else(|| int_or_none(video.get("size"))),
                ),
                (
                    "vcodec",
                    path(&video, &["videoStreamList", "0", "codec"]).cloned(),
                ),
                (
                    "acodec",
                    path(&video, &["audioStreamList", "0", "codec"]).cloned(),
                ),
                (
                    "fps",
                    float_or_none(path(&video, &["videoStreamList", "0", "fps"])),
                ),
            ] {
                if let Some(value) = value {
                    map.insert(key.into(), value);
                }
            }
            for key in ["vcodec", "acodec"] {
                if map.get(key).is_some_and(|codec| !codec.is_string()) {
                    return Err(Error::UnsupportedHtml5(
                        "AmadeusTV codecs that are not strings are not supported natively".into(),
                    ));
                }
            }
            map.insert("http_headers".into(), json!({"Referer": REFERER}));
            let keys: Vec<String> = map.keys().cloned().collect();
            let ext =
                super::direct::determine_ext(&f_url).unwrap_or_else(|| "unknown_video".into());
            map.insert("ext".into(), ext.into());
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            formats.push(format);
        }
        let mut keys = vec!["id", "display_id", "formats"];
        let mut fields: Vec<(&str, Value)> = Vec::new();
        let mut push = |key: &'static str, value: Option<Value>| {
            if let Some(value) = value {
                match fields.iter_mut().find(|(name, _)| *name == key) {
                    Some(slot) => slot.1 = value,
                    None => fields.push((key, value)),
                }
            }
        };
        push(
            "title",
            path(&video_data, &["videoInfo", "basicInfo", "name"])
                .filter(|title| title.is_string())
                .cloned(),
        );
        push(
            "thumbnail",
            path(&video_data, &["coverInfo", "coverUrl"])
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(Value::from),
        );
        push(
            "duration",
            ["floatDuration", "duration"].iter().find_map(|key| {
                float_or_none(path(&video_data, &["videoInfo", "sourceVideo", key]))
            }),
        );
        if let Some(item) = nuxt.get("item") {
            let string_of = |names: &[&str]| {
                names
                    .iter()
                    .find_map(|name| item.get(*name).filter(|value| value.is_string()))
                    .cloned()
            };
            push("title", string_of(&["title", "title_en", "title_cn"]));
            push(
                "description",
                string_of(&["description", "description_en", "description_cn"]),
            );
            push(
                "timestamp",
                item.get("date")
                    .and_then(Value::as_str)
                    .and_then(crate::dates::parse_iso8601)
                    .map(Value::from),
            );
            push("view_count", int_or_none(item.get("view")));
        }
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = video_id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        crate::metadata_parser::set_field(&mut info, "display_id", display_id.into());
        match fields
            .iter()
            .find(|(key, _)| *key == "title")
            .and_then(|(_, title)| title.as_str())
        {
            Some(title) => info.title = title.to_owned(),
            None => {
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        for (key, value) in &fields {
            keys.push(key);
            match *key {
                "title" => {}
                "thumbnail" => info.metadata.thumbnail = value.as_str().map(str::to_owned),
                "description" => info.metadata.description = value.as_str().map(str::to_owned),
                _ => crate::metadata_parser::set_field(&mut info, key, value.clone()),
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("dict;{}", keys.join(",")),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.amadeus.tv/library/65091a87ff85af59d9fc54c3"
        ));
        assert!(suitable("https://amadeus.tv/library/abc123"));
        assert!(!suitable("https://www.amadeus.tv/library/xyz"));
    }

    #[test]
    fn definitions_format_like_python() {
        assert_eq!(definition_id(&json!(0)), "http-0");
        assert_eq!(definition_id(&json!(210)), "http-210");
        assert_eq!(definition_id(&json!("")), "http-0");
        assert_eq!(definition_id(&Value::Null), "http-0");
    }
}
