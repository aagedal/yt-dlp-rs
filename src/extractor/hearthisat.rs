//! Upstream `HearThisAtIE` (`yt_dlp/extractor/hearthisat.py`): the
//! `api-v2.hearthis.at` track JSON, its stream MP3 and the original download
//! (when its file name has a known extension) as formats.
use super::{Extractor, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct HearThisAt;

const NAME: &str = "HearThisAt";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?hearthis\.at/(?P<artist>[^/?#]+)/(?P<title>[\w.-]+)").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("HearThisAt {what} are not supported natively"))
}

/// `'{}'.format(value)` for a string or `None`.
fn py_str(value: Option<&Value>) -> Result<String> {
    match value {
        None | Some(Value::Null) => Ok("None".into()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(unsupported("titles and user names that are not strings")),
    }
}

/// `str_to_int`: an integer, or a string with `,.+` removed.
fn str_to_int(value: Option<&Value>) -> Result<Value> {
    match value {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::Number(number)) if number.is_i64() || number.is_u64() => {
            Ok(Value::Number(number.clone()))
        }
        Some(Value::String(text)) => Ok(text
            .replace([',', '.', '+'], "")
            .parse::<i64>()
            .map_or(Value::Null, Value::from)),
        Some(_) => Err(unsupported("counts and durations that are not integers")),
    }
}

/// Python truthiness.
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

fn non_null(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| !value.is_null()).cloned()
}

fn format(map: Map<String, Value>, id: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = map.keys().cloned().collect();
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
        .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_quality(&mut format);
    Ok(format)
}

impl Extractor for HearThisAt {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = format!("{} - {}", &found["artist"], &found["title"]);
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api_url = url
            .as_str()
            .replace("www.", "")
            .replace("hearthis.at", "api-v2.hearthis.at");
        let data = download_json(client, &Url::parse(&api_url)?, NAME, &display_id)?;
        let data = data
            .as_object()
            .ok_or_else(|| unsupported("track responses that are not objects"))?;
        let track_id = match data.get("id") {
            Some(Value::String(id)) => id.clone(),
            _ => return Err(unsupported("tracks without a string ID")),
        };
        let user = match data.get("user") {
            Some(Value::Object(user)) => user,
            _ => return Err(unsupported("tracks without a user")),
        };
        let title = format!(
            "{} - {}",
            py_str(user.get("username"))?,
            py_str(data.get("title"))?
        );
        let thumbnail = data
            .get("artwork_url")
            .filter(|value| truthy(value))
            .or_else(|| data.get("thumb"))
            .filter(|value| !value.is_null())
            .cloned();
        let view_count = str_to_int(data.get("playback_count"))?;
        let duration = str_to_int(data.get("duration"))?;
        let mut formats = Vec::new();
        if let Some(mp3_url) = data.get("stream_url").filter(|value| truthy(value)) {
            let mut map = Map::new();
            map.insert("format_id".into(), "mp3".into());
            map.insert("vcodec".into(), "none".into());
            map.insert("acodec".into(), "mp3".into());
            map.insert("url".into(), mp3_url.clone());
            map.insert("ext".into(), "mp3".into());
            formats.push(format(map, &track_id)?);
        }
        if let Some(download_url) = data.get("download_url").filter(|value| truthy(value)) {
            let filename = data
                .get("download_filename")
                .and_then(Value::as_str)
                .ok_or_else(|| unsupported("downloads without a file name"))?;
            let ext =
                super::direct::determine_ext(filename).unwrap_or_else(|| "unknown_video".into());
            if super::archiveorg::KNOWN_EXTENSIONS.contains(&ext.as_str()) {
                let mut map = Map::new();
                map.insert("format_id".into(), ext.clone().into());
                map.insert("vcodec".into(), "none".into());
                map.insert("ext".into(), ext.clone().into());
                map.insert("url".into(), download_url.clone());
                map.insert("acodec".into(), ext.into());
                map.insert("quality".into(), json!(2));
                formats.push(format(map, &track_id)?);
            }
        }
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = track_id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        if let Some(Value::String(thumbnail)) = &thumbnail {
            info.metadata.thumbnail = Some(thumbnail.clone());
        } else if thumbnail.is_some() {
            return Err(unsupported("thumbnails that are not strings"));
        }
        crate::metadata_parser::set_field(&mut info, "display_id", display_id.into());
        let fields = [
            ("description", non_null(data.get("description"))),
            ("duration", Some(duration)),
            ("timestamp", non_null(data.get("release_timestamp"))),
            ("view_count", Some(view_count)),
            ("genre", non_null(data.get("genre"))),
        ];
        for (key, value) in fields {
            crate::metadata_parser::set_field(&mut info, key, value.unwrap_or(Value::Null));
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,display_id,title,formats,thumbnail,description,duration,timestamp,view_count,genre",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://hearthis.at/moofi/dr-kreep/"));
        assert!(suitable(
            "https://www.hearthis.at/twitchsf/dj-jim-hopkins-totally-bitchin-80s-dance-mix/"
        ));
        assert!(!suitable("https://hearthis.at/moofi"));
    }
}
