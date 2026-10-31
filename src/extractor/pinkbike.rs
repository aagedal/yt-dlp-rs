//! Upstream `PinkbikeIE` (`yt_dlp/extractor/pinkbike.py`): the video page's
//! `data-quality`/`src` pairs as formats with the OpenGraph, description,
//! uploader, date, location and stat counters.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Pinkbike;

const NAME: &str = "Pinkbike";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:www\.)?pinkbike\.com/video/|es\.pinkbike\.org/i/kvid/kvid-y5\.swf\?id=)(?P<id>[0-9]+)",
    )
    .unwrap()
});
// `data-quality=((?:\\)?["\'])(.+?)\1[^>]+src=\1(.+?)\1`, one alternative
// per quote (in the backtracking engine's order).
static FORMATS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"data-quality=(?:\\"(?P<a1>.+?)\\"[^>]+src=\\"(?P<a2>.+?)\\""#,
        r#"|\\'(?P<b1>.+?)\\'[^>]+src=\\'(?P<b2>.+?)\\'"#,
        r#"|"(?P<c1>.+?)"[^>]+src="(?P<c2>.+?)""#,
        r#"|'(?P<d1>.+?)'[^>]+src='(?P<d2>.+?)')"#,
    ))
    .unwrap()
});
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)[pP]$").unwrap());
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)id="media-description"[^>]*>(.+?)<"#).unwrap());
static UPLOADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+\brel=["']author[^>]+>([^<]+)"#).unwrap());
static UPLOAD_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class="fullTime"[^>]+title="([^"]+)""#).unwrap());
static LOCATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<dt>Location</dt>\s*<dd>(.+?)<").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_regex(pattern, webpage, name, fatal=False)`.
fn search(pattern: &Regex, webpage: &str, name: &str) -> Option<String> {
    let found = pattern.captures(webpage).map(|found| found[1].to_owned());
    if found.is_none() {
        search_warning(NAME, name);
    }
    found
}

/// `str_to_int`: the digits without `,.+`.
fn str_to_int(text: &str) -> Option<i64> {
    text.chars()
        .filter(|c| !matches!(c, ',' | '.' | '+'))
        .collect::<String>()
        .parse()
        .ok()
}

fn optional(value: Option<impl Into<Value>>) -> Value {
    value.map_or(Value::Null, Into::into)
}

impl Extractor for Pinkbike {
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
        let page = Url::parse(&format!("http://www.pinkbike.com/video/{video_id}"))?;
        let webpage = download_webpage(client, &page, NAME, &video_id)?;
        let mut formats = Vec::new();
        for found in FORMATS.captures_iter(&webpage) {
            let pair = |first: &str, second: &str| {
                Some((found.name(first)?.as_str(), found.name(second)?.as_str()))
            };
            let (format_id, src) = pair("a1", "a2")
                .or_else(|| pair("b1", "b2"))
                .or_else(|| pair("c1", "c2"))
                .or_else(|| pair("d1", "d2"))
                .unwrap();
            let height = HEIGHT
                .captures(format_id)
                .and_then(|found| found[1].parse::<i64>().ok());
            let mut map = Map::new();
            map.insert("url".into(), src.into());
            map.insert("format_id".into(), format_id.into());
            map.insert("height".into(), optional(height));
            let ext = super::direct::determine_ext(src).unwrap_or_else(|| "unknown_video".into());
            map.insert("ext".into(), ext.into());
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "dict:url,format_id,height");
            formats.push(format);
        }
        // `remove_end(self._og_search_title(webpage), ' Video - Pinkbike')`.
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            search_warning(NAME, "OpenGraph title");
        }
        let title = title.map(|title| {
            title
                .strip_suffix(" Video - Pinkbike")
                .map_or(title.clone(), str::to_owned)
        });
        let description = match DESCRIPTION
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .filter(|description| !description.is_empty())
        {
            Some(description) => Some(description),
            None => {
                let og = crate::html::og_search_property(&webpage, "description");
                if og.is_none() {
                    search_warning(NAME, "OpenGraph description");
                }
                // `remove_start(og, title + '. ')`.
                let Some(title) = &title else {
                    return Err(Error::Reported(
                        "unsupported operand type(s) for +: 'NoneType' and 'str'".into(),
                    ));
                };
                og.map(|og| {
                    og.strip_prefix(&format!("{title}. "))
                        .map_or(og.clone(), str::to_owned)
                })
            }
        };
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            search_warning(NAME, "thumbnail URL");
        }
        let duration = crate::html::html_search_meta(&webpage, "video:duration");
        if duration.is_none() {
            search_warning(NAME, "duration");
        }
        let duration = super::streamable::int_or_none(duration.map(Value::from).as_ref());
        let uploader = search(&UPLOADER, &webpage, "uploader");
        let upload_date = search(&UPLOAD_DATE, &webpage, "upload date")
            .and_then(|date| crate::dates::unified_strdate(&date, true));
        let location = search(&LOCATION, &webpage, "location")
            .map(|location| crate::html::clean_html(&location));
        let count = |label: &str| {
            let pattern = Regex::new(&format!(
                r#"<span[^>]+class="stat-num"[^>]*>([\d,.]+)</span>\s*<span[^>]+class="stat-label"[^>]*>{label}"#
            ))
            .expect("static pattern");
            search(&pattern, &webpage, label).and_then(|count| str_to_int(&count))
        };
        let view_count = count("Views");
        let comment_count = count("Comments");
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = video_id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        match title {
            Some(title) => info.title = title,
            None => {
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        for (key, value) in [
            ("description", optional(description)),
            ("thumbnail", optional(thumbnail)),
            ("duration", duration),
            ("upload_date", optional(upload_date)),
            ("uploader", optional(uploader)),
            ("location", optional(location)),
            ("view_count", optional(view_count)),
            ("comment_count", optional(comment_count)),
        ] {
            // Processing drops a `None` duration.
            if key == "duration" && value.is_null() {
                continue;
            }
            crate::metadata_parser::set_field(&mut info, key, value);
        }
        let shape = if info.metadata.duration.is_some() {
            "dict;id,title,description,thumbnail,duration,upload_date,uploader,location,view_count,comment_count,formats"
        } else {
            "dict;id,title,description,thumbnail,upload_date,uploader,location,view_count,comment_count,formats"
        };
        Ok(Some(crate::info_order::mark_info(info, shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.pinkbike.com/video/402811/"));
        assert!(suitable(
            "http://es.pinkbike.org/i/kvid/kvid-y5.swf?id=406629"
        ));
        assert!(!suitable("https://www.pinkbike.com/news/"));
        assert_eq!(str_to_int("1,234.5"), Some(12345));
    }
}
