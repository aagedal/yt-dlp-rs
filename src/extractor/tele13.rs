//! Upstream `Tele13IE` (`yt_dlp/extractor/tele13.py`): T13 videos from the
//! page's `player-vivo` JW Player setup: its deduplicated sources (HLS,
//! YouTube as a `url` result, else files ranked by label) with the setup's
//! title and image and the page description.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Tele13;

const NAME: &str = "Tele13";
const LABELS: [&str; 3] = ["Móvil", "SD", "HD"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?t13\.cl/videos(?:/[^/]+)+/(?P<id>[\w-]+)").unwrap()
});
static SETUP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)jwplayer\('player-vivo'\).setup\((\{.*?\})\)").unwrap());
static SOURCES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"sources\s*:\s*(\[[^\]]+\])").unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"title\s*:\s*"([^"]+)""#).unwrap());
static IMAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"image\s*:\s*"([^"]+)""#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn search(regex: &Regex, text: &str, id: &str, name: &str) -> Result<String> {
    regex
        .captures(text)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("T13 {what} are not supported natively"))
}

impl Extractor for Tele13 {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
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
        let setup_js = search(&SETUP, &webpage, &display_id, "setup code")?;
        let raw = search(&SOURCES, &setup_js, &display_id, "sources")?;
        let sources: Value =
            crate::json::from_str(&crate::js_to_json::js_to_json(&raw)).map_err(|error| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Failed to parse JSON  (caused by {error})"
                ))
            })?;
        let Value::Array(sources) = sources else {
            return Err(unsupported("source lists that are not lists"));
        };
        let mut formats = Vec::new();
        let mut urls: Vec<String> = Vec::new();
        for source in &sources {
            let file = source.get("file").ok_or_else(|| {
                Error::Reported(format!(
                    "{display_id}: An extractor error has occurred. (caused by KeyError('file')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
            if !super::nuxt::truthy(file) {
                continue;
            }
            let Value::String(file) = file else {
                return Err(unsupported("source files that are not strings"));
            };
            if urls.contains(file) {
                continue;
            }
            let ext = super::direct::determine_ext(file);
            if ext.as_deref() == Some("m3u8") {
                if let Some(stream) = Url::parse(file).ok().and_then(|manifest| {
                    super::extract_m3u8_master_optional(
                        client,
                        &manifest,
                        NAME,
                        &display_id,
                        Some("hls"),
                        false,
                        false,
                    )
                }) {
                    formats.extend(stream.formats);
                }
            } else if super::youtube::suitable(file) {
                return Ok(Some(super::follow_url(url, file.clone())));
            } else {
                let label = source.get("label").cloned().unwrap_or(Value::Null);
                // `qualities(['Móvil', 'SD', 'HD'])(label)`.
                let quality = LABELS
                    .iter()
                    .position(|known| label.as_str() == Some(*known))
                    .map_or(-1, |index| index as i64);
                formats.push(dict_format(
                    vec![
                        ("url", file.clone().into()),
                        ("format_id", label),
                        ("quality", quality.into()),
                        ("ext", ext.map_or(Value::Null, Value::from)),
                    ],
                    NAME,
                    &display_id,
                )?);
            }
            urls.push(file.clone());
        }
        let title = search(&TITLE, &setup_js, &display_id, "title")?;
        let description = crate::html::html_search_meta(&webpage, "description");
        if description.is_none() {
            super::search_warning(NAME, "description");
        }
        let thumbnail = IMAGE.captures(&setup_js).map(|found| found[1].to_owned());
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", display_id.into()),
                ("title", title.into()),
                ("description", optional(description)),
                ("thumbnail", optional(thumbnail)),
                ("formats", Value::Null),
            ],
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.t13.cl/videos/actualidad/el-circulo-de-hierro-de-michelle-bachelet-en-su-regreso-a-la-moneda"
        ));
        assert!(!suitable("http://www.t13.cl/noticias/x"));
    }
}
