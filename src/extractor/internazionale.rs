//! Upstream `InternazionaleIE` (`yt_dlp/extractor/internazionale.py`): the
//! video page's `data-job-id`/`data-video-path` attributes, the non-fatal
//! HLS and DASH manifests they name and the page's OpenGraph metadata.
use super::{Extractor, download_webpage, extract_m3u8_optional, extract_mpd_optional};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Internazionale;

const NAME: &str = "Internazionale";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?internazionale\.it/video/(?:[^/]+/)*(?P<id>[^/?#&]+)",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `DATA_RE % name`: `data-NAME=(["'])(?P<value>(?:(?!\1).)+)\1`.
fn data_attribute(webpage: &str, name: &str) -> Option<String> {
    let name = regex::escape(name);
    let pattern = Regex::new(&format!(
        r#"data-{name}=(?:"(?P<a>[^"\n]+)"|'(?P<b>[^'\n]+)')"#
    ))
    .expect("escaped pattern");
    let found = pattern.captures(webpage)?;
    found
        .name("a")
        .or_else(|| found.name("b"))
        .map(|value| value.as_str().to_owned())
}

/// A fatal `_search_regex` miss.
fn required(webpage: &str, attribute: &str, name: &str, id: &str) -> Result<String> {
    data_attribute(webpage, attribute).ok_or_else(|| {
        Error::Reported(format!(
            "[{NAME}] {id}: Unable to extract {name}; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })
}

/// A non-fatal `_og_search_property` / `_html_search_meta`.
fn warn_missing(value: Option<String>, name: &str) -> Option<String> {
    if value.is_none() {
        super::search_warning(NAME, name);
    }
    value
}

impl Extractor for Internazionale {
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
        let title = match data_attribute(&webpage, "video-title") {
            Some(title) => Some(title),
            None => warn_missing(
                crate::html::og_search_property(&webpage, "title"),
                "OpenGraph title",
            ),
        };
        let video_id = required(&webpage, "job-id", "video id", &display_id)?;
        let video_path = required(&webpage, "video-path", "video path", &display_id)?;
        let abroad =
            data_attribute(&webpage, "video-available_abroad").is_none_or(|value| value == "1");
        let video_base = format!(
            "https://video{}.internazionale.it/{video_path}/{video_id}.",
            if abroad { "" } else { "-ita" }
        );
        let mut formats = Vec::new();
        let hls = Url::parse(&format!("{video_base}m3u8"))?;
        if let Some(stream) =
            extract_m3u8_optional(client, &hls, NAME, &display_id, Some("hls"), false)
        {
            for mut format in stream.formats {
                format.ext = "mp4".into();
                formats.push(format);
            }
        }
        let dash = Url::parse(&format!("{video_base}mpd"))?;
        if let Some(stream) = extract_mpd_optional(client, &dash, NAME, &display_id, false) {
            for mut format in stream.formats {
                format.format_id = format!("dash-{}", format.format_id);
                formats.push(format);
            }
        }
        let timestamp = warn_missing(
            crate::html::html_search_meta(&webpage, "article:published_time"),
            "timestamp",
        )
        .and_then(|date| crate::dates::unified_timestamp(&date, false));
        let thumbnail = warn_missing(
            crate::html::og_search_property(&webpage, "image"),
            "thumbnail URL",
        );
        let description = warn_missing(
            crate::html::og_search_property(&webpage, "description"),
            "OpenGraph description",
        );
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.direct = None;
        info.id = video_id;
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(&mut info, "display_id", display_id.into());
        match thumbnail {
            Some(thumbnail) => info.metadata.thumbnail = Some(thumbnail),
            None => {
                info.extra.insert("thumbnail".into(), Value::Null);
            }
        }
        crate::metadata_parser::set_field(
            &mut info,
            "description",
            description.map_or(Value::Null, Value::String),
        );
        crate::metadata_parser::set_field(
            &mut info,
            "timestamp",
            timestamp.map_or(Value::Null, Value::from),
        );
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,display_id,title,thumbnail,description,timestamp,formats",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.internazionale.it/video/2015/01/19/richard-linklater-racconta-una-scena-di-boyhood"
        ));
        assert!(suitable("https://internazionale.it/video/x"));
        assert!(!suitable("https://www.internazionale.it/notizie/x"));
    }
}
