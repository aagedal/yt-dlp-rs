//! Upstream `VeoIE` (`yt_dlp/extractor/veo.py`): a match's API metadata and
//! its video list as formats (transport streams skipped, panoramic videos
//! checked first).
use super::{Extractor, download_json, download_json_note};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Veo;

const NAME: &str = "Veo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://app\.veo\.co/matches/(?P<id>[0-9A-Za-z\-_]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none(value, scale=scale)`: ints, floats truncated and integer
/// strings, floor-divided by `scale`.
fn int_or_none(value: Option<&Value>, scale: i64) -> Option<i64> {
    let number = match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))?,
        Value::String(text) => text.trim().parse::<i64>().ok()?,
        Value::Bool(value) => i64::from(*value),
        _ => return None,
    };
    Some(number.div_euclid(scale))
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.into()),
        _ => None,
    }
}

/// `url_or_none`.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    super::json_ld::url_or_none(text)
}

fn crash(id: &str, what: &str) -> Error {
    Error::UnsupportedHtml5(format!(
        "[{NAME}] {id}: Veo {what} that are not JSON objects are not supported natively"
    ))
}

impl Extractor for Veo {
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
        let api = Url::parse(&format!("https://app.veo.co/api/app/matches/{id}"))?;
        let metadata = download_json(client, &api, NAME, &id)?;
        let videos = Url::parse(&format!("https://app.veo.co/api/app/matches/{id}/videos"))?;
        let video_data = download_json_note(client, &videos, NAME, &id, "Downloading video data")?;
        let Value::Object(metadata) = metadata else {
            return Err(crash(&id, "metadata"));
        };
        let Value::Array(video_data) = video_data else {
            return Err(crash(&id, "video lists"));
        };
        let mut formats = Vec::new();
        for fmt in &video_data {
            let Value::Object(fmt) = fmt else {
                return Err(crash(&id, "videos"));
            };
            let mimetype = str_or_none(fmt.get("mime_type"));
            let format_url = url_or_none(fmt.get("url"));
            // Skip the panoramic video's configuration file (`video/mp2t`).
            let Some(format_url) = format_url else {
                continue;
            };
            if mimetype.as_deref() == Some("video/mp2t") {
                continue;
            }
            let height = int_or_none(fmt.get("height"), 1);
            let render_type = str_or_none(fmt.get("render_type"));
            let format_id = match (&render_type, height) {
                (Some(render_type), Some(height)) if !render_type.is_empty() && height != 0 => {
                    Some(format!("{render_type}-{height}p"))
                }
                _ => None,
            };
            // Panoramic video information is listed even when the video is
            // unavailable: `_is_valid_url(format_url, video_id, format_id)`.
            if render_type.as_deref() == Some("panorama") {
                let item = format_id.as_deref().unwrap_or("None");
                let checked = super::proto_relative_http(&format_url);
                if checked.starts_with("http://") || checked.starts_with("https://") {
                    crate::screen::screen(&format!("[{NAME}] {id}: Checking {item} URL"));
                    let target = Url::parse(&checked)?;
                    if let Err(error) = super::direct::request(client, &target, None) {
                        crate::screen::screen(&format!(
                            "[{NAME}] {id}: {item} URL is invalid, skipping: {}",
                            crate::upstream_error_text(&error)
                        ));
                        continue;
                    }
                }
            }
            let ext = mimetype
                .as_deref()
                .map(|mimetype| super::direct::mimetype2ext_or(mimetype, None));
            let mut map = Map::new();
            map.insert("url".into(), format_url.clone().into());
            map.insert("format_id".into(), format_id.clone().into());
            map.insert("ext".into(), ext.clone().into());
            map.insert("width".into(), int_or_none(fmt.get("width"), 1).into());
            map.insert("height".into(), height.into());
            map.insert("vbr".into(), int_or_none(fmt.get("bit_rate"), 1000).into());
            let keys: Vec<String> = map.keys().cloned().collect();
            map.retain(|_, value| !value.is_null());
            let ext = ext.unwrap_or_else(|| {
                super::direct::determine_ext(&format_url).unwrap_or_else(|| "unknown_video".into())
            });
            map.insert("ext".into(), ext.into());
            map.entry("format_id").or_insert_with(|| "".into());
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id;
        let title = str_or_none(metadata.get("title"));
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.metadata.thumbnail = url_or_none(metadata.get("thumbnail"));
        let timestamp = metadata
            .get("created")
            .and_then(Value::as_str)
            .and_then(|created| crate::dates::unified_timestamp(created, false));
        for (name, value) in [
            ("timestamp", timestamp),
            ("view_count", int_or_none(metadata.get("view_count"), 1)),
            ("duration", int_or_none(metadata.get("duration"), 1)),
        ] {
            crate::metadata_parser::set_field(
                &mut info,
                name,
                value.map_or(Value::Null, Value::from),
            );
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,formats,thumbnail,timestamp,view_count,duration",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://app.veo.co/matches/20201027-last-period/"));
        assert!(suitable(
            "https://app.veo.co/matches/20220313-2022-03-13_u15m-plsjq-vs-csl/"
        ));
        assert!(!suitable("https://app.veo.co/teams/x"));
    }

    #[test]
    fn bit_rates_are_floor_divided() {
        assert_eq!(int_or_none(Some(&Value::from(4_500_500)), 1000), Some(4500));
        assert_eq!(int_or_none(Some(&Value::from("360")), 1), Some(360));
    }
}
