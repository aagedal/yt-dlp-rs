//! Upstream `Mir24TvIE` (`yt_dlp/extractor/mir24tv.py`): a news page's
//! player iframe `source` query parameter as fatal HLS formats.
use super::{Extractor, download_webpage, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Mir24Tv;

const NAME: &str = "mir24.tv";
const KEY: &str = "Mir24Tv";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?mir24\.tv/news/(?P<id>[0-9]+)/[^/?#]+").unwrap()
});
static IFRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<iframe\b[^>]+\bsrc=["'](https?://mir24\.tv/players/[^"']+)"#).unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn bug(id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `parse_qs(url)['source']`: the query's non-blank `source` values.
fn source_values(url: &str) -> Vec<String> {
    let query = url
        .split_once('?')
        .map_or("", |(_, rest)| rest.split('#').next().unwrap_or_default());
    url::form_urlencoded::parse(query.as_bytes())
        .filter(|(name, value)| name == "source" && !value.is_empty())
        .map(|(_, value)| value.into_owned())
        .collect()
}

impl Extractor for Mir24Tv {
    fn key(&self) -> &'static str {
        KEY
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
        // `impersonate=True` without an available target.
        crate::screen::warning_once(&format!(
            "[{NAME}] The extractor is attempting impersonation, but no impersonate target is available. If you encounter errors, then see  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies"
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        let iframe = IFRAME
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| bug(&id, "Unable to extract iframe URL"))?;
        // `('source', -1, {self._proto_relative_url}, {url_or_none},
        // {require('m3u8 URL')})`.
        let manifest = source_values(&iframe)
            .pop()
            .map(|source| super::proto_relative(&source))
            .and_then(|source| super::json_ld::url_or_none(&source))
            .ok_or_else(|| bug(&id, "Unable to extract m3u8 URL"))?;
        let manifest = Url::parse(&manifest)?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, Some("hls"), true)?;
        let title = crate::html::og_search_property(&webpage, "title").or_else(|| {
            let title = crate::html::html_extract_title(&webpage);
            if title.is_none() {
                super::search_warning(NAME, "title");
            }
            title
        });
        info.id = id;
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.metadata.thumbnail = crate::html::og_search_property(&webpage, "image");
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,thumbnail,formats,subtitles",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://mir24.tv/news/16635210/dni-kultury-rossii-otkrylis-v-uzbekistane"
        ));
        assert!(!suitable("https://mir24.tv/news/16635210/"));
    }

    #[test]
    fn sources_follow_parse_qs() {
        assert_eq!(
            source_values("https://mir24.tv/players/a?source=&source=%2F%2Fx%2Fa.m3u8&b=1#f"),
            ["//x/a.m3u8"]
        );
        assert!(source_values("https://mir24.tv/players/a").is_empty());
    }
}
