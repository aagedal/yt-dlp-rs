//! Upstream `VidLiiIE` (`yt_dlp/extractor/vidlii.py`): VidLii watch and
//! embed pages: the player's `src` files that answer a HEAD check, and the
//! page's title, description, thumbnail, uploader, date, duration, counts,
//! rating, category and tags (each optional field warning like upstream).
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct VidLii;

const NAME: &str = "VidLii";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?vidlii\.com/(?:watch|embed)\?.*?\bv=(?P<id>[0-9A-Za-z_-]{11})",
    )
    .unwrap()
});
static SOURCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"src\s*:\s*(?:"(?P<dq>(?:https?://)?[^"\n]+)"|'(?P<sq>(?:https?://)?[^'\n]+)')"#)
        .unwrap()
});
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+).mp4").unwrap());
static TITLES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"<h1>([^<]+)</h1>").unwrap(),
        Regex::new(r"<title>([^<]+) - VidLii<").unwrap(),
    ]
});
static IMG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"img\s*:\s*(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')"#).unwrap());
static UPLOADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div[^>]+class=["']wt_person[^>]+>\s*<a[^>]+\bhref=["']/user/[^>]+>([^<]+)"#)
        .unwrap()
});
static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<date>([^<]+)").unwrap());
static DURATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"duration\s*:\s*(\d+)").unwrap());
static VIEWS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"<strong>([,0-9]+)</strong> views").unwrap(),
        Regex::new(r"Views\s*:\s*<strong>([,0-9]+)</strong>").unwrap(),
    ]
});
static COMMENTS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<span[^>]+id=["']cmt_num[^>]+>(\d+)"#).unwrap(),
        Regex::new(r"Comments\s*:\s*<strong>(\d+)").unwrap(),
    ]
});
static RATING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"rating\s*:\s*([\d.]+)").unwrap());
static CATEGORY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<div>Category\s*:\s*</div>\s*<div>\s*<a[^>]+>([^<]+)").unwrap());
static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+\bhref=["']/results\?.*?q=[^>]*>([^<]+)"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_regex(..., fatal=False)`: the first pattern's group, else a
/// `unable to extract NAME` warning.
fn search_optional(webpage: &str, patterns: &[&Regex], name: &str) -> Option<String> {
    let found = patterns
        .iter()
        .find_map(|pattern| pattern.captures(webpage).map(|found| found[1].to_owned()));
    if found.is_none() {
        crate::screen::warning(&format!(
            "[{NAME}] unable to extract {name}; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    found
}

fn meta(webpage: &str, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name))
        .filter(|value| !value.is_empty())
}

fn quoted<'a>(found: &regex::Captures<'a>) -> Option<&'a str> {
    found
        .name("dq")
        .or_else(|| found.name("sq"))
        .map(|value| value.as_str())
}

impl Extractor for VidLii {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = Url::parse(&format!("https://www.vidlii.com/watch?v={video_id}"))?;
        let webpage = download_webpage(client, &page, NAME, &video_id)?;
        let mut formats = Vec::new();
        for found in SOURCE.captures_iter(&webpage) {
            let Some(source) = quoted(&found)
                .and_then(|source| super::videopress::urljoin(Some(url.as_str()), source))
            else {
                continue;
            };
            let height: i64 = HEIGHT
                .captures(&source)
                .and_then(|found| found[1].parse().ok())
                .unwrap_or(360);
            // `_request_webpage(HEADRequest(source), ..., errnote=False)`.
            crate::screen::screen(&format!("[{NAME}] {video_id}: Checking {height}p url"));
            let reachable = Url::parse(&source).ok().is_some_and(|target| {
                super::direct::head_with_headers(client, &target, &[]).is_ok()
            });
            if reachable {
                formats.push(dict_format(
                    vec![
                        ("url", source.into()),
                        ("format_id", format!("{height}p").into()),
                        ("height", height.into()),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        let title = TITLES
            .iter()
            .find_map(|pattern| pattern.captures(&webpage).map(|found| found[1].to_owned()))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract title; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let description = meta(&webpage, &["description", "twitter:description"])
            .or_else(|| {
                crate::html_elements::elements_text_and_html_by_attribute(
                    "id", "des_text", &webpage,
                )
                .ok()
                .and_then(|found| found.into_iter().next())
                .map(|(text, _)| text.trim().to_owned())
            })
            .map_or(Value::Null, Value::from);
        let thumbnail = match meta(&webpage, &["twitter:image"]) {
            Some(thumbnail) => Some(thumbnail),
            None => {
                let path = IMG
                    .captures(&webpage)
                    .and_then(|found| quoted(&found).map(str::to_owned));
                if path.is_none() {
                    crate::screen::warning(&format!(
                        "[{NAME}] unable to extract thumbnail; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ));
                }
                path.and_then(|path| super::videopress::urljoin(Some(url.as_str()), &path))
            }
        };
        let uploader = search_optional(&webpage, &[&UPLOADER], "uploader");
        let uploader_url = uploader
            .as_deref()
            .filter(|uploader| !uploader.is_empty())
            .map_or_else(String::new, |uploader| {
                format!("https://www.vidlii.com/user/{uploader}")
            });
        let upload_date = meta(&webpage, &["datePublished"])
            .or_else(|| search_optional(&webpage, &[&DATE], "upload date"))
            .and_then(|date| crate::dates::unified_strdate(&date, true));
        let duration = meta(&webpage, &["video:duration"])
            .or_else(|| search_optional(&webpage, &[&DURATION], "duration"))
            .and_then(|duration| duration.trim().parse::<i64>().ok());
        let view_count = search_optional(&webpage, &[&VIEWS[0], &VIEWS[1]], "view count")
            .and_then(|views| views.replace(',', "").parse::<i64>().ok());
        let comment_count =
            search_optional(&webpage, &[&COMMENTS[0], &COMMENTS[1]], "comment count")
                .and_then(|count| count.parse::<i64>().ok());
        let average_rating = search_optional(&webpage, &[&RATING], "average rating")
            .and_then(|rating| rating.parse::<f64>().ok());
        let category = search_optional(&webpage, &[&CATEGORY], "category")
            .map(|category| crate::html::clean_html(&category))
            .filter(|category| !category.is_empty());
        let tags: Vec<Value> = TAG
            .captures_iter(&webpage)
            .map(|found| found[1].trim().to_owned())
            .filter(|tag| !tag.is_empty())
            .map(Value::from)
            .collect();
        let opt = |value: Option<i64>| value.map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("description", description),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
                ("uploader", uploader.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
                ("uploader_url", uploader_url.into()),
                ("upload_date", upload_date.map_or(Value::Null, Value::from)),
                ("duration", opt(duration)),
                ("view_count", opt(view_count)),
                ("comment_count", opt(comment_count)),
                (
                    "average_rating",
                    average_rating.map_or(Value::Null, |rating| json!(rating)),
                ),
                (
                    "categories",
                    category.map_or(Value::Null, |category| Value::Array(vec![category.into()])),
                ),
                (
                    "tags",
                    if tags.is_empty() {
                        Value::Null
                    } else {
                        Value::Array(tags)
                    },
                ),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.vidlii.com/watch?v=tJluaH4BJ3v"));
        assert!(suitable("https://www.vidlii.com/embed?v=tJluaH4BJ3v&a=0"));
        assert!(!suitable("https://www.vidlii.com/user/x"));
    }
}
