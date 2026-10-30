//! Upstream `CraftsyIE` (`yt_dlp/extractor/craftsy.py`): Craftsy classes as
//! playlists of Brightcove `url` results for their lessons (from the class
//! video player's Livewire snapshot), or the `video-js` preview without
//! access, with the page's title and description meta tags.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Craftsy;

const NAME: &str = "Craftsy";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://www\.craftsy\.com/class/(?P<id>[\w-]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn each(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    }
}

/// Python's `str()`/f-string of a lesson's video ID.
fn id_text(value: &Value) -> Result<String> {
    match value {
        Value::String(id) => Ok(id.clone()),
        Value::Number(id) => Ok(id.to_string()),
        _ => Err(Error::UnsupportedHtml5(
            "Craftsy video IDs that are not strings are not supported natively".into(),
        )),
    }
}

impl Extractor for Craftsy {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
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
        let player =
            crate::html_elements::elements_text_and_html_by_class("class-video-player", &webpage)
                .ok()
                .and_then(|found| found.into_iter().next())
                .map(|(_, whole)| whole);
        // `({extract_attributes}, 'wire:snapshot', {json.loads}, 'data', {dict})`.
        let video_data: Map<String, Value> = player
            .as_deref()
            .map(crate::html_elements::extract_attributes)
            .and_then(|attributes| {
                attributes
                    .get("wire:snapshot")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .and_then(|snapshot| serde_json::from_str::<Value>(&snapshot).ok())
            .and_then(|snapshot| snapshot.get("data").and_then(Value::as_object).cloned())
            .unwrap_or_default();
        let video_js = player
            .as_deref()
            .and_then(|player| {
                crate::html_elements::element_text_and_html_by_tag("video-js", player).ok()
            })
            .map(|(_, whole)| crate::html_elements::extract_attributes(&whole))
            .unwrap_or_default();
        let has_access = video_data
            .get("userHasAccess")
            .is_some_and(super::nuxt::truthy);
        // `('lessons', ..., ..., lambda _, v: v['video_id'])`: the filter
        // applies to the children of each second-level item.
        let mut lessons: Vec<Value> = each(video_data.get("lessons"))
            .into_iter()
            .flat_map(|group| each(Some(group)))
            .flat_map(|item| each(Some(item)))
            .filter(|lesson| lesson.get("video_id").is_some_and(super::nuxt::truthy))
            .cloned()
            .collect();
        if let Some(preview_id) = video_js
            .get("data-video-id")
            .filter(|id| super::nuxt::truthy(id))
            .cloned()
            && !lessons
                .iter()
                .any(|lesson| lesson.get("video_id") == Some(&preview_id))
        {
            if lessons.is_empty() && !has_access {
                crate::screen::warning(&format!(
                    "[{NAME}] Only extracting preview. For the full class, pass cookies from an account that has access. {}",
                    super::login::login_hint(None, None)
                ));
            }
            lessons.push(serde_json::json!({"video_id": preview_id}));
        }
        if lessons.is_empty() && !has_access {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: {}",
                super::login::login_required_message(
                    "You do not have access to this class",
                    None,
                    None
                )
            )));
        }
        let account_id = match video_data
            .get("accountId")
            .filter(|id| super::nuxt::truthy(id))
        {
            Some(id) => id_text(id)?,
            None => match video_js.get("data-account") {
                Some(id) => id_text(id)?,
                None => {
                    return Err(Error::Reported(format!(
                        "{video_id}: An extractor error has occurred. (caused by KeyError('data-account')); {}",
                        crate::BUG_REPORTS_MESSAGE
                    )));
                }
            },
        };
        let mut entries = Vec::new();
        for lesson in &lessons {
            let lesson_id = id_text(lesson.get("video_id").unwrap_or(&Value::Null))?;
            let title = match lesson.get("title") {
                None | Some(Value::Null) => None,
                Some(Value::String(title)) => Some(title.clone()),
                Some(_) => {
                    return Err(Error::UnsupportedHtml5(
                        "Craftsy lesson titles that are not strings are not supported natively"
                            .into(),
                    ));
                }
            };
            let mut entry = super::nrk::url_entry(
                &format!(
                    "http://players.brightcove.net/{account_id}/default_default/index.html?videoId={lesson_id}"
                ),
                "BrightcoveNew",
                Some(&lesson_id),
                None,
                title,
            )?;
            // `url_result(..., lesson['video_id'])` keeps the raw ID's type.
            if let Some(raw) = lesson.get("video_id")
                && let Some(marker) = entry.extra.get_mut(crate::TRANSPARENT)
            {
                marker["entry"]["id"] = raw.clone();
            }
            entries.push(entry);
        }
        let title = ["og:title", "twitter:title"]
            .iter()
            .find_map(|name| crate::html::html_search_meta(&webpage, name));
        if title.is_none() {
            super::search_warning(NAME, "og:title");
        }
        let description = ["og:description", "description"]
            .iter()
            .find_map(|name| crate::html::html_search_meta(&webpage, name))
            .map(Value::from);
        Ok(Some(super::nrk::playlist_result(
            url,
            NAME,
            video_id,
            title.filter(|title| !title.is_empty()),
            description,
            entries,
            true,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.craftsy.com/class/the-midnight-quilt-show-season-5/"
        ));
        assert!(!suitable("https://www.craftsy.com/pattern/x"));
    }
}
