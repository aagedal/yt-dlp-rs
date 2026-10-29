//! Upstream `ThisOldHouseIE` (`yt_dlp/extractor/thisoldhouse.py`): a page's
//! Zype or Brightcove embed iframe, each resolved with a HEAD request and
//! followed as a `url` result (Brightcove with the page as the smuggled
//! `referrer`). Subscriber-only pages raise login-required errors; the
//! account login (`--username`) is not supported natively.
use super::{Extractor, direct, follow_url, proto_relative};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::json;
use std::sync::LazyLock;
use url::Url;

pub struct ThisOldHouse;

const NAME: &str = "ThisOldHouse";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?thisoldhouse\.com/(?:(?:watch|how-to|tv-episode|(?:[^/?#]+/)?\d+)/(?P<id>[^/?#]+)|ask-this-old-house/(?P<ask_id>season-\d+/episode-\d+))",
    )
    .unwrap()
});
static ZYPE_IFRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+src=['"]((?:https?:)?//(?:www\.)?thisoldhouse\.(?:chorus\.build|com)/videos/zype/([0-9a-f]{24})[^'"]*)['"]"#,
    )
    .unwrap()
});
static BRIGHTCOVE_IFRAMES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(
            r#"<iframe[^>]+src=['"]((?:https?:)?//players\.brightcove\.net/\d+/\w+/index\.html\?videoId=(\d+))"#,
        )
        .unwrap(),
        Regex::new(
            r#"<iframe[^>]+src=['"]((?:https?:)?//(?:www\.)thisoldhouse\.com/videos/brightcove/(\d+))"#,
        )
        .unwrap(),
    ]
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_request_webpage(HEADRequest(url), id, note=...)`: the final URL.
fn resolve(client: &Client, url: &str, id: &str, note: &str) -> Result<Url> {
    crate::screen::screen(&format!("[{NAME}] {id}: {note}"));
    let target = Url::parse(&proto_relative(url))?;
    direct::head_with_headers(client, &target, &[])
        .map(|response| response.url().clone())
        .map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: NAME,
                id: id.to_owned(),
                what: "webpage",
                source: Box::new(error),
            },
            error => error,
        })
}

impl Extractor for ThisOldHouse {
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
        let display_id = found
            .name("id")
            .or_else(|| found.name("ask_id"))
            .map_or(String::new(), |id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let (webpage, page_url) = super::download_webpage_handle(client, url, NAME, &display_id)?;
        // "If login response says inactive subscription, site redirects to
        // frontpage for Insider content".
        if webpage.contains("To Unlock This content") || matches!(page_url.path(), "" | "/") {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: {}",
                super::login::login_required_message(
                    "This video is only available for subscribers",
                    Some("thisoldhouse"),
                    None,
                )
            )));
        }

        if let Some(found) = ZYPE_IFRAME.captures(&webpage) {
            let video_url = &found[1];
            let video_id = &found[2];
            let resolved = resolve(client, video_url, video_id, "Resolving Zype URL")?;
            return Ok(Some(follow_url(url, resolved.to_string())));
        }

        let Some((video_url, video_id)) = BRIGHTCOVE_IFRAMES.iter().find_map(|regex| {
            regex
                .captures(&webpage)
                .map(|found| (found[1].to_owned(), found[2].to_owned()))
        }) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract iframe url; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let has_video_id = Url::parse(&proto_relative(&video_url))
            .ok()
            .is_some_and(|parsed| parsed.query_pairs().any(|(key, _)| key == "videoId"));
        let target = if has_video_id {
            proto_relative(&video_url)
        } else {
            resolve(client, &video_url, &video_id, "Resolving Brightcove URL")?.to_string()
        };
        let smuggled = super::brightcove::smuggle(&target, &json!({ "referrer": url.as_str() }));
        Ok(Some(follow_url(url, smuggled)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.thisoldhouse.com/furniture/21017078/how-to-build-a-storage-bench"
        ));
        assert!(suitable(
            "https://www.thisoldhouse.com/glen-ridge-generational/99537/s45-e17-multi-generational"
        ));
        assert!(suitable(
            "https://www.thisoldhouse.com/ask-this-old-house/season-24/episode-17"
        ));
        assert!(suitable(
            "https://www.thisoldhouse.com/watch/arlington-arts-crafts-arts-and-crafts-class-begins"
        ));
        assert!(suitable(
            "https://www.thisoldhouse.com/21113884/s41-e13-paradise-lost"
        ));
        assert!(!suitable("https://www.thisoldhouse.com/"));
    }

    #[test]
    fn ask_urls_capture_season_and_episode() {
        let found = VALID_URL
            .captures("https://www.thisoldhouse.com/ask-this-old-house/season-24/episode-17")
            .unwrap();
        assert_eq!(
            found.name("ask_id").unwrap().as_str(),
            "season-24/episode-17"
        );
        assert!(found.name("id").is_none());
    }
}
