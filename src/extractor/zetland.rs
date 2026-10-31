//! Upstream `ZetlandDKArticleIE` (`yt_dlp/extractor/zetland.py`): Zetland
//! stories as their Next.js story audio files, with the story, page meta,
//! HTML meta and JSON-LD metadata merged in that order (`merge_dicts`).
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct ZetlandDkArticle;

const NAME: &str = "ZetlandDKArticle";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://www\.zetland\.dk/\w+/(?P<id>(?P<story_id>\w{8})-(?P<uploader_id>\w{8})-(?:\w{5}))",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Zetland {what} are not supported natively"))
}

/// `traverse_obj(value, (*keys))` through objects only; `None` and `{}`
/// results are dropped like upstream's.
fn path<'a>(value: Option<&'a Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .try_fold(value?, |value, key| value.get(*key))
        .filter(|value| !is_empty(value))
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Object(entries) => entries.is_empty(),
        _ => false,
    }
}

/// `{str}`.
fn string(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| value.is_string()).cloned()
}

/// `{url_or_none}`.
fn url_or_none(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map(Value::from)
}

/// `{unified_timestamp}`.
fn timestamp(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(|date| crate::dates::unified_timestamp(date, true))
        .map(Value::from)
}

/// The first of `paths` under `base` whose value passes `filter`
/// (`get_all=False` over a branch).
fn first(
    base: Option<&Value>,
    paths: &[&[&str]],
    filter: fn(Option<&Value>) -> Option<Value>,
) -> Option<Value> {
    paths.iter().find_map(|keys| filter(path(base, keys)))
}

/// `_html_search_meta(names, webpage)` (not fatal): the first name with a
/// match, else upstream's warning for the first name.
fn search_meta(webpage: &str, names: &[&str]) -> Value {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta_clean(webpage, name));
    if found.is_none() {
        crate::screen::warning(&format!(
            "[{NAME}] unable to extract {}; {}",
            names[0],
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    found.map_or(Value::Null, Value::from)
}

/// `merge_dicts(*dicts)`: a key's first non-`None` value, unless that is an
/// empty string and a later one is a string.
fn merge(merged: &mut Map<String, Value>, dict: Vec<(&str, Option<Value>)>) {
    for (key, value) in dict {
        let Some(value) = value.filter(|value| !value.is_null()) else {
            continue;
        };
        match merged.get(key) {
            None => {
                merged.insert(key.into(), value);
            }
            Some(Value::String(existing)) if existing.is_empty() && value.is_string() => {
                merged.insert(key.into(), value);
            }
            _ => {}
        }
    }
}

impl Extractor for ZetlandDkArticle {
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
        let display_id = found["id"].to_owned();
        let uploader_id = found["uploader_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let next_data = super::search_nextjs_data(&webpage, NAME, &display_id)?;
        let Value::Object(next_data) = next_data else {
            return Err(unsupported("Next.js data that are not objects"));
        };
        let props = next_data
            .get("props")
            .ok_or_else(|| key_error(&display_id, "props"))?;
        let Value::Object(props) = props else {
            return Err(unsupported("Next.js props that are not objects"));
        };
        let page_props = props
            .get("pageProps")
            .ok_or_else(|| key_error(&display_id, "pageProps"))?;
        let story = path(
            Some(page_props),
            &["initialState", "consume", "story", "story"],
        );

        let mut formats = Vec::new();
        let audio_files = match path(story, &["story_content", "meta", "audioFiles"]) {
            Some(Value::Array(items)) => items.iter().collect(),
            Some(Value::Object(items)) => items.values().collect(),
            _ => Vec::new(),
        };
        for audio in audio_files {
            if let Some(audio) = url_or_none(Some(audio)) {
                formats.push(dict_format(
                    vec![("url", audio), ("vcodec", "none".into())],
                    NAME,
                    &display_id,
                )?);
            }
        }

        let mut merged = Map::new();
        merge(
            &mut merged,
            vec![
                ("id", Some(display_id.as_str().into())),
                ("formats", Some(Value::Bool(true))),
                ("uploader_id", Some(uploader_id.into())),
            ],
        );
        merge(
            &mut merged,
            vec![
                (
                    "title",
                    first(
                        story,
                        &[&["story_content", "content", "title"], &["title"]],
                        string,
                    ),
                ),
                ("uploader", path(story, &["sharer", "name"]).cloned()),
                (
                    "uploader_id",
                    path(story, &["sharer", "sharer_id"]).cloned(),
                ),
                (
                    "description",
                    path(story, &["story_content", "content", "socialDescription"]).cloned(),
                ),
                (
                    "series_id",
                    path(story, &["story_content", "meta", "seriesId"]).cloned(),
                ),
                (
                    "release_timestamp",
                    timestamp(path(story, &["published_at"])),
                ),
                (
                    "modified_timestamp",
                    timestamp(path(story, &["revised_at"])),
                ),
            ],
        );
        let meta_info = path(Some(page_props), &["metaInfo"]);
        merge(
            &mut merged,
            vec![
                (
                    "title",
                    first(
                        meta_info,
                        &[
                            &["meta", "title"],
                            &["ld", "headline"],
                            &["og", "og:title"],
                            &["og", "twitter:title"],
                        ],
                        string,
                    ),
                ),
                (
                    "description",
                    first(
                        meta_info,
                        &[
                            &["meta", "description"],
                            &["ld", "description"],
                            &["og", "og:description"],
                            &["og", "twitter:description"],
                        ],
                        string,
                    ),
                ),
                (
                    "uploader",
                    first(
                        meta_info,
                        &[&["meta", "author"], &["ld", "author", "name"]],
                        string,
                    ),
                ),
                (
                    "uploader_url",
                    url_or_none(path(meta_info, &["ld", "author", "url"])),
                ),
                (
                    "thumbnail",
                    first(
                        meta_info,
                        &[
                            &["ld", "image"],
                            &["og", "og:image"],
                            &["og", "twitter:image"],
                        ],
                        url_or_none,
                    ),
                ),
                (
                    "modified_timestamp",
                    timestamp(path(meta_info, &["ld", "dateModified"])),
                ),
                (
                    "release_timestamp",
                    timestamp(path(meta_info, &["ld", "datePublished"])),
                ),
                (
                    "timestamp",
                    timestamp(path(meta_info, &["ld", "dateCreated"])),
                ),
            ],
        );
        let title = search_meta(&webpage, &["title", "og:title", "twitter:title"]);
        let description = search_meta(
            &webpage,
            &["description", "og:description", "twitter:description"],
        );
        let thumbnail = search_meta(&webpage, &["og:image", "twitter:image"]);
        let uploader = search_meta(&webpage, &["author"]);
        let published = search_meta(&webpage, &["article:published_time"]);
        merge(
            &mut merged,
            vec![
                ("title", Some(title)),
                ("description", Some(description)),
                ("thumbnail", Some(thumbnail)),
                ("uploader", Some(uploader)),
                ("release_timestamp", timestamp(Some(&published))),
            ],
        );
        // `_search_json_ld(webpage, display_id, fatal=False)`.
        if super::json_ld::yield_json_ld(&webpage).is_empty() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract JSON-LD ; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let json_ld = super::json_ld::search(&webpage);
        if ["url", "formats", "subtitles", "chapters"]
            .iter()
            .any(|key| json_ld.contains_key(*key))
        {
            return Err(unsupported("JSON-LD media and chapters"));
        }
        merge(
            &mut merged,
            json_ld
                .iter()
                .map(|(key, value)| (key.as_str(), Some(value.clone())))
                .collect(),
        );

        let mut fields = Vec::new();
        for (key, value) in &merged {
            match key.as_str() {
                "formats" => fields.push(("formats", Value::Null)),
                "title" if !value.is_string() => {
                    return Err(unsupported("titles that are not strings"));
                }
                _ => fields.push((key.as_str(), value.clone())),
            }
        }
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (NAME, NAME),
            fields,
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.zetland.dk/historie/sO9aq2MY-a81VP3BY-66e69?utm_source=instagram"
        ));
        assert!(!suitable("https://www.zetland.dk/historie/sO9aq2MY"));
    }
}
