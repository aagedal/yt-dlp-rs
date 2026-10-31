//! Upstream `UnitedNationsWebTvIE` (`yt_dlp/extractor/unitednations.py`):
//! the asset page's `partnerId` and `kentryID` as a `url` result of
//! `kaltura:PARTNER:ENTRY`.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct UnitedNationsWebTv;

const NAME: &str = "UnitedNationsWebTv";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://webtv\.un\.org/(?:ar|zh|en|fr|ru|es)/asset/\w+/(?P<id>\w+)",
    )
    .unwrap()
});
static PARTNER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"partnerId:\s*(\d+)").unwrap());
static ENTRY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"const\s+kentryID\s*=\s*["'](\w+)["']"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_html_search_regex(PATTERN, webpage, NAME)` (fatal).
fn search(pattern: &Regex, webpage: &str, id: &str, name: &str) -> Result<String> {
    pattern
        .captures(webpage)
        .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

impl Extractor for UnitedNationsWebTv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        let partner_id = search(&PARTNER, &webpage, &id, "partner_id")?;
        let entry_id = search(&ENTRY, &webpage, &id, "kentry_id")?;
        Ok(Some(super::kaltura::follow(
            url,
            format!("kaltura:{partner_id}:{entry_id}"),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://webtv.un.org/en/asset/k1o/k1o7stmi6p"));
        assert!(!suitable("https://webtv.un.org/de/asset/k1o/k1o7stmi6p"));
    }
}
