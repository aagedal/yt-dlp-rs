//! Upstream `EpidemicSoundIE` (`yt_dlp/extractor/epidemicsound.py`): a track's
//! JSON (or a sound effect's kosmos-ID JSON), its cover thumbnails and the
//! stems' MP3 files.
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct EpidemicSound;

const NAME: &str = "EpidemicSound";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?epidemicsound\.com/(?:(?P<sfx>sound-effects/tracks)|track)/(?P<id>[0-9a-zA-Z-]+)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `str_or_none` of a JSON value (numbers are converted, `None` is absent).
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// `_epidemic_parse_thumbnail`: the URL with its `width`/`height` query
/// parameters, or the resolution parsed from the URL.
fn thumbnail(url: &str) -> Option<Value> {
    let url = super::json_ld::url_or_none(url)?;
    let parsed = Url::parse(&url).ok();
    let query_int = |name: &str| {
        parsed.as_ref().and_then(|parsed| {
            parsed
                .query_pairs()
                .find(|(key, _)| key == name)
                .and_then(|(_, value)| value.parse::<i64>().ok())
        })
    };
    let (width, height) = (query_int("width"), query_int("height"));
    let mut thumb = json!({"url": url});
    if width.is_some() || height.is_some() {
        if let Some(width) = width {
            thumb["width"] = width.into();
        }
        if let Some(height) = height {
            thumb["height"] = height.into();
        }
    } else {
        let (width, height) = super::html5::parse_resolution(&url);
        if let Some(width) = width {
            thumb["width"] = width.into();
        }
        if let Some(height) = height {
            thumb["height"] = height.into();
        }
    }
    Some(thumb)
}

/// `_epidemic_fmt_or_none` over one `stems` item: `(key, stem)`.
fn stem_format(key: &str, stem: &Value, video_id: &str) -> Result<Option<VideoFormat>> {
    let falsy = |value: &Option<String>| value.as_deref().is_none_or(str::is_empty);
    let mut format = Some(key.to_owned());
    let mut format_id = stem
        .get("stemType")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let note = str_or_none(stem.get("s3TrackId"));
    let url = stem
        .get("lqMp3Url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none);
    if falsy(&format) {
        format = format_id.clone();
    } else if falsy(&format_id) {
        format_id = format.clone();
    }
    // `if not f['url'] or not f['format']: return None`.
    let (Some(url), false) = (url, falsy(&format)) else {
        return Ok(None);
    };
    let format_note = note
        .filter(|note| !note.is_empty())
        .map(|note| format!("track ID {note}"));
    let mut fields = vec![("format", Value::from(format.clone()))];
    if let Some(note) = format_note {
        fields.push(("format_note", note.into()));
    }
    fields.push(("format_id", Value::from(format_id)));
    fields.push(("url", url.into()));
    if format.as_deref() != Some("full") {
        fields.push(("preference", (-2).into()));
    }
    Ok(Some(dict_format(fields, NAME, video_id)?))
}

impl Extractor for EpidemicSound {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let is_sfx = found.name("sfx").is_some();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = if is_sfx {
            format!("https://www.epidemicsound.com/json/track/kosmos-id/{video_id}")
        } else {
            format!("https://www.epidemicsound.com/json/track/{video_id}")
        };
        let json_data = download_json(client, &Url::parse(&api)?, NAME, &video_id)?;

        // `traverse_obj(json_data, [('imageUrl', 'cover')])`, then the cover
        // art's sizes under its base URL, de-duplicated in order.
        let mut thumbnail_urls: Vec<String> = ["imageUrl", "cover"]
            .iter()
            .filter_map(|key| {
                json_data
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        if let (Some(base), Some(Value::Object(sizes))) = (
            json_data
                .pointer("/coverArt/baseUrl")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none),
            json_data.pointer("/coverArt/sizes"),
        ) {
            for size in sizes.values().filter_map(Value::as_str) {
                thumbnail_urls.push(format!("{base}{size}"));
            }
        }
        let mut seen = std::collections::HashSet::new();
        let thumbnails: Vec<Value> = thumbnail_urls
            .into_iter()
            .filter(|url| seen.insert(url.clone()))
            .filter_map(|url| thumbnail(&url))
            .collect();

        let mut formats = Vec::new();
        if let Some(Value::Object(stems)) = json_data.get("stems") {
            for (key, stem) in stems {
                if let Some(format) = stem_format(key, stem, &video_id)? {
                    formats.push(format);
                }
            }
        }

        let mut fields: Vec<(&str, Value)> = Vec::new();
        let mut push = |key: &'static str, value: Option<Value>| {
            if let Some(value) = value {
                fields.push((key, value));
            }
        };
        push("id", str_or_none(json_data.get("id")).map(Value::from));
        push(
            "display_id",
            json_data
                .get("publicSlug")
                .and_then(Value::as_str)
                .map(Value::from),
        );
        push(
            "title",
            json_data
                .get("title")
                .and_then(Value::as_str)
                .map(Value::from),
        );
        push(
            "alt_title",
            json_data
                .get("oldTitle")
                .and_then(Value::as_str)
                .map(Value::from),
        );
        push(
            "duration",
            json_data
                .get("length")
                .and_then(|length| length.as_f64().or_else(|| length.as_str()?.parse().ok()))
                .map(Value::from),
        );
        push(
            "timestamp",
            json_data
                .get("added")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map(Value::from),
        );
        push(
            "release_timestamp",
            json_data
                .get("releaseDate")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map(Value::from),
        );
        push(
            "categories",
            string_list(json_data.get("genres"), Some("tag")),
        );
        push("tags", string_list(json_data.get("metadataTags"), None));
        push(
            "age_limit",
            json_data
                .get("isExplicit")
                .and_then(Value::as_bool)
                .filter(|explicit| *explicit)
                .map(|_| Value::from(18)),
        );
        push(
            "thumbnails",
            (!thumbnails.is_empty()).then(|| Value::Array(thumbnails)),
        );
        fields.push(("formats", Value::Null));
        Ok(Some(with_thumbnail_keys(
            dict_info(url, (NAME, NAME), fields, formats),
            "url,width,height",
        )))
    }
}

/// The info's `dict;KEYS` shape with the thumbnails' key order (upstream's
/// dict literal order, `_sanitize_thumbnails` adding the rest).
fn with_thumbnail_keys(info: VideoInfo, keys: &str) -> VideoInfo {
    let shape = info
        .extra
        .get(crate::info_order::INFO_SHAPE)
        .and_then(Value::as_str)
        .unwrap_or("dict")
        .to_owned();
    crate::info_order::mark_info(info, &format!("{shape};thumbnail={keys}"))
}

/// `(..., KEY, {str})` over a list (`None` when nothing matched).
fn string_list(list: Option<&Value>, key: Option<&str>) -> Option<Value> {
    let items: Vec<Value> = list?
        .as_array()?
        .iter()
        .filter_map(|item| match key {
            Some(key) => item.get(key)?.as_str(),
            None => item.as_str(),
        })
        .map(Value::from)
        .collect();
    (!items.is_empty()).then(|| Value::Array(items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.epidemicsound.com/track/yFfQVRpSPz/"));
        assert!(suitable("https://www.epidemicsound.com/track/mj8GTTwsZd/"));
        assert!(suitable(
            "https://www.epidemicsound.com/sound-effects/tracks/2f02f54b-9faa-4daf-abac-1cfe9e9cef69/"
        ));
        assert!(!suitable("https://www.epidemicsound.com/music/"));
    }

    #[test]
    fn thumbnail_takes_query_size_or_resolution() {
        let sized = thumbnail(
            "https://cdn.epidemicsound.com/curation-assets/commercial-release-cover-images/11138/3000x3000.jpg?height=300&width=300",
        )
        .unwrap();
        assert_eq!(
            (sized["width"].as_i64(), sized["height"].as_i64()),
            (Some(300), Some(300))
        );
        let plain = thumbnail(
            "https://cdn.epidemicsound.com/curation-assets/commercial-release-cover-images/11138/3000x3000.jpg",
        )
        .unwrap();
        assert_eq!(plain["width"].as_i64(), Some(3000));
        assert!(thumbnail("not a url").is_none());
    }
}
