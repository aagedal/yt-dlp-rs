//! Upstream `JWPlatformIE` (`yt_dlp/extractor/jwplatform.py`): the
//! `cdn.jwplayer.com/v2/media/ID` JSON through `_parse_jwplayer_data`
//! (`require_title=True`).
use super::{Extractor, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct JwPlatform;

const NAME: &str = "JWPlatform";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:https?://(?:content\.jwplatform|cdn\.jwplayer)\.com/(?:(?:feed|player|thumb|preview|manifest)s|jw6|v2/media)/|jwplatform:)(?P<id>[a-zA-Z0-9]{8})",
    )
    .unwrap()
});

/// `_match_id` (also for `jwplatform:ID`).
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for JwPlatform {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!("https://cdn.jwplayer.com/v2/media/{id}"))?;
        let data = download_json(client, &api, NAME, &id)?;
        let no_formats = || {
            Error::Reported(format!(
                "[{NAME}] {id}: No video formats found!; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let Some(entry) = super::jwplayer::parse_required(&data, &id, &api)? else {
            return Err(no_formats());
        };
        let title_missing = entry.get("title").is_none_or(Value::is_null);
        let Some((mut info, keys)) =
            super::html5::jwplayer_entry_info(client, entry, url, &id, NAME)?
        else {
            return Err(no_formats());
        };
        if title_missing {
            info.title = String::new();
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let shape = format!("jwplatform;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://content.jwplatform.com/players/nPripu9l-ALJ3XQCI.js"
        ));
        assert!(suitable("https://cdn.jwplayer.com/v2/media/nPripu9l"));
        assert!(suitable("jwplatform:nPripu9l"));
        assert!(!suitable("https://cdn.jwplayer.com/players/short"));
    }
}
