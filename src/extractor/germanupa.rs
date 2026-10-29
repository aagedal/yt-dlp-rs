//! Upstream `GermanupaIE` (`yt_dlp/extractor/germanupa.py`): a Mediathek
//! page's oEmbed iframe, followed as a Vimeo video (its page as the
//! `referer`); members-only pages raise the login error.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::json;
use std::sync::LazyLock;
use url::Url;

pub struct Germanupa;

const NAME: &str = "Germanupa";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://germanupa\.de/mediathek/(?P<id>[\w-]+)").unwrap());
// The upstream `\1` back-reference, as one alternative per quote style.
static OEMBED_IFRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"<iframe[^>]+data-src\s*=\s*(?:"(https://germanupa\.de/media/oembed\?url=[^"\n]+)"|"#,
        r#"'(https://germanupa\.de/media/oembed\?url=[^'\n]+)')"#,
    ))
    .unwrap()
});
static LOGIN_WRAPPER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"<div[^>]+class\s*=\s*(?:"[^"\n]*login-wrapper[^"\n]*"|"#,
        r#"'[^'\n]*login-wrapper[^'\n]*')"#,
    ))
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Germanupa {
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
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `parse_qs` of the iframe's `data-src`, its first `url`, then `url_or_none`.
        let param_url = OEMBED_IFRAME
            .captures(&webpage)
            .and_then(|found| found.get(1).or_else(|| found.get(2)))
            .and_then(|raw| Url::parse(raw.as_str()).ok())
            .and_then(|oembed| {
                oembed
                    .query_pairs()
                    .find(|(key, _)| key == "url")
                    .map(|(_, value)| value.into_owned())
            })
            .and_then(|target| super::json_ld::url_or_none(&target));

        let Some(param_url) = param_url else {
            if LOGIN_WRAPPER.is_match(&webpage) {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: {}",
                    super::login::login_required_message(
                        "This video is only available for members",
                        None,
                        None
                    )
                )));
            }
            // Upstream hands audio pages to GenericIE; without media on the
            // page it raises its `UnsupportedError` (the native generic fallback is not used here).
            return Err(Error::UnsupportedUrl(url.to_string()));
        };
        let real_url = param_url.replace("https://vimeo.com/", "https://player.vimeo.com/video/");
        // `VimeoIE._smuggle_referrer(real_url, url)`.
        let target = super::brightcove::smuggle(&real_url, &json!({"referer": url.as_str()}));
        Ok(Some(super::follow_url(url, target)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://germanupa.de/mediathek/4-figma-beratung-deine-sprechstunde-fuer-figma-fragen"
        ));
        assert!(!suitable("https://germanupa.de/"));
    }
}
