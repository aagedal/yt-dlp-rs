//! Upstream `ListenNotesIE` (`yt_dlp/extractor/listennotes.py`): an episode
//! page's `original-content` JSON merged with its play button toolbar's
//! attributes, as one audio URL with the episode and channel metadata.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct ListenNotes;

const NAME: &str = "ListenNotes";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?listennotes\.com/podcasts/[^/]+/[^/]+-(?P<id>.+)/",
    )
    .unwrap()
});
static CONTENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<script id="original-content"[^>]+\btype="application/json">"#).unwrap()
});
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?P<duration>[\d:]+)\s*-\s*(?P<description>.+)").unwrap());
static BLOCKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(</?(div|p)>\s*)+").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("ListenNotes {what} are not supported natively"))
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// `_html_search_meta(names, webpage, display_name)` (not fatal): the first
/// name with a match, else upstream's warning and `None`.
fn search_meta(webpage: &str, names: &[&str], display_name: &str) -> Option<String> {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta_clean(webpage, name));
    if found.is_none() {
        crate::screen::warning(&format!(
            "[{NAME}] unable to extract {display_name}; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    found
}

/// `traverse_obj(data, *keys)`: the first key whose value is not `None`.
fn first_of<'a>(data: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| data.get(*key).filter(|value| !value.is_null()))
}

/// `traverse_obj(data, ('nlp_entities', ..., 'name'))`: every item's
/// non-`None` name (`{}` values dropped too), or `None` when there are none.
fn cast(data: &Map<String, Value>) -> Option<Value> {
    let items: Vec<&Value> = match data.get("nlp_entities") {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    let names: Vec<Value> = items
        .into_iter()
        .filter_map(|item| item.as_object()?.get("name"))
        .filter(|name| !name.is_null() && name.as_object().is_none_or(|map| !map.is_empty()))
        .cloned()
        .collect();
    (!names.is_empty()).then_some(Value::Array(names))
}

/// `parse_duration(value)`: only strings parse.
fn duration(value: Option<&str>) -> Value {
    match value.and_then(crate::parse_duration) {
        Some(crate::info::PyNumber::Int(seconds)) => json!(seconds),
        Some(crate::info::PyNumber::Float(seconds)) => json!(seconds),
        None => Value::Null,
    }
}

impl Extractor for ListenNotes {
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
        let audio_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &audio_id)?;
        let mut data = match super::search_json(&CONTENT, &webpage, false) {
            Some(Some(Value::Object(data))) => data,
            Some(Some(_)) => return Err(unsupported("content JSON that is not an object")),
            Some(None) => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {audio_id}: Unable to extract content - Failed to parse JSON"
                )));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {audio_id}: Unable to extract content; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let toolbar = crate::html_elements::elements_text_and_html_by_attribute_regex(
            r"[\w:.-]+",
            "id",
            "episode-play-button-toolbar|episode-no-play-button-toolbar",
            &webpage,
        )
        .map_err(|error| unsupported(&format!("toolbar elements ({error})")))?
        .into_iter()
        .next()
        .map(|(_, whole)| whole)
        .ok_or_else(|| unsupported("pages without a play button toolbar"))?;
        data.extend(crate::html_elements::extract_attributes(&toolbar));

        let meta = search_meta(
            &webpage,
            &["og:description", "description", "twitter:description"],
            "og:description",
        );
        let (meta_duration, meta_description) = match meta {
            Some(meta) => match DESCRIPTION.captures(&meta) {
                Some(found) => (
                    Some(found["duration"].to_owned()),
                    Some(found["description"].to_owned()),
                ),
                None => {
                    crate::screen::warning(&format!(
                        "[{NAME}] unable to extract description; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ));
                    (None, None)
                }
            },
            None => {
                crate::screen::warning(&format!(
                    "[{NAME}] unable to extract description; {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
                (None, None)
            }
        };

        let audio = match data.get("audio") {
            Some(Value::String(audio)) => audio.clone(),
            Some(_) => return Err(unsupported("non-string audio URLs")),
            None => {
                return Err(Error::Reported(format!(
                    "{audio_id}: An extractor error has occurred. (caused by KeyError('audio')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let title = match data.get("data-title").filter(|title| truthy(title)) {
            Some(Value::String(title)) => Some(title.clone()),
            Some(_) => return Err(unsupported("non-string titles")),
            None => crate::html_elements::element_text_and_html_by_tag("h1", &webpage)
                .ok()
                .map(|(text, _)| crate::html::clean_html(&text))
                .filter(|title| !title.is_empty())
                .or_else(|| {
                    search_meta(&webpage, &["og:title", "title", "twitter:title"], "title")
                }),
        };
        let description =
            crate::html_elements::elements_text_and_html_by_class("ln-text-p", &webpage)
                .ok()
                .and_then(|found| found.into_iter().next())
                .map(|(text, _)| crate::html::clean_html(&BLOCKS.replace_all(&text, "<br/><br/>")))
                .filter(|description| !description.is_empty())
                .or_else(|| meta_description.map(|description| description.trim().to_owned()));
        // `parse_duration(traverse_obj(data, 'audio_length', 'data-duration')
        // or duration)`.
        let duration = match first_of(&data, &["audio_length", "data-duration"]) {
            Some(value) if truthy(value) => duration(value.as_str()),
            _ => duration(meta_duration.as_deref()),
        };
        let episode_id = first_of(&data, &["uuid", "data-episode-uuid"])
            .cloned()
            .unwrap_or(Value::Null);

        let media = Url::parse(&audio).map_err(|_| unsupported("relative audio URLs"))?;
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut format: crate::VideoFormat =
            serde_json::from_value(json!({"format_id": "", "url": audio, "ext": ext}))
                .map_err(|error| Error::Reported(format!("[{NAME}] {audio_id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "url-only");
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", audio_id.into()),
            ("url", audio.into()),
            ("title", title.map_or(Value::Null, Value::from)),
            ("description", description.map_or(Value::Null, Value::from)),
            ("duration", duration),
            ("episode_id", episode_id),
        ];
        for (key, source) in [
            ("thumbnail", "data-image"),
            ("channel", "data-channel-title"),
            ("cast", ""),
            ("channel_url", "channel_url"),
            ("channel_id", "channel_short_uuid"),
        ] {
            let value = if key == "cast" {
                cast(&data)
            } else {
                data.get(source)
                    .filter(|value| {
                        !value.is_null() && value.as_object().is_none_or(|map| !map.is_empty())
                    })
                    .cloned()
            };
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("untyped-dict;{}", keys.join(","));
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            dict_info(url, (NAME, NAME), fields, vec![format]),
            &shape,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.listennotes.com/podcasts/thriving-on-overload/tim-oreilly-on-noticing-KrDgvNb_u1n/"
        ));
        assert!(!suitable(
            "https://www.listennotes.com/podcasts/thriving-on-overload/"
        ));
    }

    #[test]
    fn cast_follows_the_branching_traversal() {
        let data =
            json!({"nlp_entities": [{"name": "A"}, {"x": 1}, {"name": null}, "s", {"name": "B"}]});
        assert_eq!(cast(data.as_object().unwrap()), Some(json!(["A", "B"])));
        assert_eq!(cast(json!({"nlp_entities": []}).as_object().unwrap()), None);
    }
}
