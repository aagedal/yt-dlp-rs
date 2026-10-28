//! Upstream `APAIE` (`yt_dlp/extractor/apa.py`): APA video embeds from the
//! player page: a JW Platform media ID as a `url` result, else its HLS and
//! progressive sources with the title, description and poster; APA embed
//! iframes on other pages are embeds.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Apa;

pub(crate) const NAME: &str = "APA";
pub(crate) const KEY: &str = "APA";

const UUID: &str = r"[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?P<base_url>https?://[^/]+\.apa\.at)/embed/(?P<id>{UUID})"
    ))
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"<iframe[^>]+\bsrc=(?:"(?P<dq>(?:https?:)?//[^/]+\.apa\.at/embed/{UUID}.*?)"|'(?P<sq>(?:https?:)?//[^/]+\.apa\.at/embed/{UUID}.*?)')"#
    ))
    .unwrap()
});
static JWPLATFORM_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"media[iI]d\s*:\s*["'](?P<id>[a-zA-Z0-9]{8})"#).unwrap());
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\.mp4").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Some(raw) = captures.name("dq").or_else(|| captures.name("sq")) else {
            continue;
        };
        let Ok(embed) = url.join(&crate::html::unescape_html(raw.as_str())) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

/// `extract(field)`: `\bFIELD["']\s*:\s*(["'])(?P<value>(?:(?!\1).)+)\1`.
fn extract(webpage: &str, field: &str) -> Option<String> {
    let start = Regex::new(&format!(r#"\b{}["']\s*:\s*(["'])"#, regex::escape(field))).ok()?;
    let mut from = 0;
    while let Some(found) = start.captures_at(webpage, from) {
        let whole = found.get(0)?;
        let quote = found[1].chars().next()?;
        let rest = &webpage[whole.end()..];
        let value: String = rest
            .chars()
            .take_while(|c| *c != quote && *c != '\n')
            .collect();
        if !value.is_empty() && rest[value.len()..].starts_with(quote) {
            return Some(value);
        }
        from = whole.start() + 1;
    }
    None
}

impl Extractor for Apa {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let base_url = found["base_url"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let player = Url::parse(&format!("{base_url}/player/{video_id}"))?;
        let webpage = download_webpage(client, &player, NAME, &video_id)?;
        if let Some(jwplatform_id) = JWPLATFORM_ID.captures(&webpage) {
            return Ok(Some(super::follow_url(
                url,
                format!("jwplatform:{}", &jwplatform_id["id"]),
            )));
        }
        let title = extract(&webpage, "title").unwrap_or_else(|| video_id.clone());
        let description = extract(&webpage, "description");
        let thumbnail = extract(&webpage, "poster");
        let mut formats = Vec::new();
        for format_id in ["hls", "progressive"] {
            let Some(source_url) =
                extract(&webpage, format_id).and_then(|url| super::json_ld::url_or_none(&url))
            else {
                continue;
            };
            if super::direct::determine_ext(&source_url).as_deref() == Some("m3u8") {
                if let Some(stream) = Url::parse(&source_url).ok().and_then(|manifest| {
                    super::extract_m3u8_master_optional(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some("hls"),
                        false,
                        false,
                    )
                }) {
                    formats.extend(stream.formats);
                }
            } else {
                let height = HEIGHT
                    .captures(&source_url)
                    .and_then(|found| found[1].parse::<i64>().ok());
                formats.push(dict_format(
                    vec![
                        ("url", source_url.into()),
                        ("format_id", format_id.into()),
                        ("height", height.map_or(Value::Null, Value::from)),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
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
            "http://uvp.apa.at/embed/293f6d17-692a-44e3-9fd5-7b178f3a1029"
        ));
        assert!(!suitable(
            "http://uvp.apa.at/player/293f6d17-692a-44e3-9fd5-7b178f3a1029"
        ));
        assert_eq!(
            extract("x {\"title\": 'A \"b\"', \"description\": \"\"}", "title").as_deref(),
            Some("A \"b\"")
        );
        assert_eq!(
            extract("\"description\": \"\" 'description': 'd'", "description").as_deref(),
            Some("d")
        );
    }
}
