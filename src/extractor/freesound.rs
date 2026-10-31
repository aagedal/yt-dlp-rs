//! Upstream `FreesoundIE` (`yt_dlp/extractor/freesound.py`): the sound page's
//! `og:audio` properties, description, duration, date, channels and tags, and
//! the low/high quality MP3 previews as formats.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Freesound;

const NAME: &str = "Freesound";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?freesound\.org/people/[^/]+/sounds/(?P<id>[^/]+)",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_element_by_class` / `get_element_by_id`: the first element's text.
fn element_text(attribute: &str, value: &str, html: &str) -> Option<String> {
    let found = if attribute == "class" {
        crate::html_elements::elements_text_and_html_by_class(value, html)
    } else {
        crate::html_elements::elements_text_and_html_by_attribute(attribute, value, html)
    };
    found.ok()?.into_iter().next().map(|(text, _)| text)
}

/// `_html_search_regex(PATTERN, webpage, NAME, fatal=False)`.
fn html_search(pattern: &Regex, html: &str, name: &str) -> Option<String> {
    let found = pattern
        .captures(html)
        .and_then(|found| found.get(1))
        .map(|text| crate::html::clean_html(text.as_str()));
    if found.is_none() {
        super::search_warning(NAME, name);
    }
    found
}

fn og_fatal(html: &str, prop: &str, name: &str, id: &str) -> Result<String> {
    crate::html::og_search_property(html, prop).ok_or_else(|| {
        Error::Reported(format!(
            "[{NAME}] {id}: Unable to extract {name}; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })
}

fn optional(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::String)
}

impl Extractor for Freesound {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"(?s)id=["']sound_description["'][^>]*>(.+?)</div>"#).unwrap()
        });
        static CHANNELS: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"Channels</dt><dd>(.+?)</dd>").unwrap());
        static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<a[^>]+>([^<]+)").unwrap());
        static DOUBLE_HOST: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^https?://freesound\.org(https?://)").unwrap());
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let audio_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &audio_id)?;
        let audio_url = og_fatal(&webpage, "audio", "song url", &audio_id)?;
        let title = og_fatal(&webpage, "audio:title", "song title", &audio_id)?;
        let description = html_search(&DESCRIPTION, &webpage, "description");
        // `float_or_none(get_element_by_class('duration', webpage), scale=1000)`.
        let duration = element_text("class", "duration", &webpage)
            .and_then(|text| text.trim().parse::<f64>().ok())
            .and_then(|value| serde_json::Number::from_f64(value / 1000.0));
        let upload_date = element_text("id", "sound_date", &webpage)
            .and_then(|text| crate::dates::unified_strdate(&text, true));
        let uploader = crate::html::og_search_property(&webpage, "audio:artist");
        if uploader.is_none() {
            super::search_warning(NAME, "uploader");
        }
        let channels = html_search(&CHANNELS, &webpage, "channels info");
        let tags = element_text("class", "tags", &webpage)
            .filter(|tags| !tags.is_empty())
            .map(|html| {
                TAG.captures_iter(&html)
                    .map(|found| Value::String(found[1].to_owned()))
                    .collect::<Vec<_>>()
            });
        let audio_url = DOUBLE_HOST.replace(&audio_url, "$1").into_owned();
        let mut audio_urls = vec![audio_url.clone()];
        if audio_url.contains("-lq.mp3") {
            audio_urls.push(audio_url.replace("-lq.mp3", "-hq.mp3"));
        }
        let mut formats = Vec::new();
        for (quality, format_url) in audio_urls.iter().enumerate() {
            let mut map = Map::new();
            map.insert("url".into(), format_url.clone().into());
            map.insert("format_note".into(), optional(channels.clone()));
            map.insert("quality".into(), json!(quality));
            let keys: Vec<String> = map.keys().cloned().collect();
            let ext =
                super::direct::determine_ext(format_url).unwrap_or_else(|| "unknown_video".into());
            map.insert("ext".into(), ext.into());
            // Processing's `str(index)` for formats without an ID.
            map.insert("format_id".into(), quality.to_string().into());
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| Error::Reported(format!("[{NAME}] {audio_id}: {error}")))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            crate::info_order::mark_int_quality(&mut format);
            formats.push(format);
        }
        let first = Url::parse(&audio_urls[0])?;
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = audio_id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(&mut info, "description", optional(description));
        let mut keys = vec!["id", "title", "description"];
        if let Some(duration) = duration {
            crate::metadata_parser::set_field(&mut info, "duration", Value::Number(duration));
            keys.push("duration");
        }
        crate::metadata_parser::set_field(&mut info, "uploader", optional(uploader));
        crate::metadata_parser::set_field(&mut info, "upload_date", optional(upload_date));
        crate::metadata_parser::set_field(
            &mut info,
            "tags",
            tags.map_or(Value::Null, Value::Array),
        );
        keys.extend(["uploader", "upload_date", "tags", "formats"]);
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
            "http://www.freesound.org/people/miklovan/sounds/194503/"
        ));
        assert!(suitable("https://freesound.org/people/a/sounds/1"));
        assert!(!suitable("https://freesound.org/people/miklovan/"));
    }
}
