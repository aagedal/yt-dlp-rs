//! Upstream `MyVideoGeIE` (`yt_dlp/extractor/myvideoge.py`): the video
//! page's `jwplayer("mvplayer").setup(...)` sources (through `js_to_json`)
//! as `_parse_jwplayer_formats` formats ranked SD < HD, the title,
//! description, uploader, Georgian upload date and like counts.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MyVideoGe;

const NAME: &str = "MyVideoGe";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?myvideo\.ge/v/(?P<id>[0-9]+)").unwrap());
static SOURCES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)jwplayer\s*\(\s*['"]mvplayer['"]\s*\)\s*\.\s*setup\s*\(.*?\bsources\s*:\s*(\[.*?])\s*[,});]"#,
    )
    .unwrap()
});
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<title\b[^>]*>([^<]+)</title\b").unwrap());
static UPLOADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+class="mv_user_name"[^>]*>([^<]+)<"#).unwrap());
/// `urljoin(None, url)` keeps only absolute and protocol-relative URLs.
static ABSOLUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[a-zA-Z][a-zA-Z0-9+\-.]*:)?//").unwrap());

const MONTH_NAMES_KA: [&str; 12] = [
    "იანვარი",
    "თებერვალი",
    "მარტი",
    "აპრილი",
    "მაისი",
    "ივნისი",
    "ივლისი",
    "აგვისტო",
    "სექტემბერი",
    "ოქტომბერი",
    "ნოემბერი",
    "დეკემბერი",
];
const MONTH_NAMES_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_element_by_class(class, webpage)`.
fn element_by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(text, _)| text)
}

/// `get_element_by_id(id, webpage)`.
fn element_by_id(id: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_attribute("id", id, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(text, _)| text)
}

/// Python `int_or_none(text)` (`int(text)` strips whitespace).
fn int_or_none(text: Option<String>) -> Option<i64> {
    text?.trim().parse().ok()
}

/// `qualities(('SD', 'HD'))`.
fn quality(format_id: &str) -> f64 {
    match format_id {
        "SD" => 0.0,
        "HD" => 1.0,
        _ => -1.0,
    }
}

/// The `upload_date`: Georgian month names become English ones before
/// `unified_strdate`.
fn upload_date(text: &str) -> Option<String> {
    let mut text = text.to_owned();
    for (ka, en) in MONTH_NAMES_KA.iter().zip(MONTH_NAMES_EN) {
        text = text.replace(ka, en);
    }
    crate::dates::unified_strdate(&text, true)
}

impl Extractor for MyVideoGe {
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
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let title = crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
            .or_else(|| {
                element_by_class("my_video_title", &webpage)
                    .map(|text| crate::html::clean_html(&text))
                    .filter(|title| !title.is_empty())
            });
        let title = match title {
            Some(title) => title,
            None => TITLE
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]))
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] {video_id}: Unable to extract title; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?,
        };
        let sources = SOURCES.captures(&webpage).map(|found| found[1].to_owned());
        if sources.is_none() {
            super::search_warning(NAME, "jwplayer sources");
        }
        let json = crate::js_to_json::js_to_json(sources.as_deref().unwrap_or(""));
        let sources = match crate::json::python_decode_error_text(&json) {
            Some(error) => {
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Failed to parse JSON: {error}"
                ));
                None
            }
            None => crate::json::from_str::<Value>(&json).ok(),
        };
        // `_parse_jwplayer_formats(jwplayer_sources or [], video_id)`.
        let sources: Vec<Value> = match sources {
            Some(Value::Array(sources)) => sources
                .into_iter()
                .filter(|source| {
                    source
                        .get("file")
                        .and_then(Value::as_str)
                        .is_some_and(|file| ABSOLUTE.is_match(file))
                })
                .collect(),
            Some(Value::Null | Value::Bool(false)) | None => Vec::new(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "MyVideoGe jwplayer sources that are not a list".into(),
                ));
            }
        };
        let mut formats = Vec::new();
        let data = serde_json::json!({ "sources": sources });
        if let Some(entry) = super::jwplayer::parse(&data, &video_id, url)?
            && let Some((info, _)) =
                super::html5::jwplayer_entry_info(client, entry, url, &video_id, NAME)?
        {
            formats = info.formats;
        }
        for format in &mut formats {
            format.quality = Some(quality(&format.format_id));
            crate::info_order::mark_int_quality(format);
        }
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            super::search_warning(NAME, "OpenGraph description");
        }
        let description = description
            .filter(|description| !description.is_empty())
            .or_else(|| element_by_id("long_desc_holder", &webpage).filter(|text| !text.is_empty()))
            .or_else(|| {
                let meta = crate::html::html_search_meta_clean(&webpage, "description");
                if meta.is_none() {
                    super::search_warning(NAME, "description");
                }
                meta
            });
        let uploader = UPLOADER
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]));
        if uploader.is_none() {
            super::search_warning(NAME, "uploader");
        }
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            super::search_warning(NAME, "thumbnail URL");
        }
        let upload_date = element_by_class("mv_vid_upl_date", &webpage)
            .filter(|text| !text.is_empty())
            .and_then(|text| upload_date(&text));
        let view_count = int_or_none(element_by_class("mv_vid_views", &webpage));
        let like_count = int_or_none(element_by_id("likes_count", &webpage));
        let dislike_count = int_or_none(element_by_id("dislikes_count", &webpage));

        let first = formats.first().map_or_else(
            || url.clone(),
            |format: &crate::VideoFormat| format.url.clone(),
        );
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        if formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.formats = formats;
        info.direct = None;
        info.id = video_id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        // The returned dict keeps its `None` values.
        let fields: [(&str, Option<Value>); 8] = [
            ("description", description.map(Value::from)),
            ("uploader", uploader.map(Value::from)),
            ("thumbnail", thumbnail.map(Value::from)),
            ("upload_date", upload_date.map(Value::from)),
            ("view_count", view_count.map(Value::from)),
            ("like_count", like_count.map(Value::from)),
            ("dislike_count", dislike_count.map(Value::from)),
            ("formats", None),
        ];
        for (key, value) in fields {
            match value {
                Some(value) => crate::metadata_parser::set_field(&mut info, key, value),
                None if key != "formats" => {
                    info.extra.insert(key.into(), Value::Null);
                }
                None => {}
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,description,uploader,formats,thumbnail,upload_date,view_count,like_count,dislike_count",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.myvideo.ge/v/3941048"));
        assert!(suitable("http://myvideo.ge/v/1"));
        assert!(!suitable("https://www.myvideo.ge/v/abc"));
    }

    #[test]
    fn georgian_months_become_dates() {
        assert_eq!(upload_date("ივნისი 11, 2020").as_deref(), Some("20200611"));
    }
}
