//! Upstream `XimalayaIE` (`yt_dlp/extractor/ximalaya.py`): a sound's
//! `m.ximalaya.com/tracks/ID.json` metadata, its cover thumbnails and its two
//! public audio streams. VIP-restricted sounds (the upstream `mpay` flow,
//! which decrypts the `videa` RC4 payload) and albums are reported as
//! unsupported.
use super::{Extractor, dict_format, dict_info, download_json_note};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Ximalaya;
pub struct XimalayaAlbum;

const NAME: &str = "ximalaya";
const KEY: &str = "Ximalaya";
const ALBUM_NAME: &str = "ximalaya:album";
const ALBUM_KEY: &str = "XimalayaAlbum";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.|m\.)?ximalaya\.com/(?:(?P<uid>\d+)/)?sound/(?P<id>[0-9]+)",
    )
    .unwrap()
});

static ALBUM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.|m\.)?ximalaya\.com/(?:\d+/)?album/(?P<id>[0-9]+)").unwrap()
});

/// `_match_valid_url` of `XimalayaIE`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_valid_url` of `XimalayaAlbumIE`.
pub fn album_suitable(url: &str) -> bool {
    ALBUM_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Ximalaya {what} are not supported natively"))
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

impl Extractor for Ximalaya {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let audio_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let scheme = if url.scheme() == "https" {
            "https"
        } else {
            "http"
        };
        let info_url = Url::parse(&format!("{scheme}://m.ximalaya.com/tracks/{audio_id}.json"))?;
        let audio_info =
            download_json_note(client, &info_url, NAME, &audio_id, "Downloading info json")
                .map_err(|error| match error {
                    // `errnote='Unable to download info file'`.
                    Error::ExtractorRequest {
                        extractor,
                        id,
                        source,
                        ..
                    } => Error::ExtractorRequest {
                        extractor,
                        id,
                        what: "info file",
                        source,
                    },
                    other => other,
                })?;
        let Value::Object(audio_info) = audio_info else {
            return Err(unsupported("track metadata that is not an object"));
        };

        if truthy(audio_info.get("is_paid")) {
            return Err(unsupported("VIP-restricted sounds"));
        }

        let mut formats = Vec::new();
        for (bps, key) in [(24, "play_path_32"), (64, "play_path_64")] {
            let Some(Value::String(stream)) = audio_info.get(key) else {
                continue;
            };
            if stream.is_empty() {
                continue;
            }
            let mut format = dict_format(
                vec![
                    ("format_id", format!("{bps}k").into()),
                    ("url", stream.as_str().into()),
                    ("abr", bps.into()),
                    ("vcodec", "none".into()),
                ],
                NAME,
                &audio_id,
            )?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }

        let mut thumbnails = Vec::new();
        for (key, value) in &audio_info {
            // Cover keys look like `cover_url` and `cover_url_142`.
            if !key.starts_with("cover_url") {
                continue;
            }
            let mut thumbnail = Map::new();
            thumbnail.insert("id".into(), key.as_str().into());
            thumbnail.insert("url".into(), value.clone());
            if key == "cover_url_142" {
                thumbnail.insert("width".into(), 180.into());
                thumbnail.insert("height".into(), 180.into());
            }
            thumbnails.push(Value::Object(thumbnail));
        }

        let uploader_id = audio_info.get("uid");
        let uploader_url = if truthy(uploader_id) {
            Value::from(format!(
                "{scheme}://www.ximalaya.com/zhubo/{}/",
                uploader_id.map(json_text).unwrap_or_default()
            ))
        } else {
            Value::Null
        };
        // `try_call(lambda: intro.replace(...).replace(...))`.
        let description = match audio_info.get("intro") {
            Some(Value::String(intro)) => {
                Value::from(intro.replace("\r\n\r\n\r\n ", "\n").replace("\r\n", "\n"))
            }
            _ => Value::Null,
        };
        let categories = match audio_info.get("category_name") {
            Some(category) if truthy(Some(category)) => json!([category]),
            _ => json!([]),
        };

        let title = match audio_info.get("title") {
            Some(title) => title.clone(),
            None => return Err(key_error(&audio_id, "title")),
        };

        let info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", audio_id.into()),
                (
                    "uploader",
                    audio_info.get("nickname").cloned().unwrap_or(Value::Null),
                ),
                ("uploader_id", str_or_none(uploader_id)),
                ("uploader_url", uploader_url),
                ("title", title),
                ("thumbnails", Value::Array(thumbnails)),
                ("description", description),
                ("categories", categories),
                (
                    "duration",
                    audio_info.get("duration").cloned().unwrap_or(Value::Null),
                ),
                (
                    "view_count",
                    audio_info.get("play_count").cloned().unwrap_or(Value::Null),
                ),
                (
                    "like_count",
                    audio_info
                        .get("favorites_count")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                ("formats", Value::Null),
            ],
            formats,
        );
        // `{id, url, width, height}` cover dicts keep upstream's key order.
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,uploader,uploader_id,uploader_url,title,thumbnails,description,categories,duration,view_count,like_count,formats;thumbnail=id,url,width,height",
        )))
    }
}

/// `str(value)` of a JSON number or string.
fn json_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

impl Extractor for XimalayaAlbum {
    fn key(&self) -> &'static str {
        ALBUM_KEY
    }

    fn extract(&self, _client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !ALBUM_URL.is_match(url.as_str()) {
            return Ok(None);
        }
        crate::screen::set_extractor(ALBUM_NAME);
        Err(unsupported("albums"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.ximalaya.com/sound/47740352/"));
        assert!(suitable("http://www.ximalaya.com/61425525/sound/47740352/"));
        assert!(suitable("https://www.ximalaya.com/sound/562111701"));
        assert!(!suitable("https://www.ximalaya.com/album/6912905"));
        assert!(album_suitable("https://www.ximalaya.com/album/6912905"));
        assert!(album_suitable(
            "http://www.ximalaya.com/61425525/album/5534601/"
        ));
    }
}
