//! Upstream `XXXYMoviesIE` (`yt_dlp/extractor/xxxymovies.py`): the page's
//! `video_url` (a direct MP4, so no formats), the title, the preview
//! thumbnail, the keyword categories, the duration and the view, like and
//! dislike counts, plus the RTA age rating.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct XXXYMovies;

const NAME: &str = "XXXYMovies";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?xxxymovies\.com/videos/(?P<id>\d+)/(?P<display_id>[^/]+)",
    )
    .unwrap()
});
static VIDEO_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"video_url\s*:\s*'([^']+)'").unwrap());
static TITLES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<div[^>]+\bclass="block_header"[^>]*>\s*<h1>([^<]+)<"#).unwrap(),
        Regex::new(r"<title>(.*?)\s*-\s*(?:XXXYMovies\.com|XXX\s+Movies)</title>").unwrap(),
    ]
});
static PREVIEW_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"preview_url\s*:\s*'([^']+)'").unwrap());
static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<span>Duration:</span>\s*(\d+:\d+)").unwrap());
static VIEWS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div class="video_views">\s*(\d+)"#).unwrap());
static LIKES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r">\s*Likes? <b>\((\d+)\)").unwrap());
static DISLIKES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r">\s*Dislike <b>\((\d+)\)</b>").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none` of a captured count.
fn count(text: Option<regex::Match<'_>>) -> Value {
    text.and_then(|found| found.as_str().parse::<i64>().ok())
        .map_or(Value::Null, Value::from)
}

/// The single format of a top-level `url` result: the info dict itself
/// (`process_video_result`), marked as such.
fn url_only_format(url: &str, ext: &str) -> Result<crate::VideoFormat> {
    let mut fields = serde_json::Map::new();
    fields.insert("format_id".into(), "".into());
    fields.insert("url".into(), url.into());
    fields.insert("ext".into(), ext.into());
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(fields))
        .map_err(|error| Error::Reported(format!("[{NAME}] {error}")))?;
    crate::info_order::mark_format(&mut format, "url-only");
    Ok(format)
}

impl Extractor for XXXYMovies {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;

        let Some(video_url) = VIDEO_URL.captures(&webpage) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract video URL; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let video_url = video_url[1].to_owned();

        let title = TITLES
            .iter()
            .find_map(|pattern| pattern.captures(&webpage))
            .map(|captures| crate::html::clean_html(&captures[1]))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract title; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let thumbnail = PREVIEW_URL
            .captures(&webpage)
            .map(|captures| captures[1].to_owned());
        let categories: Vec<Value> = crate::html::html_search_meta(&webpage, "keywords")
            .unwrap_or_default()
            .split(',')
            .map(Value::from)
            .collect();
        let duration = DURATION
            .captures(&webpage)
            .and_then(|captures| crate::parse_duration(&captures[1]))
            .map_or(Value::Null, |duration| match duration {
                crate::info::PyNumber::Int(value) => Value::from(value),
                crate::info::PyNumber::Float(value) => Value::from(value),
            });
        let view_count = count(VIEWS.captures(&webpage).and_then(|found| found.get(1)));
        let like_count = count(LIKES.captures(&webpage).and_then(|found| found.get(1)));
        let dislike_count = count(DISLIKES.captures(&webpage).and_then(|found| found.get(1)));
        let age_limit = crate::html::rta_search(&webpage);

        let fields = vec![
            ("id", video_id.into()),
            ("display_id", display_id.into()),
            ("url", video_url.as_str().into()),
            ("title", title.into()),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            ("categories", Value::Array(categories)),
            ("duration", duration),
            ("view_count", view_count),
            ("like_count", like_count),
            ("dislike_count", dislike_count),
            ("age_limit", age_limit.map_or(Value::Null, Value::from)),
        ];
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let format = url_only_format(&video_url, "mp4")?;
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
        assert!(suitable(
            "http://xxxymovies.com/videos/138669/ecstatic-orgasm-sofcore/"
        ));
        assert!(suitable("https://www.xxxymovies.com/videos/1/x"));
        assert!(!suitable("http://xxxymovies.com/videos/138669/"));
    }
}
