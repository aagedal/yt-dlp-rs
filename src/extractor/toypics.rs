//! Upstream `ToypicsIE` (`yt_dlp/extractor/toypics.py`, marked broken): the
//! first HTML5 media entry's formats, the heading and the uploader.
//! `ToypicsUserIE` (profile playlists) is not ported.
use super::{Extractor, broken_warning, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Toypics;

const NAME: &str = "Toypics";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://videos\.toypics\.net/view/(?P<id>[0-9]+)").unwrap()
});
static TITLES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<h1[^>]+class=["']view-video-title[^>]+>([^<]+)</h"#).unwrap(),
        Regex::new(r"<title>([^<]+) - Toypics</title>").unwrap(),
    ]
});
static UPLOADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"More videos from <strong>([^<]+)</strong>").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_html_search_regex`: the first pattern's cleaned group.
fn html_search(patterns: &[Regex], html: &str) -> Option<String> {
    patterns.iter().find_map(|pattern| {
        pattern
            .captures(html)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
    })
}

impl Extractor for Toypics {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        broken_warning();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        // `_parse_html5_media_entries(...)[0]['formats']`.
        let entry =
            super::html5::parse_html5_media_entries(client, url, &webpage, &id, NAME, None, None)?
                .into_iter()
                .next()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let title = html_search(&*TITLES, &webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract title; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let uploader = html_search(std::slice::from_ref(&*UPLOADER), &webpage);
        if uploader.is_none() {
            super::search_warning(NAME, "uploader");
        }
        let first = entry
            .formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        if entry.formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.formats = entry.formats;
        info.direct = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        match uploader {
            Some(uploader) => {
                crate::metadata_parser::set_field(&mut info, "uploader", Value::String(uploader))
            }
            None => {
                info.extra.insert("uploader".into(), Value::Null);
            }
        }
        crate::metadata_parser::set_field(&mut info, "age_limit", Value::from(18));
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,formats,title,uploader,age_limit",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://videos.toypics.net/view/514/chancebulged,-2-1/"
        ));
        assert!(!suitable("http://videos.toypics.net/Mikey"));
    }
}
