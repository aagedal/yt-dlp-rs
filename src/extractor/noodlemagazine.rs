//! Upstream `NoodleMagazineIE` (`yt_dlp/extractor/noodlemagazine.py`): the
//! watch page's OpenGraph and `video:*` metadata and the `window.playlist`
//! JSON's direct MP4 sources. HLS sources are not supported natively.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::extractor::streamable::int_or_none;
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct NoodleMagazine;

const NAME: &str = "NoodleMagazine";
const BASE: &str = "https://adult.noodlemagazine.com";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www|adult\.)?noodlemagazine\.com/watch/(?P<id>[0-9_-]+)").unwrap()
});
static PLAYLIST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"window\.playlist\s*=").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("NoodleMagazine {what} are not supported natively"))
}

/// Upstream `parse_duration` / `parse_count` results as JSON numbers.
fn number(value: crate::info::PyNumber) -> Value {
    match value {
        crate::info::PyNumber::Int(value) => Value::from(value),
        crate::info::PyNumber::Float(value) => Value::from(value),
    }
}

impl Extractor for NoodleMagazine {
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

        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            super::search_warning(NAME, "OpenGraph title");
        }
        let duration = crate::html::html_search_meta(&webpage, "video:duration")
            .and_then(|text| crate::parse_duration(&text))
            .map_or(Value::Null, number);
        let description = crate::html::og_search_property(&webpage, "description")
            .unwrap_or_default()
            .replace(" watch online hight quality video", "");
        let tags: Vec<Value> = crate::html::html_search_meta(&webpage, "video:tag")
            .unwrap_or_default()
            .split(", ")
            .map(Value::from)
            .collect();
        let view_count = crate::html::html_search_meta(&webpage, "ya:ovs:views_total")
            .and_then(|text| super::youtube::dict::parse_count(&text));
        let like_count = crate::html::html_search_meta(&webpage, "ya:ovs:likes")
            .and_then(|text| super::youtube::dict::parse_count(&text));
        let upload_date =
            crate::html::html_search_meta(&webpage, "ya:ovs:upload_date").unwrap_or_default();
        let upload_date = crate::dates::unified_strdate(&upload_date, true);

        let playlist = match super::search_json(&PLAYLIST, &webpage, false) {
            Some(Some(value)) => value,
            Some(None) => return Err(unsupported("playlist JSON that does not parse")),
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract {video_id}; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };

        let mut formats = Vec::new();
        let sources = playlist.get("sources").and_then(Value::as_array);
        for source in sources.into_iter().flatten() {
            if !source.get("file").is_some_and(super::nuxt::truthy) {
                continue;
            }
            if source.get("type").and_then(Value::as_str) == Some("hls") {
                return Err(unsupported("HLS sources"));
            }
            let Some(file) = source.get("file").and_then(Value::as_str) else {
                return Err(unsupported("sources whose file is not a string"));
            };
            let file_url = Url::parse(BASE)?.join(file)?;
            formats.push(dict_format(
                vec![
                    ("url", file_url.as_str().into()),
                    (
                        "format_id",
                        source.get("label").cloned().unwrap_or(Value::Null),
                    ),
                    ("height", int_or_none(source.get("label"))),
                    ("ext", source.get("type").cloned().unwrap_or(Value::Null)),
                ],
                NAME,
                &video_id,
            )?);
        }

        // `_og_search_property('image', ...) or playlist_info.get('image')`.
        let thumbnail = crate::html::og_search_property(&webpage, "image")
            .filter(|image| !image.is_empty())
            .map(Value::from)
            .or_else(|| playlist.get("image").cloned())
            .unwrap_or(Value::Null);

        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("formats", Value::Null),
                ("title", title.map_or(Value::Null, Value::from)),
                ("thumbnail", thumbnail),
                ("duration", duration),
                ("description", description.into()),
                ("tags", Value::Array(tags)),
                ("view_count", view_count.map_or(Value::Null, Value::from)),
                ("like_count", like_count.map_or(Value::Null, Value::from)),
                ("upload_date", upload_date.map_or(Value::Null, Value::from)),
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
        assert!(suitable(
            "https://adult.noodlemagazine.com/watch/-67421364_456239604"
        ));
        assert!(suitable("https://noodlemagazine.com/watch/1_2"));
        assert!(!suitable("https://adult.noodlemagazine.com/"));
    }
}
