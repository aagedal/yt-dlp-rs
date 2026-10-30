//! Upstream `StreetVoiceIE` (`yt_dlp/extractor/streetvoice.py`): a song's
//! v4 API metadata and its HLS, HTTP and original file URLs (each a
//! non-fatal empty POST) as audio formats.
use super::{Extractor, download_json, update_url_query};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub struct StreetVoice;

const NAME: &str = "StreetVoice";
const FIELDS: &str = "album,comments_count,created_at,id,image,length,likes_count,name,nickname,plays_count,profile,share_count,synopsis,user,username";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:.+?\.)?streetvoice\.com/[^/]+/songs/(?P<id>[0-9]+)")
        .unwrap()
});
static ABR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.mp3\.(\d+)k").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none`: ints, floats truncated and integer strings.
fn int_or_none(value: Option<&Value>) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        Some(Value::Bool(value)) => Some(i64::from(*value)),
        _ => None,
    };
    number.map_or(Value::Null, Value::from)
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Number(number)) => Value::String(number.to_string()),
        Some(Value::Bool(value)) => Value::String(if *value { "True" } else { "False" }.into()),
        _ => Value::Null,
    }
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("StreetVoice {what} are not supported natively"))
}

/// `_download_json(url, song_id, note, data=b'', fatal=False)`: an empty
/// POST whose failures are warnings.
fn post_json_optional(client: &Client, url: &Url, id: &str, note: &str) -> Result<Option<Value>> {
    crate::screen::screen(&format!("[{NAME}] {id}: {note}"));
    let mut response = match super::direct::post(client, url, &[], String::new()) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, response.headers());
    if text.trim().is_empty() {
        crate::screen::warning(&format!(
            "[{NAME}] {id}: Failed to parse JSON: Expecting value in '': line 1 column 1 (char 0)"
        ));
        return Ok(None);
    }
    crate::json::from_str(&text)
        .map(Some)
        .map_err(|_| unsupported("format URL responses that are not JSON"))
}

impl Extractor for StreetVoice {
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
        let base_url = format!("https://streetvoice.com/api/v4/song/{id}/");
        let api = update_url_query(&Url::parse(&base_url)?, &[("fields", FIELDS)]);
        let song = download_json(client, &api, NAME, &id)?;
        let Value::Object(song) = song else {
            return Err(unsupported("songs that are not JSON objects"));
        };
        // `song['name']`.
        let title = match song.get("name") {
            None => {
                return Err(Error::Reported(format!(
                    "{id}: An extractor error has occurred. (caused by KeyError('name')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            Some(Value::String(title)) => title.clone(),
            Some(_) => return Err(unsupported("song names that are not strings")),
        };
        let mut formats = Vec::new();
        for (suffix, format_id) in [
            ("hls/file", "hls"),
            ("file", "http"),
            ("file/original", "original"),
        ] {
            let target = Url::parse(&format!("{base_url}{suffix}/"))?;
            let data = post_json_optional(
                client,
                &target,
                &id,
                &format!("Downloading {format_id} format URL"),
            )?;
            // `(... or {}).get('file')`.
            let f_url = match data {
                None | Some(Value::Null) => None,
                Some(Value::Object(data)) => data.get("file").cloned(),
                Some(Value::Array(items)) if items.is_empty() => None,
                Some(Value::String(text)) if text.is_empty() => None,
                Some(_) => return Err(unsupported("format URL responses that are not objects")),
            };
            let f_url = match f_url {
                None | Some(Value::Null) => continue,
                Some(Value::String(text)) if text.is_empty() => continue,
                Some(Value::String(text)) => text,
                Some(_) => return Err(unsupported("format URLs that are not strings")),
            };
            let mut map = Map::new();
            map.insert("ext".into(), "mp3".into());
            map.insert("format_id".into(), format_id.into());
            map.insert("url".into(), f_url.clone().into());
            map.insert("vcodec".into(), "none".into());
            if format_id == "hls" {
                map.insert("protocol".into(), "m3u8_native".into());
            }
            if let Some(abr) = ABR
                .captures(&f_url)
                .and_then(|found| found[1].parse::<i64>().ok())
            {
                map.insert("abr".into(), abr.into());
                map.insert("tbr".into(), abr.into());
            }
            let keys: Vec<String> = map.keys().cloned().collect();
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
        // `song.get('user') or {}`.
        let empty = Map::new();
        let user = match song.get("user") {
            Some(Value::Object(user)) => user,
            None | Some(Value::Null | Value::Bool(false)) => &empty,
            Some(Value::String(text)) if text.is_empty() => &empty,
            Some(Value::Array(items)) if items.is_empty() => &empty,
            Some(_) => return Err(unsupported("song users that are not objects")),
        };
        let username = user.get("username").filter(|username| match username {
            Value::Null | Value::Bool(false) => false,
            Value::String(text) => !text.is_empty(),
            Value::Number(number) => number.as_f64() != Some(0.0),
            _ => true,
        });
        let uploader_url = match username {
            None => Value::Null,
            Some(Value::String(username)) => url
                .join(&format!("/{username}/"))
                .map_or(Value::Null, |joined| Value::String(joined.into())),
            Some(_) => return Err(unsupported("usernames that are not strings")),
        };
        let thumbnail = match song.get("image") {
            None | Some(Value::Null) => None,
            Some(Value::String(image)) => Some(image.clone()),
            Some(_) => return Err(unsupported("song images that are not strings")),
        };
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id.clone();
        info.title = title.clone();
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        // `strip_or_none(song.get('synopsis'))`.
        info.metadata.description = song
            .get("synopsis")
            .and_then(Value::as_str)
            .map(|synopsis| synopsis.trim().to_owned());
        info.metadata.thumbnail = thumbnail;
        let timestamp = song
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);
        let count = |name: &str| int_or_none(song.get(&format!("{name}_count")));
        let duration = int_or_none(song.get("length"));
        // Processing drops a `None` duration.
        let has_duration = !duration.is_null();
        if has_duration {
            crate::metadata_parser::set_field(&mut info, "duration", duration);
        }
        let uploader = user
            .get("profile")
            .and_then(Value::as_object)
            .and_then(|profile| profile.get("nickname"))
            .cloned()
            .unwrap_or(Value::Null);
        let album = song
            .get("album")
            .and_then(Value::as_object)
            .and_then(|album| album.get("name"))
            .cloned()
            .unwrap_or(Value::Null);
        for (name, value) in [
            ("timestamp", timestamp),
            ("uploader", uploader),
            ("uploader_id", str_or_none(user.get("id"))),
            ("uploader_url", uploader_url),
            ("view_count", count("plays")),
            ("like_count", count("likes")),
            ("comment_count", count("comments")),
            ("repost_count", count("share")),
            ("track", Value::String(title)),
            ("track_id", Value::String(id)),
            ("album", album),
        ] {
            if value.is_null() {
                info.extra.insert(name.into(), Value::Null);
            } else {
                crate::metadata_parser::set_field(&mut info, name, value);
            }
        }
        let shape = format!(
            "dict;id,formats,title,description,thumbnail,{}timestamp,uploader,uploader_id,uploader_url,view_count,like_count,comment_count,repost_count,track,track_id,album",
            if has_duration { "duration," } else { "" }
        );
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://streetvoice.com/skippylu/songs/123688/"));
        assert!(suitable("http://tw.streetvoice.com/skippylu/songs/94440/"));
        assert!(!suitable("https://streetvoice.com/skippylu/"));
    }
}
