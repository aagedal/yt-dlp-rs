//! Upstream `StarTVIE` (`yt_dlp/extractor/startv.py`): Star TV episodes,
//! trailers and extras from the page's `videoUrl` API JSON (`data`), with
//! its HLS flavor, release date and OpenGraph fallbacks.
use super::{Extractor, dict_info, download_json, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct StarTv;

const NAME: &str = "startv";
const KEY: &str = "StarTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?startv\.com\.tr/(?:(?:dizi|program)/(?:[^/?#&]+)/(?:bolumler|fragmanlar|ekstralar)|video/arsiv/(?:dizi|program)/(?:[^/?#&]+))/(?P<id>[^/?#&]+)",
    )
    .unwrap()
});
/// `(["\'])videoUrl\1\s*:\s*\1(?P<url>(?:(?!\1).)+)\1` without backreferences.
static INFO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""videoUrl"\s*:\s*"(?P<dq>[^"\n]+)"|'videoUrl'\s*:\s*'(?P<sq>[^'\n]+)'"#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Star TV {what} are not supported natively"))
}

fn bug(id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `str(value)` for a JSON scalar.
fn py_str(value: Option<&Value>) -> Result<String> {
    match value {
        None | Some(Value::Null) => Ok("None".into()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Bool(flag)) => Ok(if *flag { "True" } else { "False" }.into()),
        Some(Value::Number(number)) => Ok(number.to_string()),
        Some(_) => Err(unsupported("video IDs that are not scalars")),
    }
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        Some(Value::Bool(flag)) => Value::from(i64::from(*flag)),
        _ => Value::Null,
    }
}

impl Extractor for StarTv {
    fn key(&self) -> &'static str {
        KEY
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
        let info_url = INFO_URL
            .captures(&webpage)
            .and_then(|found| found.name("dq").or_else(|| found.name("sq")))
            .map(|found| found.as_str().to_owned())
            .ok_or_else(|| bug(&display_id, "Unable to extract video info url"))?;
        let info_url =
            Url::parse(&info_url).map_err(|_| unsupported("video info URLs that are relative"))?;
        let response = download_json(client, &info_url, NAME, &display_id)?;
        // `traverse_obj(..., 'data', expected_type=dict)`, required truthy.
        let info = match response.get("data") {
            Some(Value::Object(info)) if !info.is_empty() => info,
            _ => return Err(bug(&display_id, "Failed to extract API data")),
        };
        let video_id = py_str(info.get("id"))?;
        let title = match info.get("title") {
            Some(Value::String(title)) if !title.is_empty() => Some(title.clone()),
            Some(title) if super::nuxt::truthy(title) => {
                return Err(unsupported("titles that are not strings"));
            }
            _ => {
                let title = crate::html::og_search_property(&webpage, "title");
                if title.is_none() {
                    search_warning(NAME, "OpenGraph title");
                }
                title
            }
        };
        let description = match info.get("description") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(crate::html::clean_html(text)),
            Some(_) => return Err(unsupported("descriptions that are not strings")),
        }
        .filter(|text| !text.is_empty())
        .or_else(|| crate::html::og_search_property(&webpage, "description"));
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            search_warning(NAME, "thumbnail URL");
        }
        let thumbnail = thumbnail.map(|url| super::proto_relative_http(&url));
        let manifest = match info.get("flavors").and_then(|flavors| flavors.get("hls")) {
            Some(Value::String(manifest)) if !manifest.is_empty() => Some(
                Url::parse(manifest)
                    .map_err(|_| unsupported("HLS flavors that are not absolute URLs"))?,
            ),
            Some(manifest) if super::nuxt::truthy(manifest) => {
                return Err(unsupported("HLS flavors that are not strings"));
            }
            _ => {
                crate::screen::warning(&format!(
                    "[{NAME}] Failed to obtain m3u8 URL; {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
                None
            }
        };
        let formats = manifest
            .and_then(|manifest| {
                super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("hls"),
                    false,
                    false,
                )
            })
            .map(|stream| stream.formats)
            .unwrap_or_default();
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("display_id", display_id.into()),
                ("title", optional(title)),
                ("description", optional(description)),
                ("thumbnail", optional(thumbnail)),
                ("timestamp", int_or_none(info.get("release_date"))),
                ("formats", Value::Null),
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
        assert!(suitable(
            "https://www.startv.com.tr/dizi/cocuk/bolumler/3-bolum"
        ));
        assert!(suitable(
            "https://www.startv.com.tr/video/arsiv/program/buyukrisk/14-bolumde-hangi-unlu-ne-sordu-"
        ));
        assert!(!suitable("https://www.startv.com.tr/dizi/cocuk"));
    }
}
