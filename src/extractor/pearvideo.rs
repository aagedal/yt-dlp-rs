//! Upstream `PearVideoIE` (`yt_dlp/extractor/pearvideo.py`): the page's
//! `KINDUrl = "..."` assignments as ranked formats, else the `videoStatus.jsp`
//! API's videos, with the page's title, summary and date.
use super::{Extractor, download_json_headers, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct PearVideo;

const NAME: &str = "PearVideo";
const QUALITIES: [&str; 7] = ["ldflv", "ld", "sdflv", "sd", "hdflv", "hd", "src"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?pearvideo\.com/video_(?P<id>\d+)").unwrap()
});
// `(?P<id>[a-zA-Z]+)Url\s*=\s*(["\'])(?P<url>(?:https?:)?//.+?)\2`.
static FORMAT_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?P<id>[a-zA-Z]+)Url\s*=\s*(?:"(?P<dq>(?:https?:)?//.+?)"|'(?P<sq>(?:https?:)?//.+?)')"#,
    )
    .unwrap()
});
static TITLES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<h1[^>]+\bclass=(?:"video-tt"|'video-tt')[^>]*>(?P<value>[^<]+)"#).unwrap(),
        Regex::new(r#"<[^>]+\bdata-title=(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')"#).unwrap(),
    ]
});
static DESCRIPTIONS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<div[^>]+\bclass=(?:"summary"|'summary')[^>]*>(?P<value>[^<]+)"#).unwrap(),
        Regex::new(r#"<[^>]+\bdata-summary=(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')"#).unwrap(),
    ]
});
static DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div[^>]+\bclass=["']date["'][^>]*>([^<]+)"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Pear Video {what} are not supported natively"))
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// `_search_regex((...), group='value')`: the first pattern that matches.
fn search_value(patterns: &[Regex], webpage: &str) -> Option<String> {
    patterns.iter().find_map(|pattern| {
        pattern.captures(webpage).and_then(|found| {
            ["value", "dq", "sq"]
                .iter()
                .find_map(|name| found.name(name))
                .map(|value| value.as_str().to_owned())
        })
    })
}

fn format(map: Map<String, Value>, keys: &[&str], video_id: &str) -> Result<crate::VideoFormat> {
    let mut map = map;
    let media = map
        .get("url")
        .and_then(Value::as_str)
        .map(super::proto_relative_http)
        .unwrap_or_default();
    map.insert("url".into(), media.clone().into());
    let ext = super::direct::determine_ext(&media).unwrap_or_else(|| "unknown_video".into());
    map.insert("ext".into(), ext.into());
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
        .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    Ok(format)
}

impl Extractor for PearVideo {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let mut formats = Vec::new();
        for found in FORMAT_URL.captures_iter(&webpage) {
            let format_id = &found["id"];
            let media = found.name("dq").or_else(|| found.name("sq")).unwrap();
            // `qualities(...)`: the list index, else -1.
            let quality = QUALITIES
                .iter()
                .position(|known| *known == format_id)
                .map_or(-1, |index| index as i64);
            let mut map = Map::new();
            map.insert("url".into(), media.as_str().into());
            map.insert("format_id".into(), format_id.into());
            map.insert("quality".into(), json!(quality));
            let mut format = format(map, &["url", "format_id", "quality"], &video_id)?;
            crate::info_order::mark_int_quality(&mut format);
            formats.push(format);
        }
        if formats.is_empty() {
            let api = super::update_url_query(
                &Url::parse("https://www.pearvideo.com/videoStatus.jsp")?,
                &[("contId", &video_id)],
            );
            let info =
                download_json_headers(client, &api, NAME, &video_id, &[("Referer", url.as_str())])?;
            if let Some(Value::Object(videos)) =
                info.get("videoInfo").and_then(|info| info.get("videos"))
            {
                for (key, value) in videos {
                    if !truthy(value) {
                        continue;
                    }
                    let Value::String(media) = value else {
                        return Err(unsupported("API video URLs that are not strings"));
                    };
                    let media = if key == "srcUrl" {
                        let system_time = match info.get("systemTime") {
                            Some(Value::String(time)) => time.clone(),
                            Some(_) => {
                                return Err(unsupported("API system times that are not strings"));
                            }
                            None => {
                                return Err(Error::Reported(format!(
                                    "{video_id}: An extractor error has occurred. (caused by KeyError('systemTime')); {}",
                                    crate::BUG_REPORTS_MESSAGE
                                )));
                            }
                        };
                        media.replace(&system_time, &format!("cont-{video_id}"))
                    } else {
                        media.clone()
                    };
                    let mut map = Map::new();
                    map.insert("format_id".into(), key.as_str().into());
                    map.insert("url".into(), media.into());
                    formats.push(format(map, &["format_id", "url"], &video_id)?);
                }
            }
        }
        let title = search_value(&*TITLES, &webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract title; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let description = search_value(&*DESCRIPTIONS, &webpage)
            .filter(|description| !description.is_empty())
            .or_else(|| {
                let meta = crate::html::html_search_meta(&webpage, "Description");
                if meta.is_none() {
                    search_warning(NAME, "Description");
                }
                meta
            });
        let date = DATE.captures(&webpage).map(|found| found[1].to_owned());
        if date.is_none() {
            search_warning(NAME, "timestamp");
        }
        let timestamp = date.and_then(|date| crate::dates::unified_timestamp(&date, true));
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = video_id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        crate::metadata_parser::set_field(
            &mut info,
            "description",
            description.map_or(Value::Null, Value::from),
        );
        crate::metadata_parser::set_field(
            &mut info,
            "timestamp",
            timestamp.map_or(Value::Null, Value::from),
        );
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,description,timestamp,formats",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.pearvideo.com/video_1076290"));
        assert!(suitable("https://pearvideo.com/video_1"));
        assert!(!suitable("https://www.pearvideo.com/category_1"));
    }
}
