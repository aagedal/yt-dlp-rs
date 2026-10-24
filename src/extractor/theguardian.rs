//! Upstream `TheGuardianPodcastIE` (`yt_dlp/extractor/theguardian.py`): an
//! episode page's `podcast__player` `data-source` with its OpenGraph and
//! meta-tag metadata (the series playlist extractor is not ported).
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct TheGuardianPodcast;

const NAME: &str = "TheGuardianPodcast";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?theguardian\.com/\w+/audio/\d{4}/\w{3}/\d{1,2}/(?P<id>[\w-]+)")
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for TheGuardianPodcast {
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
        let webpage = download_webpage(client, url, NAME, &id)?;
        let og = |property: &str, name: &str| {
            let value = crate::html::og_search_property(&webpage, property);
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let meta = |name: &str| {
            let value = crate::html::html_search_meta(&webpage, name);
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let by_class = |class: &str| {
            crate::html_elements::elements_text_and_html_by_class(class, &webpage)
                .ok()
                .and_then(|found| found.into_iter().next())
        };
        // `_og_search_title(webpage) or get_element_by_class('content__headline', webpage)`.
        let title = og("title", "OpenGraph title")
            .filter(|title| !title.is_empty())
            .or_else(|| by_class("content__headline").map(|(text, _)| text));
        let description = og("description", "OpenGraph description");
        let creator = meta("author");
        let thumbnail = og("image", "thumbnail URL");
        let release_date = meta("article:published_time")
            .and_then(|date| crate::dates::unified_strdate(&date, true));
        // `extract_attributes(get_element_html_by_class('podcast__player',
        // webpage) or '').get('data-source')`.
        let source = by_class("podcast__player")
            .map(|(_, element)| crate::html_elements::extract_attributes(&element))
            .and_then(|attributes| match attributes.get("data-source") {
                Some(Value::String(source)) => Some(source.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                Error::UnsupportedHtml5("TheGuardianPodcast pages without a player source".into())
            })?;
        let media = Url::parse(&source).map_err(|_| {
            Error::UnsupportedHtml5("TheGuardianPodcast player sources that are not URLs".into())
        })?;
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.id = id;
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = description;
        info.metadata.thumbnail = thumbnail;
        for (name, value) in [("creator", creator), ("release_date", release_date)] {
            match value {
                Some(value) => crate::metadata_parser::set_field(&mut info, name, value.into()),
                None => {
                    info.extra.insert(name.into(), Value::Null);
                }
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            "theguardianpodcast",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.theguardian.com/news/audio/2023/nov/03/we-are-just-getting-started-podcast"
        ));
        assert!(suitable(
            "https://www.theguardian.com/football/audio/2023/nov/06/arsenal-football-weekly"
        ));
        assert!(!suitable(
            "https://www.theguardian.com/news/series/todayinfocus"
        ));
    }
}
