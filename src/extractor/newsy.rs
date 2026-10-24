//! Upstream `NewsyIE` (`yt_dlp/extractor/newsy.py`): the page's
//! `data-video-player` attribute (`js_to_json`), its HLS `stream` and
//! metadata, merged under the page's JSON-LD (`merge_dicts`).
use super::{Extractor, download_webpage, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Newsy;

const NAME: &str = "Newsy";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?newsy\.com/stories/(?P<id>[^/?#$&]+)").unwrap()
});

static DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-video-player\s?=\s?"(\{[^"]+\})">"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Newsy {what} are not supported natively"))
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

impl Extractor for Newsy {
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
        // `_html_search_regex(...)`: the cleaned match.
        let data = DATA
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let data: Value = crate::json::from_str(&crate::js_to_json::js_to_json(&data))
            .map_err(|_| unsupported("player data that are not JSON"))?;
        let Value::Object(data) = data else {
            return Err(unsupported("player data that are not objects"));
        };
        // `_search_json_ld(webpage, display_id, fatal=False)`.
        let json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract JSON-LD ; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        if ["url", "formats", "id", "subtitles"]
            .iter()
            .any(|key| json_ld.contains_key(*key))
        {
            return Err(unsupported("JSON-LD media URLs"));
        }
        let mut stream = None;
        if let Some(manifest) = data.get("stream").filter(|value| truthy(value)) {
            let Value::String(manifest) = manifest else {
                return Err(unsupported("streams that are not URLs"));
            };
            let manifest = Url::parse(manifest)?;
            let mut info = extract_m3u8_fatal(client, &manifest, NAME, &display_id, None, true)?;
            super::hls::without_ext(&mut info);
            stream = Some(info);
        }
        let id = data.get("id").ok_or_else(|| {
            Error::Reported(format!(
                "{display_id}: An extractor error has occurred. (caused by KeyError('id')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let id = match id {
            Value::String(id) => id.clone(),
            Value::Number(id) => id.to_string(),
            _ => return Err(unsupported("IDs that are not strings")),
        };
        let mut info = match stream {
            Some(info) => info,
            None => {
                let mut info = super::direct::build_at(url.clone(), url, String::new());
                info.formats.clear();
                info.extra
                    .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
                info
            }
        };
        info.direct = None;
        info.id = id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        // `merge_dicts(ld_json, {...})`: the JSON-LD's values win; a later
        // value fills a missing (or empty-string) one.
        let mut merged: Map<String, Value> = json_ld;
        for (key, value) in [
            ("id", Value::Null),
            ("display_id", Value::from(display_id.clone())),
            (
                "title",
                data.get("headline").cloned().unwrap_or(Value::Null),
            ),
            (
                "duration",
                data.get("duration").cloned().unwrap_or(Value::Null),
            ),
            (
                "thumbnail",
                data.get("image").cloned().unwrap_or(Value::Null),
            ),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ] {
            if matches!(key, "id" | "formats" | "subtitles") {
                merged.entry(key).or_insert(Value::Null);
                continue;
            }
            if value.is_null() {
                continue;
            }
            match merged.get(key) {
                None => {
                    merged.insert(key.into(), value);
                }
                Some(Value::String(existing))
                    if existing.is_empty() && value.as_str().is_some_and(|v| !v.is_empty()) =>
                {
                    merged.insert(key.into(), value);
                }
                _ => {}
            }
        }
        let keys: Vec<String> = merged.keys().cloned().collect();
        let mut title = None;
        for (key, value) in merged {
            match key.as_str() {
                "id" | "formats" | "subtitles" => {}
                "title" => match value {
                    Value::String(text) => title = Some(text),
                    _ => return Err(unsupported("titles that are not strings")),
                },
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
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
            "https://www.newsy.com/stories/nft-trend-leads-to-fraudulent-art-auctions/"
        ));
        assert!(suitable("http://newsy.com/stories/x"));
        assert!(!suitable("https://www.newsy.com/videos/x"));
    }
}
