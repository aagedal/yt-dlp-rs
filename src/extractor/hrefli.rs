//! Upstream `HrefLiRedirectIE` (`yt_dlp/extractor/hrefli.py`): the URL after
//! `href.li/?` as a `url` result, followed when processed.
use super::Extractor;
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct HrefLiRedirect;

const NAME: &str = "href.li";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| crate::extractor::url_regex(r"^https?://href\.li/\?(?P<url>.+)").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for HrefLiRedirect {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `url_result(url)`: followed by whichever extractor suits it.
        // A target without a scheme reaches `GenericIE`'s invalid-URL error
        // (its `http://` guess for domain-like text is not ported).
        let target = Url::parse(&found["url"]).map_err(|_| {
            crate::Error::Reported(format!("[generic] '{}' is not a valid URL", &found["url"]))
        })?;
        let mut info = super::direct::build_at(target.clone(), &target, String::new());
        info.formats.clear();
        info.direct = None;
        info.extra
            .insert(crate::FOLLOW_URL.into(), target.as_str().into());
        Ok(Some(ExtractResult::Video(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://href.li/?https://www.reddit.com/r/cats/comments/12bluel/my_cat_helps_me_with_water/?utm_source=share"
        ));
        assert!(!suitable("https://href.li/"));
        assert!(!suitable("https://www.href.li/?https://example.com/"));
    }
}
