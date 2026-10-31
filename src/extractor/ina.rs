//! Upstream `InaIE` (`yt_dlp/extractor/ina.py`): the page's
//! `asset-details-url` JSON and its resource URL as an untyped result.
use super::clyp::key_error;
use super::{Extractor, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Ina;

const NAME: &str = "Ina";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:(?:www|m)\.)?ina\.fr/(?:[^?#]+/)(?P<id>[\w-]+)")
        .unwrap()
});
static API_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"asset-details-url\s*=\s*["'](?P<api_url>[^"']+)"#).unwrap());
static ASSET_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"assets/([^?/]+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unable(id: &str, name: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for Ina {
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
        // `_html_search_regex` (fatal).
        let api_url = API_URL
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found["api_url"]).trim().to_owned())
            .ok_or_else(|| unable(&display_id, "api_url"))?;
        let asset_id = ASSET_ID
            .captures(&api_url)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| unable(&display_id, "asset_id"))?;
        let json_url = api_url.replace(&asset_id, &format!("{asset_id}.json"));
        let json_url = Url::parse(&json_url).map_err(|_| {
            Error::UnsupportedHtml5(format!(
                "Ina asset URLs that are not absolute are not supported natively: {json_url}"
            ))
        })?;
        let api = download_json(client, &json_url, NAME, &asset_id)?;
        let media = api
            .get("resourceUrl")
            .ok_or_else(|| key_error(&asset_id, "resourceUrl"))?
            .as_str()
            .and_then(|media| Url::parse(media).ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Ina resource URLs that are not absolute URLs are not supported natively"
                        .into(),
                )
            })?;
        let ext = match api.get("type").and_then(Value::as_str) {
            Some("video") => "mp4".to_owned(),
            Some("audio") => "mp3".to_owned(),
            _ => super::direct::determine_ext(media.as_str())
                .unwrap_or_else(|| "unknown_video".into()),
        };
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.id = asset_id;
        let get = |key: &str| api.get(key).cloned().unwrap_or(Value::Null);
        match get("title") {
            Value::String(title) => info.title = title,
            Value::Null => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "Ina titles that are not strings are not supported natively".into(),
                ));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let upload_date = api
            .get("dateOfBroadcast")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_strdate(date, true))
            .map_or(Value::Null, Value::from);
        for (name, value) in [
            ("description", get("description")),
            ("upload_date", upload_date),
            ("duration", get("duration")),
            ("thumbnail", get("resourceThumbnail")),
        ] {
            if value.is_null() && name == "duration" {
                // Processing drops a `None` duration.
            } else if value.is_null() {
                info.extra.insert(name.into(), Value::Null);
            } else {
                crate::metadata_parser::set_field(&mut info, name, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            "untyped-dict;id,url,ext,title,description,upload_date,duration,thumbnail",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.ina.fr/video/I12055569/francois-hollande-je-crois-que-c-est-clair-video.html"
        ));
        assert!(suitable("https://m.ina.fr/video/I12055569"));
        assert!(!suitable("https://www.ina.fr/I12055569"));
    }
}
