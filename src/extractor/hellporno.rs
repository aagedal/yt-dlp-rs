//! Upstream `HellPornoIE` (`yt_dlp/extractor/hellporno.py`): the first HTML5
//! media entry merged with the page's ID, title, description, keywords and
//! OpenGraph duration/release date.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct HellPorno;

const NAME: &str = "HellPorno";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?hellporno\.(?:com/videos|net/v)/(?P<id>[^/]+)",
    )
    .unwrap()
});
static VIDEO_IDS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"chs_object\s*=\s*["'](\d+)"#).unwrap(),
        Regex::new(r#"params\[["']video_id["']\]\s*=\s*(\d+)"#).unwrap(),
    ]
});
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class=["']desc_video_view_v2[^>]+>([^<]+)"#).unwrap());
static VIEWS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r">Views\s+(\d+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none` of a string.
fn int_or_none(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}

/// `_og_search_property(prop, webpage, fatal=False)`.
fn og_optional(html: &str, prop: &str) -> Option<String> {
    let value = crate::html::og_search_property(html, prop);
    if value.is_none() {
        search_warning(NAME, &format!("OpenGraph {prop}"));
    }
    value
}

impl Extractor for HellPorno {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        // `remove_end(self._html_extract_title(webpage), ' - Hell Porno')`.
        let title = crate::html::html_extract_title(&webpage).map(|title| {
            title
                .strip_suffix(" - Hell Porno")
                .map_or(title.clone(), str::to_owned)
        });
        if title.is_none() {
            search_warning(NAME, "title");
        }
        let entry = super::html5::parse_html5_media_entries(
            client,
            url,
            &webpage,
            &display_id,
            NAME,
            None,
            None,
        )?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let id = VIDEO_IDS
            .iter()
            .find_map(|pattern| pattern.captures(&webpage).map(|found| found[1].to_owned()))
            .unwrap_or_else(|| display_id.clone());
        let description = DESCRIPTION
            .captures(&webpage)
            .map(|found| found[1].to_owned());
        if description.is_none() {
            search_warning(NAME, "description");
        }
        let categories: Vec<Value> = crate::html::html_search_meta(&webpage, "keywords")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|category| !category.is_empty())
            .map(Value::from)
            .collect();
        let duration = og_optional(&webpage, "video:duration")
            .as_deref()
            .and_then(int_or_none);
        let timestamp = og_optional(&webpage, "video:release_date")
            .and_then(|date| crate::dates::unified_timestamp(&date, true));
        let view_count = VIEWS
            .captures(&webpage)
            .and_then(|found| int_or_none(&found[1]));
        if !VIEWS.is_match(&webpage) {
            search_warning(NAME, "view count");
        }
        let first = entry
            .formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        if entry.formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        // `merge_dicts(info, {...})` keeps the first non-`None` values.
        let mut keys = vec!["formats", "subtitles"];
        if entry.thumbnail.is_some() {
            keys.push("thumbnail");
        }
        keys.extend(["id", "display_id"]);
        match title {
            Some(title) => {
                info.title = title;
                keys.push("title");
            }
            None => {
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.formats = entry.formats;
        info.subtitles = entry.subtitles;
        info.metadata.thumbnail = entry.thumbnail;
        info.direct = None;
        info.id = id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(&mut info, "display_id", Value::String(display_id));
        for (name, value) in [
            ("description", description.map(Value::from)),
            ("categories", Some(Value::Array(categories))),
            ("duration", duration.map(Value::from)),
            ("timestamp", timestamp.map(Value::from)),
            ("view_count", view_count.map(Value::from)),
            ("age_limit", Some(Value::from(18))),
        ] {
            if let Some(value) = value {
                crate::metadata_parser::set_field(&mut info, name, value);
                keys.push(name);
            }
        }
        // A missing `title` key gets processing's generic title last.
        let untitled = !keys.contains(&"title");
        let shape = format!(
            "dict;{}{}",
            keys.join(","),
            if untitled { "-untitled" } else { "" }
        );
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://hellporno.com/videos/dixie-is-posing-with-naked-ass-very-erotic/"
        ));
        assert!(suitable("http://hellporno.net/v/186271/"));
        assert!(!suitable("http://hellporno.com/categories/"));
    }
}
