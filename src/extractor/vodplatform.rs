//! Upstream `VODPlatformIE` (`yt_dlp/extractor/vodplatform.py`): VOD
//! Platform and KwikMotion embeds from the page's hidden HLS (else DASH)
//! link as Wowza formats, with the OpenGraph title and the hidden (else
//! OpenGraph) thumbnail; their iframes on other pages are embeds.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct VodPlatform;

pub(crate) const NAME: &str = "VODPlatform";
pub(crate) const KEY: &str = "VODPlatform";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www\.)?vod-platform\.net|embed\.kwikmotion\.com)/[eE]mbed/(?P<id>[^/?#]+)",
    )
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+src=(?:"(?P<dq>(?:https?:)?//(?:(?:www\.)?vod-platform\.net|embed\.kwikmotion\.com)/[eE]mbed/.+?)"|'(?P<sq>(?:https?:)?//(?:(?:www\.)?vod-platform\.net|embed\.kwikmotion\.com)/[eE]mbed/.+?)')"#,
    )
    .unwrap()
});

/// `_match_id`.
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
        if !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

impl Extractor for VodPlatform {
    fn key(&self) -> &'static str {
        KEY
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
        let search = |value: Option<String>, name: &str| {
            if value.is_none() {
                super::search_warning(NAME, name);
            }
            value
        };
        // `unescapeHTML(self._og_search_title(webpage))`: unescaped twice.
        let title = search(
            crate::html::og_search_property(&webpage, "title"),
            "OpenGraph title",
        )
        .map(|title| crate::html::unescape_html(&title));
        let hidden = super::login::hidden_inputs(&webpage);
        let input = |name: &str| {
            hidden
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        let stream = match input("HiddenmyhHlsLink").filter(|link| !link.is_empty()) {
            Some(link) => link,
            None => input("HiddenmyDashLink").ok_or_else(|| {
                Error::Reported(format!(
                    "{video_id}: An extractor error has occurred. (caused by KeyError('HiddenmyDashLink')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?,
        };
        let formats =
            super::extract_wowza_formats(client, &stream, NAME, &video_id, &["f4m", "smil"])?;
        let thumbnail = input("HiddenThumbnail")
            .filter(|thumbnail| !thumbnail.is_empty())
            .or_else(|| {
                search(
                    crate::html::og_search_property(&webpage, "image"),
                    "thumbnail URL",
                )
            });
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", optional(title)),
                ("thumbnail", optional(thumbnail)),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://vod-platform.net/embed/RufMcytHDolTH1MuKHY9Fw"
        ));
        assert!(suitable("https://embed.kwikmotion.com/Embed/abc"));
        assert!(!suitable("https://vod-platform.net/watch/abc"));
    }
}
