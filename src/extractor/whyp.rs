//! Upstream `WhypIE` (`yt_dlp/extractor/whyp.py`): the page's Nuxt 3
//! `__NUXT_DATA__` payload (`_search_nuxt_json`), its Pinia track and user
//! data, and the track's audio/lossy/lossless URLs as audio formats.
use super::nuxt::{Mode, search_nuxt_json};
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Whyp;

const NAME: &str = "Whyp";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?whyp\.it/tracks/(?:(?P<id>\d{5,})/)?(?P<display_id>[^/?#]+)")
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Whyp {what} are not supported natively"))
}

/// `traverse_obj(data, (KEY, 'data', ..., {dict}, any))`.
fn first_dict<'a>(data: &'a Map<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
    let items: Box<dyn Iterator<Item = &Value>> = match data.get(key)?.get("data")? {
        Value::Array(items) => Box::new(items.iter()),
        Value::Object(map) => Box::new(map.values()),
        _ => return None,
    };
    items.filter_map(Value::as_object).next()
}

/// `{int}, {str_or_none}`: an integer (not a bool) as a string.
fn int_string(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_number)
        .filter(|number| number.is_i64() || number.is_u64())
        .map(|number| Value::String(number.to_string()))
}

/// `float_or_none`.
fn float_or_none(value: Option<&Value>) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    number
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

impl Extractor for Whyp {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let unique_id = found.name("id").map(|id| id.as_str().to_owned());
        let display_id = found["display_id"].to_owned();
        let video_id = unique_id.clone().unwrap_or_else(|| display_id.clone());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let payload =
            search_nuxt_json(&webpage, NAME, &video_id, unique_id.as_deref(), Mode::Fatal)?
                .unwrap_or(Value::Null);
        // `require(NAME)`: the temporary ID is the URL's (optional) `id`.
        let require = |name: &str| {
            let prefix = unique_id
                .as_deref()
                .map_or(String::new(), |id| format!("{id}: "));
            Error::Reported(format!(
                "[{NAME}] {prefix}Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let data = payload
            .get("pinia")
            .and_then(Value::as_object)
            .ok_or_else(|| require("pinia data"))?;
        let track = first_dict(data, "track").ok_or_else(|| require("track data"))?;
        let mut formats = Vec::new();
        for prefix in ["audio", "lossy", "lossless"] {
            let Some(f_url) = track
                .get(&format!("{prefix}_url"))
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let filesize = super::streamable::int_or_none(track.get(&format!("{prefix}_size")));
            let mut map = Map::new();
            let f_url_text = f_url.clone();
            map.insert("url".into(), f_url.into());
            map.insert("format_id".into(), prefix.into());
            map.insert("filesize".into(), filesize);
            map.insert("vcodec".into(), "none".into());
            map.insert(
                "quality".into(),
                json!(if prefix == "lossless" { 10 } else { -1 }),
            );
            map.insert(
                "http_headers".into(),
                json!({"Referer": "https://whyp.it/"}),
            );
            let keys: Vec<String> = map.keys().cloned().collect();
            // Processing's `determine_ext(url)`; not one of upstream's keys.
            let ext =
                super::direct::determine_ext(&f_url_text).unwrap_or_else(|| "unknown_video".into());
            map.insert("ext".into(), ext.into());
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            crate::info_order::mark_int_quality(&mut format);
            formats.push(format);
        }
        let id = int_string(track.get("id"))
            .and_then(|id| id.as_str().map(str::to_owned))
            .or_else(|| unique_id.clone())
            .ok_or_else(|| unsupported("tracks without an ID"))?;
        let mut keys = vec!["id", "formats"];
        let mut fields: Vec<(&str, Value)> = Vec::new();
        if let Some(title) = track.get("title").filter(|title| title.is_string()) {
            fields.push(("title", title.clone()));
        }
        if let Some(slug) = track.get("slug").filter(|slug| slug.is_string()) {
            fields.push(("display_id", slug.clone()));
        }
        if let Some(description) = track.get("description").filter(|value| !value.is_null()) {
            fields.push(("description", description.clone()));
        }
        let duration = float_or_none(track.get("duration"));
        if !duration.is_null() {
            fields.push(("duration", duration));
        }
        if let Some(timestamp) = track
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.push(("timestamp", timestamp.into()));
        }
        let thumbnail = track
            .get("artwork_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        if let Some(user) = first_dict(data, "user") {
            if let Some(username) = user.get("username").filter(|name| name.is_string()) {
                fields.push(("uploader", username.clone()));
            }
            if let Some(uploader_id) = int_string(user.get("id")) {
                fields.push(("uploader_id", uploader_id));
            }
        }
        let title = fields
            .iter()
            .find(|(key, _)| *key == "title")
            .and_then(|(_, title)| title.as_str().map(str::to_owned));
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        match title {
            Some(title) => info.title = title,
            None => {
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        if let Some(thumbnail) = thumbnail {
            info.metadata.thumbnail = Some(thumbnail);
            fields.push(("thumbnail", Value::Null));
        }
        // Upstream's key order: the track fields, then the user's.
        let order = [
            "title",
            "display_id",
            "description",
            "duration",
            "timestamp",
            "thumbnail",
            "uploader",
            "uploader_id",
        ];
        for key in order {
            if let Some((key, value)) = fields.iter().find(|(name, _)| *name == key) {
                keys.push(key);
                if *key != "thumbnail" && *key != "title" {
                    crate::metadata_parser::set_field(&mut info, key, value.clone());
                }
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
            "https://whyp.it/tracks/example-track-HPuZgly3yViP"
        ));
        assert!(suitable(
            "https://www.whyp.it/tracks/18337/home-page-example-track-b4kq7"
        ));
        assert!(suitable("https://www.whyp.it/tracks/18337"));
        assert!(!suitable("https://whyp.it/users/1"));
    }
}
