//! Upstream `DaystarClipIE` (`yt_dlp/extractor/daystar.py`): the player
//! page's iframe, its `config2.php` configuration's `sources` (`js_to_json`)
//! as HLS formats and subtitles, and the page's meta title/description.
use super::{Extractor, download_webpage, download_webpage_note_headers};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct DaystarClip;

const NAME: &str = "daystar:clip";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://player\.daystar\.tv/(?P<id>\w+)").unwrap());

static IFRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<iframe[^>]+src="([^"]+)""#).unwrap());

static SOURCES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"sources:\s*(\[.*?\])").unwrap());

static THUMBNAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"image:\s*"([^"]+)"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Daystar {what} are not supported natively"))
}

/// A fatal `_search_regex` miss.
fn unable(id: &str, name: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_html_search_meta([og, twitter], webpage)` (non-fatal).
fn meta(webpage: &str, names: [&str; 2]) -> Option<String> {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name));
    if found.is_none() {
        crate::screen::warning(&format!(
            "[{NAME}] unable to extract {}; {}",
            names[0],
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    found
}

impl Extractor for DaystarClip {
    fn key(&self) -> &'static str {
        "DaystarClip"
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
        let src_iframe = IFRAME
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| unable(&video_id, "src iframe"))?;
        let config = Url::parse(&src_iframe.replace("player.php", "config2.php"))?;
        let webpage_iframe = download_webpage_note_headers(
            client,
            &config,
            NAME,
            &video_id,
            "Downloading webpage",
            &[("Referer", &src_iframe)],
        )?;
        let sources = SOURCES
            .captures(&webpage_iframe)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| unable(&video_id, "m3u8 source"))?;
        let sources: Value = crate::json::from_str(&crate::js_to_json::js_to_json(&sources))
            .map_err(|_| unsupported("sources that are not JSON"))?;
        let Value::Array(sources) = sources else {
            return Err(unsupported("sources that are not lists"));
        };
        let base = Url::parse("https://www.lightcast.com/embed/")?;
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        for source in &sources {
            let Value::Object(source) = source else {
                return Err(unsupported("sources that are not objects"));
            };
            let file = source.get("file").filter(|file| super::nuxt::truthy(file));
            let Some(file) = file else { continue };
            if source.get("type").and_then(Value::as_str) != Some("m3u8") {
                continue;
            }
            let Value::String(file) = file else {
                return Err(unsupported("source files that are not strings"));
            };
            let Ok(manifest) = base.join(file) else {
                return Err(unsupported("source files that are not URLs"));
            };
            if let Some(mut stream) = super::extract_m3u8_master_optional_headers(
                client,
                &manifest,
                NAME,
                &video_id,
                &[("Referer", &src_iframe)],
            ) {
                formats.append(&mut stream.formats);
                super::html5::merge_subtitles(
                    &mut subtitles,
                    std::mem::take(&mut stream.subtitles),
                );
            }
        }
        let title = meta(&webpage, ["og:title", "twitter:title"]);
        let description = meta(&webpage, ["og:description", "twitter:description"]);
        let thumbnail = THUMBNAIL
            .captures(&webpage_iframe)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| unable(&video_id, "thumbnail"))?;
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.subtitles = subtitles;
        info.direct = None;
        info.id = video_id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = "DaystarClip".into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        match description {
            Some(description) => info.metadata.description = Some(description),
            None => {
                info.extra.insert("description".into(), Value::Null);
            }
        }
        info.metadata.thumbnail = Some(thumbnail);
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,description,thumbnail,formats,subtitles",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://player.daystar.tv/0MTO2ITM"));
        assert!(suitable("http://player.daystar.tv/abc_1"));
        assert!(!suitable("https://www.daystar.tv/0MTO2ITM"));
    }
}
