//! Upstream `DBTVIE` (`yt_dlp/extractor/dbtv.py`): Dagbladet videos as
//! transparent `url` results: 11-character IDs for YouTube, 8-character IDs
//! for JW Platform; Dagbladet embed iframes on other pages are embeds.
use super::Extractor;
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Dbtv;

pub(crate) const NAME: &str = "DBTV";
pub(crate) const KEY: &str = "DBTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?dagbladet\.no/video/(?:(?:embed|(?P<display_id>[^/]+))/)?(?P<id>[0-9A-Za-z_-]{11}|[a-zA-Z0-9]{8})",
    )
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+src=(?:"(?P<dq>(?:https?:)?//(?:www\.)?dagbladet\.no/video/embed/(?:[0-9A-Za-z_-]{11}|[a-zA-Z0-9]{8}).*?)"|'(?P<sq>(?:https?:)?//(?:www\.)?dagbladet\.no/video/embed/(?:[0-9A-Za-z_-]{11}|[a-zA-Z0-9]{8}).*?)')"#,
    )
    .unwrap()
});

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

impl Extractor for Dbtv {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found.name("display_id").map(|id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let target = if video_id.len() == 11 {
            video_id.clone()
        } else {
            format!("jwplatform:{video_id}")
        };
        let mut fields = Map::new();
        fields.insert("id".into(), video_id.into());
        fields.insert(
            "display_id".into(),
            display_id.map_or(Value::Null, Value::from),
        );
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.dagbladet.no/video/PynxJnNWChE/"));
        assert!(suitable(
            "https://www.dagbladet.no/video/embed/xlGmyIeN9Jo/?autoplay=false"
        ));
        assert!(suitable(
            "https://www.dagbladet.no/video/truer-iran-bor-passe-dere/PalfB2Cw"
        ));
        assert_eq!(
            temp_id("https://www.dagbladet.no/video/x/PalfB2Cw").as_deref(),
            Some("PalfB2Cw")
        );
    }
}
