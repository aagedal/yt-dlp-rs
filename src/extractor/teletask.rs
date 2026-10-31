//! Upstream `TeleTaskIE` (`yt_dlp/extractor/teletask.py`, marked broken): a
//! lecture page's `<video class="KIND"><source src>` pairs as a playlist of
//! untyped entries with the lecture's title and date.
use super::{Extractor, broken_warning, download_webpage, search_warning};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct TeleTask;

const NAME: &str = "TeleTask";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?tele-task\.de/archive/video/html5/(?P<id>[0-9]+)",
    )
    .unwrap()
});
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"itemprop="name">([^<]+)</a>"#).unwrap());
static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Date:</td><td>([^<]+)</td>").unwrap());
static VIDEO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<video class="([^"]+)"[^>]*>\s*<source src="([^"]+)""#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for TeleTask {
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
        let lecture_id = found["id"].to_owned();
        broken_warning();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &lecture_id)?;
        // `_html_search_regex(..., 'title')` (fatal).
        let title = TITLE
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {lecture_id}: Unable to extract title; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // `unified_strdate(_html_search_regex(..., 'date', fatal=False))`.
        let date = DATE
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned());
        if date.is_none() {
            search_warning(NAME, "date");
        }
        let upload_date = date.and_then(|date| crate::dates::unified_strdate(&date, true));
        let mut entries = Vec::new();
        for found in VIDEO.captures_iter(&webpage) {
            let media = Url::parse(&found[2])?;
            let ext = super::direct::determine_ext(media.as_str())
                .unwrap_or_else(|| "unknown_video".into());
            let mut info = super::direct::with_format(
                super::direct::build_at(media.clone(), &media, ext.clone()),
                "",
            );
            for format in &mut info.formats {
                format.ext = ext.clone();
                format.format_id = String::new();
            }
            info.format_id = None;
            info.id = format!("{lecture_id}-{}", &found[1]);
            info.title = title.clone();
            info.webpage_url = url.clone();
            info.direct = None;
            info.extractor = NAME.into();
            info.extractor_key = NAME.into();
            match &upload_date {
                Some(date) => info.metadata.upload_date = Some(date.clone()),
                None => {
                    info.extra.insert("upload_date".into(), Value::Null);
                }
            }
            entries.push(crate::info_order::mark_info(
                info,
                "untyped-dict;id,url,title,upload_date",
            ));
        }
        // `playlist_result(entries, lecture_id, title)`.
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: lecture_id,
            title,
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.tele-task.de/archive/video/html5/26168/"
        ));
        assert!(!suitable(
            "http://www.tele-task.de/archive/video/flash/26168/"
        ));
    }
}
