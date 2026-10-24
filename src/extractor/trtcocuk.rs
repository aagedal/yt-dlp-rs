//! Upstream `TrtCocukVideoIE` (`yt_dlp/extractor/trtcocuk.py`): the page's
//! `window.__NUXT__` data (`_search_nuxt_data`), its HLS `video` (a JSON
//! string or a plain URL) and metadata, with the page `<title>`.
use super::nuxt::{Step, search_nuxt_data};
use super::{Extractor, download_webpage, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct TrtCocukVideo;

const NAME: &str = "TrtCocukVideo";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://www\.trtcocuk\.net\.tr/video/(?P<id>[\w-]+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("TRT Çocuk {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// Python's `str(value)` for scalar JSON values.
fn python_str(value: &Value) -> Option<String> {
    Some(match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        _ => return None,
    })
}

impl Extractor for TrtCocukVideo {
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
        // `[...]['data']`.
        let data = match &nuxt {
            Value::Object(map) => map
                .get("data")
                .ok_or_else(|| key_error(&display_id, "data"))?,
            _ => return Err(unsupported("Nuxt data that are not objects")),
        };
        let Value::Object(data) = data else {
            return Err(unsupported("video data that are not objects"));
        };
        // `_parse_json(data['video'])`, else the raw value.
        let video = data
            .get("video")
            .ok_or_else(|| key_error(&display_id, "video"))?;
        let Value::String(video) = video else {
            return Err(unsupported("video values that are not strings"));
        };
        let video_url = match crate::json::from_str::<Value>(video) {
            Ok(Value::String(parsed)) => parsed,
            Ok(_) => return Err(unsupported("video values that are not URLs")),
            Err(_) => video.clone(),
        };
        let id = data.get("id").ok_or_else(|| key_error(&display_id, "id"))?;
        let id = python_str(id).ok_or_else(|| unsupported("video IDs that are not scalars"))?;
        let manifest = Url::parse(&video_url)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &display_id, None, true)?;
        super::hls::without_ext(&mut info);
        info.direct = None;
        info.id = id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        match crate::html::html_extract_title(&webpage) {
            Some(title) => info.title = title,
            None => {
                // `_html_extract_title(webpage)` (non-fatal).
                super::search_warning(NAME, "title");
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        let season = super::streamable::int_or_none(data.get("season"));
        let release = data
            .get("publishedDate")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);
        let series = data
            .get("show")
            .and_then(|show| show.get(0))
            .and_then(|show| show.get("title"))
            .cloned()
            .unwrap_or(Value::Null);
        for (key, value) in [
            ("season_number", season),
            ("release_timestamp", release),
            ("series", series),
        ] {
            if !value.is_null() {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,formats,subtitles,season_number,release_timestamp,series,title",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.trtcocuk.net.tr/video/kaptan-pengu-ve-arkadaslari-1"
        ));
        assert!(suitable(
            "https://www.trtcocuk.net.tr/video/sef-rokanin-lezzet-dunyasi-17"
        ));
        assert!(!suitable("https://trtcocuk.net.tr/video/x"));
        assert!(!suitable("https://www.trtcocuk.net.tr/oyun/x"));
    }
}
