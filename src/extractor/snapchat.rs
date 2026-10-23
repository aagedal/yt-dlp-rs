//! Upstream `SnapchatSpotlightIE` (`yt_dlp/extractor/snapchat.py`): the
//! spotlight page's Next.js story metadata, whose `contentUrl` is the
//! result's own (only) format.
use super::Extractor;
use crate::{Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct SnapchatSpotlight;

const NAME: &str = "SnapchatSpotlight";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?snapchat\.com/spotlight/(?P<id>\w+)").unwrap()
});

/// `_VALID_URL`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `traverse_obj(value, (*path))`: `None` for a missing key or `null`.
fn at<'a>(value: Option<&'a Value>, path: &[&str]) -> Option<&'a Value> {
    let mut current = value?;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current).filter(|value| !value.is_null())
}

/// `int_or_none`.
fn int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(flag) => Some(i64::from(*flag)),
        _ => None,
    }
}

/// `int_or_none` then `None` for `-1` (a hidden count).
fn count(value: Option<&Value>) -> Option<Value> {
    int(value).filter(|count| *count != -1).map(Value::from)
}

/// `float_or_none(scale=1000)`.
fn milliseconds(value: Option<&Value>) -> Option<Value> {
    let number = match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    serde_json::Number::from_f64(number / 1000.0).map(Value::Number)
}

fn text(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| value.is_string()).cloned()
}

fn url(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map(Value::from)
}

impl Extractor for SnapchatSpotlight {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, page: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let webpage = super::download_webpage(client, page, NAME, &video_id)?;
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        // `['props']['pageProps']`.
        let key_error = |key: &str| {
            crate::Error::Reported(format!(
                "[{NAME}] {video_id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let props = next_data.get("props").ok_or_else(|| key_error("props"))?;
        let page_props = props
            .get("pageProps")
            .ok_or_else(|| key_error("pageProps"))?;
        // `('spotlightFeed', 'spotlightStories', lambda _, v:
        // v['story']['storyId']['value'] == video_id, 'metadata', any)`.
        let video_data = page_props
            .pointer("/spotlightFeed/spotlightStories")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|story| {
                story
                    .pointer("/story/storyId/value")
                    .and_then(Value::as_str)
                    == Some(video_id.as_str())
            })
            .find_map(|story| story.get("metadata").filter(|value| !value.is_null()));

        // `{'id': ..., 'ext': 'mp4', **traverse_obj(video_data,
        // ('videoMetadata', {...})), **traverse_obj(video_data, {...})}`:
        // `None` values left out, a later key replacing an earlier value.
        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("ext".into(), "mp4".into());
        let metadata = at(video_data, &["videoMetadata"]);
        let creator = |key: &str| at(metadata, &["creator", "personCreator", key]);
        for (key, value) in [
            ("title", text(at(metadata, &["name"]))),
            ("description", text(at(metadata, &["description"]))),
            ("timestamp", milliseconds(at(metadata, &["uploadDateMs"]))),
            ("view_count", count(at(metadata, &["viewCount"]))),
            (
                "repost_count",
                int(at(metadata, &["shareCount"])).map(Value::from),
            ),
            ("url", url(at(metadata, &["contentUrl"]))),
            ("width", int(at(metadata, &["width"])).map(Value::from)),
            ("height", int(at(metadata, &["height"])).map(Value::from)),
            ("duration", milliseconds(at(metadata, &["durationMs"]))),
            ("thumbnail", url(at(metadata, &["thumbnailUrl"]))),
            ("uploader", text(creator("username"))),
            ("uploader_url", url(creator("url"))),
        ] {
            if let Some(value) = value {
                dict.insert(key.into(), value);
            }
        }
        let tags: Vec<Value> = at(video_data, &["hashtags"])
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|tag| tag.is_string())
            .cloned()
            .collect();
        for (key, value) in [
            ("description", text(at(video_data, &["description"]))),
            ("tags", (!tags.is_empty()).then_some(Value::Array(tags))),
            (
                "view_count",
                count(at(video_data, &["engagementStats", "viewCount"])),
            ),
            (
                "repost_count",
                int(at(video_data, &["engagementStats", "shareCount"])).map(Value::from),
            ),
        ] {
            if let Some(value) = value {
                dict.insert(key.into(), value);
            }
        }

        let keys: Vec<String> = dict.keys().cloned().collect();
        let media = dict
            .get("url")
            .and_then(Value::as_str)
            .and_then(|media| Url::parse(media).ok());
        let mut info = match &media {
            Some(media) => {
                let mut info = super::direct::with_format(
                    super::direct::build_at(media.clone(), media, "mp4".into()),
                    "",
                );
                for format in &mut info.formats {
                    format.ext = "mp4".into();
                    format.format_id = String::new();
                    format.width = dict
                        .get("width")
                        .and_then(Value::as_u64)
                        .and_then(|width| u32::try_from(width).ok());
                    format.height = dict
                        .get("height")
                        .and_then(Value::as_u64)
                        .and_then(|height| u32::try_from(height).ok());
                }
                info.format_id = None;
                info
            }
            // Without a URL upstream's result has no formats.
            None => {
                let mut info = super::direct::build_at(page.clone(), page, "mp4".into());
                info.formats.clear();
                info.extra
                    .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
                info
            }
        };
        info.webpage_url = page.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.id = video_id;
        match dict.get("title").and_then(Value::as_str) {
            Some(title) => info.title = title.to_owned(),
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        for (key, value) in dict {
            if matches!(key.as_str(), "id" | "ext" | "title" | "url") {
                continue;
            }
            crate::metadata_parser::set_field(&mut info, &key, value);
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("untyped-dict;{}", keys.join(",")),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.snapchat.com/spotlight/W7_EDlXWTBiXAEEniNoMPwAAYYWtidGhudGZpAX1TKn0JAX1TKnXJAAAAAA"
        ));
        assert!(suitable("http://snapchat.com/spotlight/abc_123?x=1"));
        assert!(!suitable("https://www.snapchat.com/add/shreypatel57"));
        assert!(!suitable("https://story.snapchat.com/spotlight/abc"));
    }

    #[test]
    fn hidden_counts_are_none() {
        assert_eq!(count(Some(&Value::from("-1"))), None);
        assert_eq!(count(Some(&Value::from(-1))), None);
        assert_eq!(count(Some(&Value::from("12"))), Some(Value::from(12)));
    }
}
