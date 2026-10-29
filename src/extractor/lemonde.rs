//! Upstream `LemondeIE` (`yt_dlp/extractor/lemonde.py`): a Le Monde article's
//! Digiteka/Ultimedia player (its `url` player config) as a `Digiteka`
//! result. Pages without one fall back to upstream's `Generic` extractor,
//! which is this port's generic route (`Ok(None)` lets it run).
use super::{Extractor, download_webpage, follow_url, proto_relative};
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct Lemonde;

const NAME: &str = "Lemonde";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:.+?\.)?lemonde\.fr/(?:[^/]+/)*(?P<id>[^/]+)\.html").unwrap()
});
// `url\s*:\s*(["'])(?P<url>...)\1` split into one alternative per quote.
static DIGITEKA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"url\s*:\s*(?:"(?P<dq>(?:https?://)?//(?:www\.)?(?:digiteka\.net|ultimedia\.com)/deliver/[^"\n]+?)"|'(?P<sq>(?:https?://)?//(?:www\.)?(?:digiteka\.net|ultimedia\.com)/deliver/[^'\n]+?)')"#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Lemonde {
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
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let digiteka = DIGITEKA
            .captures(&webpage)
            .and_then(|found| found.name("dq").or_else(|| found.name("sq")))
            .map(|found| proto_relative(found.as_str()));
        match digiteka {
            Some(target) => Ok(Some(follow_url(url, target))),
            // `url_result(url, 'Generic')`: the generic route runs on the page.
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.lemonde.fr/police-justice/video/2016/01/19/comprendre-l-affaire-bygmalion-en-cinq-minutes_4849702_1653578.html"
        ));
        assert!(!suitable("https://www.lemonde.fr/"));
    }
}
