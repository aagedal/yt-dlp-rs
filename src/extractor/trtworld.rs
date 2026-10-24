//! Upstream `TrtWorldIE` (`yt_dlp/extractor/trtworld.py`): the page's
//! `window.__NUXT__` data (`_search_nuxt_data`), the website/OTT platform
//! HLS or HTTP media URLs and metadata, else the YouTube platform's ID.
use super::nuxt::{Step, search_nuxt_data};
use super::{Extractor, download_webpage, extract_m3u8_master_optional};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct TrtWorld;

const NAME: &str = "TrtWorld";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://www\.trtworld\.com/video/[\w-]+/[\w-]+-(?P<id>\d+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("TRT World {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `value[key]` on a dict (a missing key is upstream's `KeyError`).
fn index<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    match value {
        Value::Object(map) => map.get(key).ok_or_else(|| key_error(id, key)),
        _ => Err(unsupported("Nuxt data that are not objects")),
    }
}

/// `traverse_obj(value, keys)` over dict keys (`None` ends the path).
fn path<'a>(mut value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    for key in keys {
        value = value.as_object()?.get(*key)?;
    }
    (!value.is_null()).then_some(value)
}

impl Extractor for TrtWorld {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let nuxt = search_nuxt_data(
            &webpage,
            NAME,
            Some(&display_id),
            "__NUXT__",
            true,
            &[Step::Key("data"), Step::Index(0)],
        )?;
        let platforms = index(
            index(
                index(&nuxt, "videoData", &display_id)?,
                "content",
                &display_id,
            )?,
            "platforms",
            &display_id,
        )?;
        let branches: Vec<&Value> = ["website", "ott"]
            .iter()
            .filter_map(|key| path(platforms, &[key]))
            .collect();
        let mut formats = Vec::new();
        for branch in &branches {
            for key in ["hls_url", "url"] {
                let Some(media_url) = path(branch, &["metadata", key])
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                else {
                    continue;
                };
                // NB: Website sometimes serves mp4 files under `hls_url` key.
                if super::direct::determine_ext(&media_url).as_deref() == Some("m3u8") {
                    let manifest = Url::parse(&media_url)?;
                    if let Some(mut stream) = extract_m3u8_master_optional(
                        client,
                        &manifest,
                        NAME,
                        &display_id,
                        None,
                        false,
                        false,
                    ) {
                        super::hls::without_ext(&mut stream);
                        formats.append(&mut stream.formats);
                    }
                } else {
                    let mut map = Map::new();
                    map.insert("format_id".into(), "http".into());
                    map.insert("url".into(), media_url.clone().into());
                    let ext = super::direct::determine_ext(&media_url)
                        .unwrap_or_else(|| "unknown_video".into());
                    map.insert("ext".into(), ext.into());
                    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                        .map_err(|error| {
                            Error::Reported(format!("[{NAME}] {display_id}: {error}"))
                        })?;
                    crate::info_order::mark_format(&mut format, "dict:format_id,url");
                    formats.push(format);
                }
            }
        }
        if formats.is_empty() {
            if let Some(youtube_id) = path(platforms, &["youtube", "metadata", "youtubeId"]) {
                let Value::String(youtube_id) = youtube_id else {
                    return Err(unsupported("YouTube IDs that are not strings"));
                };
                // `url_result(youtube_id, 'Youtube')`.
                let mut follow = super::direct::build_at(url.clone(), url, String::new());
                follow.formats.clear();
                follow.direct = None;
                follow
                    .extra
                    .insert(crate::FOLLOW_URL.into(), youtube_id.clone().into());
                return Ok(Some(follow));
            }
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: No video found"
            )));
        }
        // `traverse_obj(nuxtjs_data, (('website', 'ott'), {...}),
        // get_all=False)`: the first branch with any field.
        let mut fields: Vec<(&str, Value)> = Vec::new();
        for branch in &branches {
            let candidates = [
                (
                    "title",
                    path(branch, &["fields", "title", "text"])
                        .filter(|value| value.is_string())
                        .cloned(),
                ),
                (
                    "description",
                    path(branch, &["fields", "description", "text"])
                        .filter(|value| value.is_string())
                        .cloned(),
                ),
                (
                    "thumbnail",
                    path(branch, &["fields", "thumbnail", "url"])
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                        .map(Value::from),
                ),
                (
                    "release_timestamp",
                    path(branch, &["published", "date"])
                        .and_then(Value::as_str)
                        .and_then(crate::dates::parse_iso8601)
                        .map(Value::from),
                ),
            ];
            fields = candidates
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key, value)))
                .collect();
            if !fields.is_empty() {
                break;
            }
        }
        if fields.is_empty() {
            // `{**None}` is upstream's `TypeError`.
            return Err(unsupported("videos without website or OTT metadata"));
        }
        let first = formats[0].url.clone();
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = display_id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let mut keys = vec!["id", "formats"];
        match fields
            .iter()
            .find(|(key, _)| *key == "title")
            .and_then(|(_, title)| title.as_str())
        {
            Some(title) => info.title = title.to_owned(),
            None => {
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        for (key, value) in &fields {
            keys.push(key);
            match *key {
                "title" => {}
                "thumbnail" => info.metadata.thumbnail = value.as_str().map(str::to_owned),
                "description" => info.metadata.description = value.as_str().map(str::to_owned),
                _ => crate::metadata_parser::set_field(&mut info, key, value.clone()),
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
            "https://www.trtworld.com/video/news/turkiye-switches-to-sustainable-tourism-16067690"
        ));
        assert!(suitable(
            "https://www.trtworld.com/video/one-offs/frames-from-anatolia-14541780"
        ));
        assert!(!suitable("https://www.trtworld.com/video/news-16067690"));
        assert!(!suitable("https://trtworld.com/video/news/x-1"));
    }
}
