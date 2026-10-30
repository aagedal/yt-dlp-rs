//! Upstream `GoProIE` (`yt_dlp/extractor/gopro.py`): GoPro shared videos from
//! the page's `__reflectData` and the media download API's variations, with
//! the collection's title, creation time and account, and the music track.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct GoPro;

const NAME: &str = "GoPro";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(www\.)?gopro\.com/v/(?P<id>[A-Za-z0-9]+)").unwrap()
});
static METADATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__reflectData\s*=\s*").unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<title[^>]*>([^<]+)</title>").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `str_or_none(value)`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.into()),
        other => Some(other.to_string()),
    }
}

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

/// `_html_search_meta(names, webpage)`: the first name present, else a
/// warning naming the first.
fn search_meta(webpage: &str, names: &[&str]) -> Option<String> {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name));
    if found.is_none() {
        super::search_warning(NAME, names[0]);
    }
    found
}

impl Extractor for GoPro {
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
        let metadata = super::search_json(&METADATA, &webpage, false)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract metadata; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "GoPro metadata that do not parse are not supported natively".into(),
                )
            })?;
        let video_info = match metadata.get("collectionMedia") {
            // An uncaught `IndexError` is reported as its message.
            Some(Value::Array(media)) => media
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "GoPro media lists that are not lists are not supported natively".into(),
                ));
            }
            None => return Err(key_error(&video_id, "collectionMedia")),
        };
        let media_id = str_or_none(Some(
            video_info
                .get("id")
                .ok_or_else(|| key_error(&video_id, "id"))?,
        ))
        .unwrap_or_else(|| "None".into());
        let api = Url::parse(&format!("https://api.gopro.com/media/{media_id}/download"))?;
        let media_data = download_json(client, &api, NAME, &video_id)?;
        let mut formats = Vec::new();
        if let Some(Value::Array(variations)) = media_data.pointer("/_embedded/variations") {
            for variation in variations {
                let Some(format_url) = variation
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                else {
                    continue;
                };
                let text =
                    |key: &str| str_or_none(variation.get(key)).map_or(Value::Null, Value::from);
                formats.push(dict_format(
                    vec![
                        ("url", format_url.into()),
                        ("format_id", text("quality")),
                        ("format_note", text("label")),
                        ("ext", text("type")),
                        ("width", int_or_none(variation.get("width"))),
                        ("height", int_or_none(variation.get("height"))),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        let collection_title = metadata
            .pointer("/collection/title")
            .filter(|title| super::nuxt::truthy(title))
            .cloned();
        let title = match collection_title {
            Some(title) => str_or_none(Some(&title)),
            None => search_meta(&webpage, &["og:title", "twitter:title"])
                .filter(|title| !title.is_empty())
                .or_else(|| {
                    let found = TITLE
                        .captures(&webpage)
                        .map(|found| crate::html::clean_html(&found[1]));
                    if found.is_none() {
                        super::search_warning(NAME, "title");
                    }
                    found.map(|title| {
                        title
                            .strip_suffix(" | GoPro")
                            .map_or(title.clone(), str::to_owned)
                    })
                }),
        }
        .map(|title| title.replace('\n', " "));
        let thumbnail = search_meta(&webpage, &["og:image", "twitter:image"])
            .and_then(|image| super::json_ld::url_or_none(&image));
        let timestamp = metadata
            .pointer("/collection/created_at")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, false))
            .map_or(Value::Null, Value::from);
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let nonempty = |key: &str| {
            str_or_none(video_info.get(key))
                .filter(|value| !value.is_empty())
                .map_or(Value::Null, Value::from)
        };
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", optional(title)),
                ("formats", Value::Null),
                ("thumbnail", optional(thumbnail)),
                ("timestamp", timestamp),
                (
                    "uploader_id",
                    optional(str_or_none(metadata.pointer("/account/nickname"))),
                ),
                ("duration", int_or_none(video_info.get("source_duration"))),
                ("artist", nonempty("music_track_artist")),
                ("track", nonempty("music_track_name")),
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
        assert!(suitable("https://gopro.com/v/ZNVvED8QDzR5V"));
        assert!(!suitable("https://gopro.com/shop"));
    }
}
