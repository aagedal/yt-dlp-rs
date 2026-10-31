//! Upstream `FrancaisFacileIE` (`yt_dlp/extractor/francaisfacile.py`): RFI
//! Français facile audio from the page's `data-media-id` JSON (media ID,
//! title, first source URL and duration), the page title and its JSON-LD.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct FrancaisFacile;

const NAME: &str = "FrancaisFacile";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://francaisfacile\.rfi\.fr/[a-z]{2}/(?:actualit%C3%A9|podcasts/[^/#?]+)/(?P<id>[^/#?]+)",
    )
    .unwrap()
});
static AUDIO_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<script[^>]+\bdata-media-id=[^>]+\btype="application/json"[^>]*>\s*"#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Français facile {what} are not supported natively"))
}

/// `traverse_obj(data, ('sources', ..., KEY, ...))`'s values.
fn sources<'a>(data: &'a Value, key: &str) -> Vec<&'a Value> {
    let items: Vec<&Value> = match data.get("sources") {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    items.into_iter().filter_map(|item| item.get(key)).collect()
}

impl Extractor for FrancaisFacile {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        // `urllib.parse.unquote(self._match_id(url))`.
        let display_id = percent_encoding::percent_decode_str(&found["id"])
            .decode_utf8_lossy()
            .into_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let data = super::search_json(&AUDIO_DATA, &webpage, false)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract audio data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .ok_or_else(|| unsupported("audio data that do not parse"))?;
        let numeric_id = matches!(data.get("mediaId"), Some(Value::Number(_)));
        let id = match data.get("mediaId") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            Some(_) => return Err(unsupported("media IDs that are not strings")),
            None => {
                return Err(Error::Reported(format!(
                    "{display_id}: An extractor error has occurred. (caused by KeyError('mediaId')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        // `_html_extract_title(webpage)` (non-fatal).
        let title = crate::html::html_extract_title(&webpage);
        if title.is_none() {
            super::search_warning(NAME, "title");
        }
        let mut fields: Vec<(String, Value)> = vec![
            ("id".into(), id.clone().into()),
            ("display_id".into(), display_id.into()),
            ("vcodec".into(), "none".into()),
            ("title".into(), title.map_or(Value::Null, Value::from)),
        ];
        let mut set =
            |key: &str, value: Value| match fields.iter_mut().find(|(name, _)| name == key) {
                Some((_, old)) => *old = value,
                None => fields.push((key.to_owned(), value)),
            };
        // `**_search_json_ld(webpage, display_id, fatal=False)`.
        let json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract JSON-LD ; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        for (key, value) in json_ld {
            set(&key, value);
        }
        if let Some(title) = data.get("title").filter(|title| title.is_string()) {
            set("title", title.clone());
        }
        let media = sources(&data, "url")
            .into_iter()
            .filter_map(Value::as_str)
            .find_map(super::json_ld::url_or_none);
        if let Some(media) = &media {
            set("url", media.clone().into());
        }
        let duration = sources(&data, "duration")
            .into_iter()
            .find_map(|value| match value {
                Value::Number(number) => number.as_f64(),
                Value::String(text) => text.trim().parse::<f64>().ok(),
                _ => None,
            });
        if let Some(duration) = duration {
            set("duration", json!(duration));
        }
        let formats = match &media {
            Some(media) => {
                let ext = super::direct::determine_ext(media)
                    .unwrap_or_else(|| "unknown_video".into())
                    .to_lowercase();
                let mut format: crate::VideoFormat = serde_json::from_value(
                    json!({"format_id": "", "url": media, "ext": ext, "vcodec": "none"}),
                )
                .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
                crate::info_order::mark_format(&mut format, "url-only");
                vec![format]
            }
            None => Vec::new(),
        };
        let fields: Vec<(&str, Value)> = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("untyped-dict;{}", keys.join(","));
        let info = dict_info(url, (NAME, NAME), fields, formats);
        if numeric_id {
            // Processing's `_fill_common_fields` warning.
            crate::screen::warning(
                "\"id\" field is not a string - forcing string conversion, there is an error in extractor",
            );
        }
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://francaisfacile.rfi.fr/fr/podcasts/journal-en-fran%C3%A7ais-facile/20250127-journal-en-fran%C3%A7ais-facile-20h00-gmt"
        ));
        assert!(suitable(
            "https://francaisfacile.rfi.fr/fr/actualit%C3%A9/20250305-r%C3%A9concilier-les-jeunes"
        ));
        assert!(!suitable("https://francaisfacile.rfi.fr/fr/exercices/x"));
    }
}
