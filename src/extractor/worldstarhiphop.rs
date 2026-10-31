//! Upstream `WorldStarHipHopIE` (`yt_dlp/extractor/worldstarhiphop.py`):
//! the first HTML5 media entry with the page heading as its title.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct WorldStarHipHop;

const NAME: &str = "WorldStarHipHop";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www|m)\.worldstar(?:candy|hiphop)\.com/(?:videos|android)/video\.php\?.*?\bv=(?P<id>[^&]+)",
    )
    .unwrap()
});
static TITLES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"(?s)<div class="content-heading">\s*<h1>(.*?)</h1>"#).unwrap(),
        Regex::new(r#"<span[^>]+class="tc-sp-pinned-title">(.*)</span>"#).unwrap(),
    ]
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for WorldStarHipHop {
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
        let Some(entry) =
            super::html5::parse_html5_media_entries(client, url, &webpage, &id, NAME, None, None)?
                .into_iter()
                .next()
        else {
            // `url_result(url, 'Generic')`.
            return Err(Error::UnsupportedHtml5(
                "WorldStarHipHop pages without HTML5 media (a Generic url result) are not supported natively"
                    .into(),
            ));
        };
        let title = TITLES
            .iter()
            .find_map(|pattern| {
                pattern
                    .captures(&webpage)
                    .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            })
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract title; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
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
        info.subtitles = entry.subtitles;
        info.metadata.thumbnail = entry.thumbnail;
        info.direct = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;formats,subtitles,thumbnail,id,title",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.worldstarhiphop.com/videos/video.php?v=wshh6a7q1ny0G34ZwuIO"
        ));
        assert!(suitable(
            "http://m.worldstarhiphop.com/android/video.php?v=wshh6a7q1ny0G34ZwuIO"
        ));
        assert!(!suitable("http://www.worldstarhiphop.com/videos/"));
    }
}
