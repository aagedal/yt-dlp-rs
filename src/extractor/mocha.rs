//! Upstream `MochaVideoIE` (`yt_dlp/extractor/mocha.py`): the Mocha video
//! API's detail, its resolution HLS playlists and original MP4 as formats.
use super::{Extractor, download_json, extract_m3u8_fatal, update_url_query};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct MochaVideo;

const NAME: &str = "MochaVideo";
const API: &str = "http://apivideo.mocha.com.vn:8081/onMediaBackendBiz/mochavideo/getVideoDetail";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://video\.mocha\.com\.vn/(?P<video_slug>[\w-]+)").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Upstream's wrapped `KeyError`: without an `id` group, `get_temp_id`
/// gives no ID prefix.
fn key_error(key: &str) -> Error {
    Error::Reported(format!(
        "An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("{what} are not supported natively"))
}

/// `int_or_none(value, scale=1000)`: `int(value) // 1000`.
fn timestamp(value: Option<&Value>) -> Option<i64> {
    let number = match value? {
        Value::Bool(flag) => i64::from(*flag),
        value => super::streamable::int_or_none(Some(value)).as_i64()?,
    };
    Some(number.div_euclid(1000))
}

/// `traverse_obj(json_data, ('channels', '0', KEY))`: the string key `'0'`
/// never indexes a list, so only an object's `"0"` entry is read.
fn channel_field(data: &Value, key: &str) -> Option<Value> {
    let first = data.get("channels")?.as_object()?.get("0")?;
    first.get(key).filter(|value| !value.is_null()).cloned()
}

impl Extractor for MochaVideo {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_slug = found["video_slug"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = update_url_query(&Url::parse(API)?, &[("url", url.as_str()), ("token", "")]);
        let response = download_json(client, &api, NAME, &video_slug)?;
        let data = response.get("data").ok_or_else(|| key_error("data"))?;
        let detail = data
            .get("videoDetail")
            .ok_or_else(|| key_error("videoDetail"))?;
        if !detail.is_object() {
            return Err(unsupported("non-object Mocha video details"));
        }
        let id = match detail.get("id") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Number(number)) if number.is_i64() || number.is_u64() => number.to_string(),
            Some(_) => return Err(unsupported("non-integer Mocha video IDs")),
            None => return Err(key_error("id")),
        };
        let mut videos: Vec<Value> = match detail.get("list_resolution") {
            Some(Value::Array(items)) => items.clone(),
            None | Some(Value::Null) => Vec::new(),
            Some(_) => return Err(unsupported("non-list Mocha resolution lists")),
        };
        videos.push(detail.get("original_path").cloned().unwrap_or(Value::Null));
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        for video in videos {
            match video {
                Value::String(media) => {
                    let mut format: VideoFormat = serde_json::from_value(
                        json!({"url": media, "ext": "mp4", "format_id": ""}),
                    )
                    .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
                    crate::info_order::mark_format(&mut format, "dict:url,ext");
                    formats.push(format);
                }
                Value::Object(map) => {
                    let Some(Value::String(path)) = map.get("video_path") else {
                        return Err(unsupported("Mocha resolutions without a string video_path"));
                    };
                    let manifest = Url::parse(path)?;
                    let stream = extract_m3u8_fatal(client, &manifest, NAME, &id, None, true)?;
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                }
                Value::Null => {
                    // `None.get(...)`: upstream's bare (unwrapped) error.
                    return Err(Error::Reported(
                        "'NoneType' object has no attribute 'get'".into(),
                    ));
                }
                _ => return Err(unsupported("non-object Mocha resolutions")),
            }
        }
        let mut info = super::direct::build_at(url.clone(), url, String::new());
        info.formats = formats;
        info.subtitles = subtitles;
        info.direct = None;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.id = id;
        let display_id = match detail.get("slug") {
            Some(Value::String(slug)) if !slug.is_empty() => slug.clone(),
            Some(Value::Null | Value::Bool(false)) | None => video_slug,
            Some(Value::String(_)) => video_slug,
            Some(_) => return Err(unsupported("non-string Mocha slugs")),
        };
        info.extra.insert("display_id".into(), display_id.into());
        match detail.get("name") {
            Some(Value::String(title)) => info.title = title.clone(),
            None | Some(Value::Null) => {
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
            Some(_) => return Err(unsupported("non-string Mocha names")),
        }
        let has_duration = detail
            .get("durationS")
            .is_some_and(|value| !value.is_null());
        let mut set = |key: &str, value: Option<Value>| {
            if let Some(value) = value.filter(|value| !value.is_null()) {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        };
        let get = |key: &str| detail.get(key).cloned();
        set("description", get("description"));
        set("duration", get("durationS"));
        set("view_count", get("total_view"));
        set("like_count", get("total_like"));
        set("dislike_count", get("total_unlike"));
        set("thumbnail", get("image_path_thumb"));
        set(
            "timestamp",
            timestamp(detail.get("publish_time")).map(Value::from),
        );
        set("is_live", get("isLive"));
        set("channel", channel_field(detail, "name"));
        set("channel_id", channel_field(detail, "id"));
        set("channel_follower_count", channel_field(detail, "numfollow"));
        let categories: Vec<Value> = match detail.get("categories") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.get("categoryname"))
                .filter(|value| !value.is_null())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        set("categories", Some(Value::Array(categories)));
        set("comment_count", get("total_comment"));
        // Processing drops a `None` duration.
        let duration = if has_duration { "duration," } else { "" };
        let shape = format!(
            "dict;id,display_id,title,formats,subtitles,description,{duration}view_count,like_count,dislike_count,thumbnail,timestamp,is_live,channel,channel_id,channel_follower_count,categories,comment_count"
        );
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://video.mocha.com.vn/chuyen-meo-gia-su-tu-thong-diep-cuoc-song-v18694039"
        ));
        assert!(!suitable("http://mocha.com.vn/x"));
    }

    #[test]
    fn timestamps_are_scaled_like_int_or_none() {
        assert_eq!(timestamp(Some(&json!(1652254203123_i64))), Some(1652254203));
        assert_eq!(timestamp(Some(&json!("1652254203000"))), Some(1652254203));
        assert_eq!(timestamp(Some(&json!("x"))), None);
        assert_eq!(timestamp(None), None);
    }
}
