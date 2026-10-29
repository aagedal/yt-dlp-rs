//! Upstream `EmbedlyIE` (`yt_dlp/extractor/embedly.py`): an
//! `embedly.com/widgets/media.html` wrapper's `url` (YouTube tab/watch) or
//! `src` target, followed with the wrapper page as the referer. The
//! page-embedded (`_extract_from_webpage`) form is not ported.
use super::{Extractor, follow_url};
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::json;
use std::sync::LazyLock;
use url::Url;

pub struct Embedly;

const NAME: &str = "Embedly";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www|cdn\.)?embedly\.com/widgets/media\.html\?(?:[^#]*?&)?(?:src|url)=(?:[^#&]+)",
    )
    .unwrap()
});

/// `_match_id`-free `suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `parse_qs(url)`'s first value of `key`, unquoted once more as upstream
/// does (`urllib.parse.unquote`).
fn first_value(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| {
            percent_encoding::percent_decode_str(&value)
                .decode_utf8_lossy()
                .into_owned()
        })
}

impl Extractor for Embedly {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !VALID_URL.is_match(url.as_str()) {
            return Ok(None);
        }
        let src = first_value(url, "url").unwrap_or_default();
        if !src.is_empty() && super::youtube::extractor_suitable("youtube:tab", &src) {
            // `url_result(src, YoutubeTabIE)`; the tab ie is not forced here.
            return Ok(Some(follow_url(url, src)));
        }
        let target = first_value(url, "src")
            .or_else(|| first_value(url, "url"))
            .unwrap_or_default();
        Ok(Some(follow_url(
            url,
            super::brightcove::smuggle(&target, &json!({"referer": url.as_str()})),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://cdn.embedly.com/widgets/media.html?src=http%3A%2F%2Fwww.youtube.com%2Fembed%2Fvideoseries&url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3DSU4fj_aEMVw"
        ));
        assert!(!suitable("https://embedly.com/"));
    }
}
