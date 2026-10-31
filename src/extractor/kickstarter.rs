//! Upstream `KickStarterIE` (`yt_dlp/extractor/kickstarter.py`): a project
//! page's native `data-video-url` as an untyped result. Pages without one,
//! which upstream hands to the generic extractor, are an explicit error.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct KickStarter;

const NAME: &str = "KickStarter";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?kickstarter\.com/projects/(?P<id>[^/]*)/.*")
        .unwrap()
});
static TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<title>\s*(.*?)(?:\s*&mdash;\s*Kickstarter)?\s*</title>").unwrap()
});
static VIDEO_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-video-url="(.*?)""#).unwrap());
static POSTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<img[^>]+class="[^"]+\s*poster\s*[^"]+"[^>]+src="([^"]+)""#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for KickStarter {
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
        // `_html_search_regex` (fatal).
        let title = TITLE
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract title; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let Some(video_url) = VIDEO_URL
            .captures(&webpage)
            .map(|found| found[1].to_owned())
        else {
            return Err(Error::UnsupportedHtml5(
                "KickStarter pages without a native video (upstream's generic embed fallback) are not supported natively".into(),
            ));
        };
        let media = Url::parse(&video_url).map_err(|_| {
            Error::UnsupportedHtml5(format!(
                "KickStarter video URLs that are not absolute are not supported natively: {video_url}"
            ))
        })?;
        let thumbnail = crate::html::og_search_property(&webpage, "image").or_else(|| {
            let found = POSTER
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]).trim().to_owned());
            if found.is_none() {
                search_warning(NAME, "thumbnail image");
            }
            found
        });
        let description = crate::html::og_search_property(&webpage, "description");
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
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        for (name, value) in [("description", description), ("thumbnail", thumbnail)] {
            match value {
                Some(value) => crate::metadata_parser::set_field(&mut info, name, value.into()),
                None => {
                    info.extra.insert(name.into(), Value::Null);
                }
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            "untyped-dict;id,url,title,description,thumbnail",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.kickstarter.com/projects/1404461844/intersection-the-story-of-josh-grant/description"
        ));
        assert!(suitable(
            "https://www.kickstarter.com/projects/1420158244/power-drive-2000/widget/video.html"
        ));
        assert!(!suitable("https://www.kickstarter.com/projects/1404461844"));
    }
}
