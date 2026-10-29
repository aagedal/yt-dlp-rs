//! Upstream `NuvidIE` (`yt_dlp/extractor/nuvid.py`): the `player_config_json`
//! MP4 files (one per quality, checked like upstream's `_check_formats`), the
//! title and the `video-tmb-wrap` thumbnails of the mobile page, and the
//! poster. The mobile page's title fallback is approximated: the backreference
//! quote matching of the upstream patterns is split into alternatives.
use super::{
    Extractor, check_formats, dict_format, dict_info, download_json_headers, download_webpage_note,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Nuvid;

const NAME: &str = "Nuvid";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www|m)\.nuvid\.com/video/(?P<id>[0-9]+)").unwrap());
static TITLE_ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span\s[^>]*?\btitle\s*=\s*(?:"([^"]+)"|'([^']+)'|\b([^"]+))\s*>"#).unwrap()
});
static TITLE_HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<div\s[^>]*?\bclass\s*=\s*(?:"|')?thumb-holder video(?:"|')?>\s*<h5\b[^>]*>([^<]+)</h5"#,
    )
    .unwrap()
});
static TITLE_CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span\s[^>]*?\bclass\s*=\s*(?:"|')?title_thumb(?:"|')?>([^<]+)</span"#).unwrap()
});
static THUMBNAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div\s+class\s*=\s*"video-tmb-wrap"\s*>\s*<img\s+src\s*=\s*"([^"]+)"\s*/>"#)
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Nuvid {what} are not supported natively"))
}

fn search_error(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// The mobile page, or `''` after a warning (`fatal=False`).
fn optional_webpage(client: &Client, url: &Url, id: &str) -> Result<String> {
    match download_webpage_note(client, url, NAME, id, "Downloading video page") {
        Ok(text) => Ok(text),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download webpage: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(String::new())
        }
        Err(error) => Err(error),
    }
}

/// `_html_search_regex` over the title patterns (`clean_html`ed).
fn title_from_page(webpage: &str) -> Option<String> {
    let found = TITLE_ATTRIBUTE
        .captures(webpage)
        .and_then(|captures| {
            (1..=3).find_map(|group| captures.get(group).map(|text| text.as_str().to_owned()))
        })
        .or_else(|| {
            TITLE_HEADING
                .captures(webpage)
                .map(|captures| captures[1].to_owned())
        })
        .or_else(|| {
            TITLE_CLASS
                .captures(webpage)
                .map(|captures| captures[1].to_owned())
        })?;
    Some(crate::html::clean_html(&found))
}

/// Python's `int(text)` for the quality labels.
fn height_of(label: &str) -> Value {
    label.parse::<i64>().map_or(Value::Null, Value::from)
}

impl Extractor for Nuvid {
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

        let json_url = Url::parse(&format!(
            "https://www.nuvid.com/player_config_json/?vid={video_id}&aid=0&domain_id=0&embed=0&check_speed=0"
        ))?;
        let video_data = download_json_headers(
            client,
            &json_url,
            NAME,
            &video_id,
            &[
                ("Accept", "application/json, text/javascript, */*; q = 0.01"),
                (
                    "Content-Type",
                    "application/x-www-form-urlencoded; charset=utf-8",
                ),
            ],
        )?;

        let page_url = Url::parse(&format!("http://m.nuvid.com/video/{video_id}"))?;
        let webpage = optional_webpage(client, &page_url, &video_id)?;

        // `strip_or_none(video_data.get('title') or _html_search_regex(...))`.
        let title = match video_data.get("title").and_then(Value::as_str) {
            Some(title) if !title.is_empty() => title.trim().to_owned(),
            _ => title_from_page(&webpage).ok_or_else(|| search_error(&video_id, "title"))?,
        };
        let title = Some(title.trim().to_owned()).filter(|title| !title.is_empty());

        let files = match video_data.get("files") {
            Some(Value::Object(files)) => files,
            Some(Value::Null) | None => {
                return Err(Error::Reported(
                    "'NoneType' object has no attribute 'items'".into(),
                ));
            }
            Some(_) => return Err(unsupported("files that are not objects")),
        };
        let qualities = [("lq", "360p"), ("hq", "720p")];
        let mut formats = Vec::new();
        for (quality, source) in files {
            if !super::nuxt::truthy(source) {
                continue;
            }
            let Some((_, format_id)) = qualities.iter().find(|(name, _)| *name == quality.as_str())
            else {
                return Err(Error::Reported(
                    "'NoneType' object is not subscriptable".into(),
                ));
            };
            let Some(source) = source.as_str() else {
                return Err(unsupported("files whose source is not a string"));
            };
            formats.push(dict_format(
                vec![
                    ("url", source.into()),
                    ("format_id", (*format_id).into()),
                    ("height", height_of(&format_id[..format_id.len() - 1])),
                ],
                NAME,
                &video_id,
            )?);
        }
        check_formats(client, &mut formats, NAME, &video_id)?;

        let mut thumbnails: Vec<Value> = THUMBNAIL
            .captures_iter(&webpage)
            .filter_map(|captures| super::json_ld::url_or_none(&captures[1]))
            .map(|thumb| serde_json::json!({ "url": thumb }))
            .collect();
        if let Some(poster) = video_data
            .get("poster")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            thumbnails.push(serde_json::json!({ "url": poster, "preference": 1 }));
        }

        let duration = [
            video_data.get("duration"),
            video_data.get("duration_format"),
        ]
        .into_iter()
        .flatten()
        .find(|value| !value.is_null())
        .and_then(|value| match value {
            Value::String(text) => crate::parse_duration(text),
            Value::Number(number) => number.as_f64().map(crate::info::PyNumber::Float),
            _ => None,
        })
        .map_or(Value::Null, |duration| match duration {
            crate::info::PyNumber::Int(value) => Value::from(value),
            crate::info::PyNumber::Float(value) => Value::from(value),
        });

        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("formats", Value::Null),
                ("title", title.map_or(Value::Null, Value::from)),
                ("thumbnails", Value::Array(thumbnails)),
                ("duration", duration),
                ("age_limit", 18.into()),
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
        assert!(suitable("https://www.nuvid.com/video/6513023/italian-babe"));
        assert!(suitable("https://m.nuvid.com/video/6523263"));
        assert!(suitable("http://m.nuvid.com/video/6415801/"));
        assert!(!suitable("https://www.nuvid.com/"));
    }

    #[test]
    fn page_title_fallback() {
        assert_eq!(
            title_from_page(r#"<span class="title_thumb">A &amp; B</span"#).as_deref(),
            Some("A & B")
        );
    }
}
