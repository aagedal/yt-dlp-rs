//! Upstream `FiveTVIE` (`yt_dlp/extractor/fivetv.py`): 5-tv.ru videos from
//! the page's player `data-href` (or `videoplayer` link), with the generic
//! title and the OpenGraph description, image and duration.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct FiveTv;

const NAME: &str = "FiveTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?5-tv\.ru/(?:(?:[^/]+/)+(?P<id>\d+)|(?P<path>[^/?#]+)(?:[/?#])?)",
    )
    .unwrap()
});
static VIDEO_URL: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<div[^>]+?class="(?:flow)?player[^>]+?data-href="([^"]+)""#).unwrap(),
        Regex::new(r#"<a[^>]+?href="([^"]+)"[^>]+?class="videoplayer""#).unwrap(),
    ]
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_og_search_property(prop, webpage, default=None)`.
fn og(webpage: &str, prop: &str) -> Value {
    crate::html::og_search_property(webpage, prop).map_or(Value::Null, Value::from)
}

impl Extractor for FiveTv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found
            .name("id")
            .or_else(|| found.name("path"))
            .map_or("", |id| id.as_str())
            .to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let video_url = VIDEO_URL
            .iter()
            .find_map(|regex| regex.captures(&webpage))
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                // `get_temp_id`: only the `id` group, so `path` pages have
                // no `ID: ` prefix.
                let id = found
                    .name("id")
                    .map_or_else(String::new, |id| format!("{}: ", id.as_str()));
                Error::Reported(format!(
                    "[{NAME}] {id}Unable to extract video url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // `_generic_title('', webpage)`: `og:title`, else `<title>`, else
        // the empty URL's basename (`None`).
        let title = crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
            .or_else(|| crate::html::html_extract_title(&webpage).filter(|title| !title.is_empty()))
            .map_or(Value::Null, Value::from);
        let duration = crate::html::og_search_property(&webpage, "video:duration")
            .and_then(|duration| duration.trim().parse::<i64>().ok())
            .map_or(Value::Null, |duration| json!(duration));
        let ext = super::direct::determine_ext(&video_url)
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        let mut format: crate::VideoFormat =
            serde_json::from_value(json!({"format_id": "", "url": video_url, "ext": ext}))
                .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "url-only");
        let fields = vec![
            ("id", Value::from(video_id)),
            ("url", video_url.into()),
            ("title", title),
            ("description", og(&webpage, "description")),
            ("thumbnail", og(&webpage, "image")),
            ("duration", duration),
        ];
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("untyped-dict;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(
            dict_info(url, (NAME, NAME), fields, vec![format]),
            &shape,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "http://5-tv.ru/news/96814/",
            "http://www.5-tv.ru/glavnoe/#itemDetails",
            "http://www.5-tv.ru/glavnoe/broadcasts/508645/",
            "http://5-tv.ru/angel/",
            "http://www.5-tv.ru/schedule/?iframe=true&width=900&height=450",
        ] {
            assert!(suitable(url), "{url}");
        }
        assert!(!suitable("http://5-tv.ru/"));
        let found = VALID_URL.captures("http://5-tv.ru/news/96814/").unwrap();
        assert_eq!(&found["id"], "96814");
        let found = VALID_URL.captures("http://5-tv.ru/angel/").unwrap();
        assert!(found.name("id").is_none());
        assert_eq!(&found["path"], "angel");
    }
}
