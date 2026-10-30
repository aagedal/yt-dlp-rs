//! Upstream `GettrIE` (`yt_dlp/extractor/gettr.py`): a GETTR post's API
//! record (`api.gettr.com/u/post/ID`), its HLS video (`vid`) and original
//! MP4 (`ovid`) on media.gettr.com, with the author from the user info, and
//! the shared/embedded post it links to when the post has no video.
//! GETTR streams (`GettrStreamingIE`) are reported as unsupported.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Gettr;
pub struct GettrStreaming;

const NAME: &str = "Gettr";
const KEY: &str = "Gettr";
const STREAMING_NAME: &str = "GettrStreaming";
const MEDIA_BASE_URL: &str = "https://media.gettr.com/";
const API_BASE_URL: &str = "https://api.gettr.com/u/";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(www\.)?gettr\.com/post/(?P<id>[a-z0-9]+)").unwrap());

static STREAMING_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(www\.)?gettr\.com/streaming/(?P<id>[a-z0-9]+)").unwrap()
});

static UPLOADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.+?) on GETTR").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_valid_url` of `GettrStreamingIE`.
pub fn streaming_suitable(url: &str) -> bool {
    STREAMING_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("GETTR {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Value {
    match value {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(text)) => Value::from(text.as_str()),
        Some(other) => Value::from(other.to_string()),
    }
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(items)) => !items.is_empty(),
    }
}

/// `int_or_none` of a JSON number or numeric string.
fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

/// `float_or_none(value, scale=...)`.
fn float_or_none(value: Option<&Value>, scale: f64) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    number.map_or(Value::Null, |number| Value::from(number / scale))
}

/// `urljoin(base, path)`: `None` for an empty or missing path.
fn media_url(path: Option<&Value>) -> Option<Url> {
    let path = path?.as_str().filter(|path| !path.is_empty())?;
    let base = Url::parse(MEDIA_BASE_URL).ok()?;
    base.join(path).ok()
}

/// `url_or_none` of a string.
fn url_or_none(value: Option<&str>) -> Option<String> {
    let text = value?;
    Url::parse(text)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.to_string())
}

/// `self._call_api(path, video_id)`: the response's `result`.
fn call_api(client: &Client, path: &str, video_id: &str) -> Result<Value> {
    let url = Url::parse(&format!("{API_BASE_URL}{path}"))?;
    let response = download_json(client, &url, NAME, video_id)?;
    response
        .get("result")
        .cloned()
        .ok_or_else(|| key_error(video_id, "result"))
}

/// A post without a usable media object, which upstream reports as an
/// `ExtractorError` (with the bug report note).
fn no_video(post_id: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {post_id}: There's no video in this post.; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for Gettr {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<crate::ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let post_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let webpage = download_webpage(client, url, NAME, &post_id)?;
        // `incl="poststats|userinfo"`, percent-encoded as urllib sends it.
        let api_data = call_api(
            client,
            &format!("post/{post_id}?incl=%22poststats%7Cuserinfo%22"),
            &post_id,
        )?;

        let Some(post_data) = api_data.get("data").and_then(Value::as_object) else {
            return Err(unsupported("posts without a data object"));
        };
        let user_data: Map<String, Value> = match (
            api_data.get("aux"),
            post_data.get("uid").and_then(Value::as_str),
        ) {
            (Some(aux), Some(uid)) => aux
                .get("uinf")
                .and_then(|uinf| uinf.get(uid))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            _ => Map::new(),
        };

        let vid = post_data.get("vid");
        let ovid = post_data.get("ovid");

        if post_data.get("p_type").and_then(Value::as_str) == Some("stream") {
            return Err(unsupported("streams"));
        }

        if !truthy(ovid) && !truthy(vid) {
            let embed_url = url_or_none(post_data.get("prevsrc").and_then(Value::as_str));
            let shared_post_id = api_data
                .get("aux")
                .and_then(|aux| aux.get("shrdpst"))
                .and_then(|shared| shared.get("_id"))
                .and_then(Value::as_str)
                .or_else(|| {
                    post_data
                        .get("rpstIds")
                        .and_then(Value::as_array)
                        .and_then(|ids| ids.first())
                        .and_then(Value::as_str)
                })
                .map(str::to_owned);
            if let Some(embed_url) = embed_url {
                return Ok(Some(super::follow_url(url, embed_url)));
            } else if let Some(shared_post_id) = shared_post_id {
                let target = format!("https://gettr.com/post/{shared_post_id}");
                return Ok(Some(super::follow_url(url, target)));
            }
            return Err(no_video(&post_id));
        }

        let og_description = crate::html::og_search_property(&webpage, "description");
        let description_text = match post_data.get("txt") {
            Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
            _ => og_description,
        };
        let og_title = crate::html::og_search_property(&webpage, "title").unwrap_or_default();

        let nickname = user_data.get("nickname");
        let uploader: Option<String> = if truthy(nickname) {
            nickname.and_then(Value::as_str).map(str::to_owned)
        } else {
            let found = UPLOADER
                .captures(&og_title)
                .map(|captures| captures[1].to_owned());
            if found.is_none() {
                super::search_warning(NAME, "uploader");
            }
            found
        };
        let title = match (&uploader, &description_text) {
            (Some(uploader), title) => Some(format!(
                "{uploader} - {}",
                title.as_deref().unwrap_or("None")
            )),
            (None, title) => title.clone(),
        };

        let mut subtitles = crate::Subtitles::default();
        let mut formats = Vec::new();
        if truthy(vid) {
            let manifest =
                media_url(vid).ok_or_else(|| unsupported("media paths that are not URLs"))?;
            // `_extract_m3u8_formats_and_subtitles` reads only the master playlist.
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &post_id,
                Some("hls"),
                true,
                false,
            ) {
                formats.extend(stream.formats);
                subtitles = stream.subtitles;
            }
        }
        if truthy(ovid) {
            let ovid_url =
                media_url(ovid).ok_or_else(|| unsupported("media paths that are not URLs"))?;
            formats.push(dict_format(
                vec![
                    ("url", ovid_url.as_str().into()),
                    ("format_id", "ovid".into()),
                    ("ext", "mp4".into()),
                    ("width", int_or_none(post_data.get("vid_wid"))),
                    ("height", int_or_none(post_data.get("vid_hgt"))),
                ],
                NAME,
                &post_id,
            )?);
        }

        let uploader_id = match ["_id", "username"]
            .iter()
            .find_map(|key| user_data.get(*key).filter(|value| truthy(Some(*value))))
        {
            Some(value) => str_or_none(Some(value)),
            None => str_or_none(post_data.get("uid")),
        };

        let thumbnail = match media_url(post_data.get("main")) {
            Some(main) => url_or_none(Some(main.as_str())).map_or(Value::Null, Value::from),
            None => {
                let found = ["og:image", "image"]
                    .iter()
                    .find_map(|prop| crate::html::html_search_meta(&webpage, prop));
                if found.is_none() {
                    super::search_warning(NAME, "thumbnail");
                }
                url_or_none(found.as_deref()).map_or(Value::Null, Value::from)
            }
        };

        let timestamp_source = ["cdate", "udate"]
            .iter()
            .find_map(|key| post_data.get(*key).filter(|value| truthy(Some(*value))));
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", post_id.clone().into()),
                ("title", title.map_or(Value::Null, Value::from)),
                (
                    "description",
                    description_text.map_or(Value::Null, Value::from),
                ),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                (
                    "uploader",
                    uploader.as_deref().map_or(Value::Null, Value::from),
                ),
                ("uploader_id", uploader_id),
                ("thumbnail", thumbnail),
                ("timestamp", float_or_none(timestamp_source, 1000.0)),
                ("duration", float_or_none(post_data.get("vid_dur"), 1.0)),
                (
                    "tags",
                    post_data.get("htgs").cloned().unwrap_or(Value::Null),
                ),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(crate::ExtractResult::Video(info)))
    }
}

impl Extractor for GettrStreaming {
    fn key(&self) -> &'static str {
        STREAMING_NAME
    }

    fn extract(&self, _client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !STREAMING_URL.is_match(url.as_str()) {
            return Ok(None);
        }
        crate::screen::set_extractor(STREAMING_NAME);
        Err(unsupported("streams"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.gettr.com/post/pcf6uv838f"));
        assert!(suitable("https://gettr.com/post/p4iahp"));
        assert!(!suitable("https://gettr.com/streaming/psoiulc122"));
        assert!(streaming_suitable("https://gettr.com/streaming/psoiulc122"));
    }

    #[test]
    fn numeric_fields_follow_upstream_helpers() {
        assert_eq!(int_or_none(Some(&Value::from("854"))), Value::from(854));
        assert_eq!(
            float_or_none(Some(&Value::from("58.5585")), 1.0),
            Value::from(58.5585)
        );
        assert_eq!(
            float_or_none(Some(&Value::from(1632782458942_i64)), 1000.0),
            Value::from(1632782458.942)
        );
    }
}
