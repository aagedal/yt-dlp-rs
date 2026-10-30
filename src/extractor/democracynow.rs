//! Upstream `DemocracynowIE` (`yt_dlp/extractor/democracynow.py`): a
//! show's or story's inline `text/json` player data, its file/audio/video
//! URLs, caption files and the page's `og:description`.
use super::clyp::key_error;
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Democracynow;

const NAME: &str = "democracynow";
const KEY: &str = "Democracynow";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?democracynow\.org/(?P<id>[^?]*)").unwrap()
});
static JSON_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script[^>]+type="text/json"[^>]*>\s*(\{[^>]+\})"#).unwrap());
static QUERY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\?.*").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Democracynow {what} are not supported natively"))
}

/// `urllib.parse.urljoin(url, path)` of a string path.
fn join(url: &Url, path: &Value) -> Result<String> {
    let path = path
        .as_str()
        .ok_or_else(|| unsupported("media and caption paths that are not strings"))?;
    Ok(url.join(path)?.to_string())
}

/// `remove_start(os.path.splitext(url_basename(url))[0], 'dn')`.
fn media_id(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let path = path.split_once("://").map_or(path, |(_, rest)| {
        rest.split_once('/').map_or("", |(_, path)| path)
    });
    let basename = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let stem = match basename.rfind('.') {
        Some(index) if basename[..index].chars().any(|c| c != '.') => &basename[..index],
        _ => basename,
    };
    stem.strip_prefix("dn").unwrap_or(stem).to_owned()
}

impl Extractor for Democracynow {
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
        let json = JSON_DATA
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract json; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let json_data: Value = crate::json::from_str(&json).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {display_id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        if !json_data.is_object() {
            return Err(unsupported("player data that are not objects"));
        }
        let title = json_data
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&display_id, "title"))?;
        if !title.is_string() {
            return Err(unsupported("titles that are not strings"));
        }
        let mut formats = Vec::new();
        let mut video_id: Option<String> = None;
        for key in ["file", "audio", "video", "high_res_video"] {
            let Some(media_url) = json_data
                .get(key)
                .filter(|value| super::nuxt::truthy(value))
            else {
                continue;
            };
            let media_url = QUERY.replace(&join(url, media_url)?, "").into_owned();
            if video_id.as_deref().is_none_or(str::is_empty) {
                video_id = Some(media_id(&media_url));
            }
            formats.push(dict_format(
                vec![
                    ("url", media_url.into()),
                    (
                        "vcodec",
                        if key == "audio" {
                            "none".into()
                        } else {
                            Value::Null
                        },
                    ),
                ],
                NAME,
                &display_id,
            )?);
        }
        let mut subtitles: Map<String, Value> = Map::new();
        let mut add = |lang: String, url: String| {
            subtitles
                .entry(lang)
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("subtitle lists")
                .push(serde_json::json!({
                    "url": url,
                    // Processing's `determine_ext(url).lower()`.
                    "ext": super::direct::determine_ext(&url)
                        .unwrap_or_else(|| "unknown_video".into())
                        .to_lowercase(),
                }));
        };
        if let Some(caption_file) = json_data.get("caption_file") {
            add("en".into(), join(url, caption_file)?);
        }
        let captions = match json_data.get("captions") {
            None => Vec::new(),
            Some(Value::Array(items)) => items.clone(),
            Some(_) => return Err(unsupported("caption lists that are not arrays")),
        };
        for item in captions {
            let lang = match item.get("language") {
                None => String::new(),
                Some(Value::String(lang)) => lang.to_lowercase(),
                Some(_) => return Err(unsupported("caption languages that are not strings")),
            };
            let lang = if lang.is_empty() { "en".into() } else { lang };
            let path = item
                .get("url")
                .ok_or_else(|| key_error(&display_id, "url"))?;
            add(lang, join(url, path)?);
        }
        let description = crate::html::og_search_property(&webpage, "description");
        let id = video_id
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| display_id.clone());
        let subtitles: crate::Subtitles = serde_json::from_value(Value::Object(subtitles))
            .map_err(|error| Error::Reported(format!("[{NAME}] {display_id}: {error}")))?;
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", id.into()),
                ("title", title),
                ("description", description.map_or(Value::Null, Value::from)),
                (
                    "thumbnail",
                    json_data.get("image").cloned().unwrap_or(Value::Null),
                ),
                ("subtitles", Value::Null),
                ("formats", Value::Null),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.democracynow.org/shows/2015/7/3"));
        assert!(suitable(
            "http://www.democracynow.org/2015/7/3/this_flag_comes_down_today_bree"
        ));
        assert!(!suitable("https://democracynow.com/x"));
    }

    #[test]
    fn media_ids_follow_upstream_basenames() {
        assert_eq!(
            media_id("https://hot.dn.org/democracynow/360/dn2015-0703.mp4"),
            "2015-0703"
        );
        assert_eq!(media_id("https://a.test/dir/"), "dir");
    }
}
