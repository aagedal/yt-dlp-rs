//! Upstream `GaskrankIE` (`yt_dlp/extractor/gaskrank.py`): Gaskrank.tv
//! videos from the page's first HTML5 media entry, with its title, category,
//! uploader, date, homepage, tags, views and rating.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Gaskrank;

const NAME: &str = "Gaskrank";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?gaskrank\.tv/tv/(?P<categories>[^/]+)/(?P<id>[^/]+)\.htm",
    )
    .unwrap()
});
static UPLOADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"Video von:\s*(?P<uploader_id>[^|]*?)\s*\|\s*vom:\s*(?P<upload_date>[0-9][0-9]\.[0-9][0-9]\.[0-9][0-9][0-9][0-9])",
    )
    .unwrap()
});
static HOMEPAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Homepage:\s*<[^>]*>(?P<uploader_url>[^<]*)").unwrap());
static TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"/tv/tags/[^/]+/"\s*>(?P<tag>[^<]*?)<"#).unwrap());
static VIEWS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"class\s*=\s*"gkRight"(?:[^>]*>\s*<[^>]*)*icon-eye-open(?:[^>]*>\s*<[^>]*)*>\s*(?P<view_count>[0-9\.]*)"#,
    )
    .unwrap()
});
static RATING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"itemprop\s*=\s*"ratingValue"[^>]*>\s*(?P<average_rating>[0-9,]+)"#).unwrap()
});
static VIDEO_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://movies\.gaskrank\.tv/([^-]*?)(-[^\.]*)?\.mp4").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Gaskrank {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        let category = found["categories"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let fail = |name: &str| {
            Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let title = crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
            .or_else(|| crate::html::html_search_meta(&webpage, "title"))
            .ok_or_else(|| fail("title"))?;
        let uploader = UPLOADER.captures(&webpage);
        let uploader_url = HOMEPAGE
            .captures(&webpage)
            .map(|found| found["uploader_url"].to_owned());
        let tags: Vec<Value> = TAGS
            .captures_iter(&webpage)
            .map(|found| Value::from(found["tag"].to_owned()))
            .collect();
        let view_count = VIEWS
            .captures(&webpage)
            .map(|found| found["view_count"].to_owned())
            .filter(|views| !views.is_empty())
            .map(|views| {
                views
                    .replace('.', "")
                    .parse::<i64>()
                    .map_or(Value::Null, Value::from)
            });
        let rating = RATING
            .captures(&webpage)
            .map(|found| found["average_rating"].to_owned())
            .ok_or_else(|| fail("average_rating"))?;
        let average_rating = rating
            .replace(',', ".")
            .parse::<f64>()
            .map_or(Value::Null, |rating| json!(rating));
        let video_id = VIDEO_ID
            .captures(&webpage)
            .map_or_else(|| display_id.clone(), |found| found[1].to_owned());
        let entry = super::html5::parse_html5_media_entries(
            client, url, &webpage, &video_id, NAME, None, None,
        )?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        // Without the uploader line, `entry.update` fails on unbound locals.
        let Some(uploader) = uploader else {
            return Err(Error::Reported(
                "cannot access local variable 'uploader_id' where it is not associated with a value"
                    .into(),
            ));
        };
        let uploader_id = uploader["uploader_id"].to_owned();
        let upload_date = crate::dates::unified_strdate(&uploader["upload_date"], true);
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let mut fields = vec![
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("thumbnail", optional(entry.thumbnail.clone())),
        ];
        fields.extend([
            ("id", video_id.into()),
            ("title", title.into()),
            ("categories", json!([category])),
            ("display_id", display_id.into()),
            ("uploader_id", uploader_id.into()),
            ("upload_date", optional(upload_date)),
            ("uploader_url", optional(uploader_url)),
            ("tags", Value::Array(tags)),
            ("view_count", view_count.unwrap_or(Value::Null)),
            ("average_rating", average_rating),
        ]);
        let mut info = dict_info(url, (NAME, NAME), fields, entry.formats);
        info.subtitles = entry.subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.gaskrank.tv/tv/motorrad-fun/strike-einparken-durch-anfaenger-crash-mit-groesserem-flurschaden.htm"
        ));
        assert!(!suitable("http://www.gaskrank.tv/tv/"));
    }
}
