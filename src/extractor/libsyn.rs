//! Upstream `LibsynIE` (`yt_dlp/extractor/libsyn.py`): the HTML5 player
//! page's `playlistItem` JSON, its episode and podcast titles, description,
//! release date and the libsyn/main/download media URLs as formats.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Libsyn;

const NAME: &str = "Libsyn";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<mainurl>https?://html5-player\.libsyn\.com/embed/episode/id/(?P<id>[0-9]+))")
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Libsyn {what} are not supported natively"))
}

fn regex_error(id: &str, name: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `get_element_by_class(name, webpage)`.
fn element_by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(text, _)| text)
}

/// `data.get(key)` as an optional string.
fn data_str(data: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    match data.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(unsupported("player fields that are not strings")),
    }
}

/// Python's `str.strip()`.
fn py_strip(text: &str) -> String {
    crate::batch::py_strip(text).to_owned()
}

impl Extractor for Libsyn {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        static DATA: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"var\s+playlistItem\s*=\s*(\{.+?\});").unwrap());
        static DATA_TITLE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"data-title="([^"]+)""#).unwrap());
        static TITLE_TAG: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"<title>(.+?)</title>").unwrap());
        static H3: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<h3>([^<]+)</h3>").unwrap());
        static DESCRIPTION: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"<p\s+id="info_text_body">(.+?)</p>"#).unwrap());
        static RELEASE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"<div class="release_date">Released: ([^<]+)<"#).unwrap()
        });
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let main_url = Url::parse(&found["mainurl"])?;
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, &main_url, NAME, &video_id)?;
        let block = DATA
            .captures(&webpage)
            .ok_or_else(|| regex_error(&video_id, "JSON data block"))?;
        let data: Value = serde_json::from_str(&block[1]).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let data = data
            .as_object()
            .ok_or_else(|| unsupported("player data that is not an object"))?;
        let episode_title = data_str(data, "item_title")?
            .filter(|title| !title.is_empty())
            .or_else(|| element_by_class("episode-title", &webpage))
            .filter(|title| !title.is_empty());
        let episode_title = match episode_title {
            Some(title) => title,
            None => DATA_TITLE
                .captures(&webpage)
                .or_else(|| TITLE_TAG.captures(&webpage))
                .map(|found| found[1].to_owned())
                .ok_or_else(|| regex_error(&video_id, "episode title"))?,
        };
        let episode_title = py_strip(&episode_title);
        let podcast_title = H3
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .filter(|title| !title.is_empty())
            .or_else(|| element_by_class("podcast-title", &webpage))
            .map(|title| py_strip(&crate::html::clean_html(&title)))
            .filter(|title| !title.is_empty());
        let title = match podcast_title {
            Some(podcast) => format!("{podcast} - {episode_title}"),
            None => episode_title,
        };
        let mut formats = Vec::new();
        for (key, format_id) in [
            ("media_url_libsyn", "libsyn"),
            ("media_url", "main"),
            ("download_link", "download"),
        ] {
            let Some(f_url) = data_str(data, key)?.filter(|f_url| !f_url.is_empty()) else {
                continue;
            };
            let mut map = Map::new();
            map.insert("url".into(), f_url.clone().into());
            map.insert("format_id".into(), format_id.into());
            let keys: Vec<String> = map.keys().cloned().collect();
            let ext =
                super::direct::determine_ext(&f_url).unwrap_or_else(|| "unknown_video".into());
            map.insert("ext".into(), ext.into());
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            formats.push(format);
        }
        let description = DESCRIPTION
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .filter(|description| !description.is_empty())
            .map(|description| py_strip(&description.replace('\u{a0}', " ")));
        let release = RELEASE
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .filter(|date| !date.is_empty());
        let release = match release {
            Some(date) => Some(date),
            None => data_str(data, "release_date")?,
        };
        let upload_date = release.and_then(|date| crate::dates::unified_strdate(&date, true));
        // `parse_duration` returns None for anything but a string.
        let duration = match data.get("duration") {
            Some(Value::String(text)) => crate::parse_duration(text),
            _ => None,
        };
        let thumbnail = data_str(data, "thumbnail_url")?;
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = video_id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        crate::metadata_parser::set_field(
            &mut info,
            "description",
            description.map_or(Value::Null, Value::String),
        );
        match thumbnail {
            Some(thumbnail) => info.metadata.thumbnail = Some(thumbnail),
            None => {
                info.extra.insert("thumbnail".into(), Value::Null);
            }
        }
        crate::metadata_parser::set_field(
            &mut info,
            "upload_date",
            upload_date.map_or(Value::Null, Value::String),
        );
        let mut keys = "dict;id,title,description,thumbnail,upload_date".to_owned();
        if let Some(duration) = duration {
            info.metadata.duration = Some(duration);
            keys.push_str(",duration");
        }
        keys.push_str(",formats");
        Ok(Some(crate::info_order::mark_info(info, &keys)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://html5-player.libsyn.com/embed/episode/id/6385796/"
        ));
        assert!(suitable(
            "https://html5-player.libsyn.com/embed/episode/id/3727166/height/75/width/200/theme/standard/direction/no/autoplay/no/autonext/no/thumbnail/no/preload/no/no_addthis/no/"
        ));
        assert!(!suitable("https://html5-player.libsyn.com/embed/episode/"));
    }
}
