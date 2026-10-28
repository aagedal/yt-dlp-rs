//! Upstream `ArnesIE` (`yt_dlp/extractor/arnes.py`): Arnes Video
//! (video.arnes.si) videos from the public video API: its media files and
//! the title, thumbnail, description, license, author, channel, duration,
//! views, hashtags and the URL's `t` start time.
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Arnes;

const NAME: &str = "video.arnes.si";
const KEY: &str = "Arnes";
const BASE_URL: &str = "https://video.arnes.si";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://video\.arnes\.si/(?:[a-z]{2}/)?(?:watch|embed|api/(?:asset|public/video))/(?P<id>[0-9a-zA-Z]{12})",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Arnes Video {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `self._BASE_URL + value`: a `None` value is Python's `TypeError`.
fn based(value: Option<&Value>, what: &str) -> Result<String> {
    match value {
        Some(Value::String(path)) => Ok(format!("{BASE_URL}{path}")),
        None | Some(Value::Null) => Err(Error::Reported(
            "can only concatenate str (not \"NoneType\") to str".into(),
        )),
        Some(_) => Err(unsupported(what)),
    }
}

/// `float_or_none(value, 1000)`.
fn milliseconds(value: Option<&Value>) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    number.map_or(Value::Null, |number| Value::from(number / 1000.0))
}

impl Extractor for Arnes {
    fn key(&self) -> &'static str {
        KEY
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
        let api = Url::parse(&format!("{BASE_URL}/api/public/video/{video_id}"))
            .map_err(|_| unsupported("video IDs that do not form URLs"))?;
        let response = super::download_json(client, &api, NAME, &video_id)?;
        let video = match &response {
            Value::Object(response) => response
                .get("data")
                .ok_or_else(|| key_error(&video_id, "data"))?,
            _ => return Err(unsupported("API responses that are not objects")),
        };
        let Value::Object(video) = video else {
            return Err(unsupported("API data that are not objects"));
        };
        let title = video
            .get("title")
            .ok_or_else(|| key_error(&video_id, "title"))?
            .clone();
        let media = match video.get("media") {
            Some(Value::Array(media)) => media.as_slice(),
            Some(value) if super::nuxt::truthy(value) => {
                return Err(unsupported("media lists that are not lists"));
            }
            _ => &[],
        };
        let mut formats = Vec::new();
        for item in media {
            let Value::Object(item) = item else {
                return Err(unsupported("media that are not objects"));
            };
            match item.get("url") {
                Some(value) if super::nuxt::truthy(value) => {}
                _ => continue,
            }
            // `remove_start(media.get('format'), 'FORMAT_')`.
            let format_id = match item.get("format") {
                None | Some(Value::Null) => Value::Null,
                Some(Value::String(format)) => format
                    .strip_prefix("FORMAT_")
                    .unwrap_or(format)
                    .to_owned()
                    .into(),
                Some(_) => return Err(unsupported("media formats that are not strings")),
            };
            formats.push(dict_format(
                vec![
                    (
                        "url",
                        based(item.get("url"), "media URLs that are not strings")?.into(),
                    ),
                    ("format_id", format_id),
                    (
                        "format_note",
                        item.get("formatTranslation")
                            .cloned()
                            .unwrap_or(Value::Null),
                    ),
                    ("width", int_or_none(item.get("width"))),
                    ("height", int_or_none(item.get("height"))),
                ],
                NAME,
                &video_id,
            )?);
        }
        let channel = match video.get("channel") {
            Some(Value::Object(channel)) => Some(channel),
            Some(value) if super::nuxt::truthy(value) => {
                return Err(unsupported("channels that are not objects"));
            }
            _ => None,
        };
        let channel_field = |key: &str| {
            channel
                .and_then(|channel| channel.get(key))
                .cloned()
                .unwrap_or(Value::Null)
        };
        let channel_id = channel_field("url");
        // `format_field(channel_id, None, f'{BASE}/?channel=%s')`.
        let channel_url = match &channel_id {
            Value::Null => String::new(),
            Value::String(id) if id.is_empty() => String::new(),
            Value::String(id) => format!("{BASE_URL}/?channel={id}"),
            Value::Number(id) => format!("{BASE_URL}/?channel={id}"),
            _ => return Err(unsupported("channel URLs that are not strings")),
        };
        let thumbnail = based(
            video.get("thumbnailUrl"),
            "thumbnail URLs that are not strings",
        )?;
        let field = |key: &str| video.get(key).cloned().unwrap_or(Value::Null);
        let timestamp = video
            .get("creationTime")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);
        // `int_or_none(parse_qs(url).get('t', [None])[0])`.
        let start_time = url
            .query_pairs()
            .find(|(key, value)| key == "t" && !value.is_empty())
            .map(|(_, value)| Value::from(value.into_owned()));
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("formats", Value::Null),
                ("thumbnail", thumbnail.into()),
                ("description", field("description")),
                ("license", field("license")),
                ("creator", field("author")),
                ("timestamp", timestamp),
                ("channel", channel_field("name")),
                ("channel_id", channel_id),
                ("channel_url", channel_url.into()),
                ("duration", milliseconds(video.get("duration"))),
                ("view_count", int_or_none(video.get("views"))),
                ("tags", field("hashtags")),
                ("start_time", int_or_none(start_time.as_ref())),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://video.arnes.si/api/asset/s1YjnV7hadlC/play.mp4"
        ));
        assert!(suitable("https://video.arnes.si/en/watch/s1YjnV7hadlC"));
        assert!(!suitable("https://video.arnes.si/watch/short"));
    }
}
