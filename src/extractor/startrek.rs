//! Upstream `StarTrekIE` (`yt_dlp/extractor/startrek.py`): STAR TREK videos
//! from the page's Next.js `video.data`: a YouTube video as a `url` result,
//! else the legacy video URL with its title, subhead, category, description,
//! publication time, subtitles, poster and series.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct StarTrek;

const NAME: &str = "startrek";
const KEY: &str = "StarTrek";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?startrek\.com(?:/en-(?:ca|un))?/videos/(?P<id>[^/?#]+)",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `...` over a list's items or a dict's values.
fn each(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    }
}

/// `(KEY, ..., 'text', {clean_html}, any)`.
fn first_text(value: Option<&Value>) -> Option<String> {
    each(value)
        .into_iter()
        .filter_map(|item| item.get("text"))
        .find_map(|text| text.as_str().map(crate::html::clean_html))
}

fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

impl Extractor for StarTrek {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
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
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        let mut page_props = &next_data;
        for key in ["props", "pageProps"] {
            page_props = page_props
                .get(key)
                .ok_or_else(|| key_error(&video_id, key))?;
        }
        let mut video_data = page_props;
        for key in ["video", "data"] {
            video_data = video_data
                .get(key)
                .ok_or_else(|| key_error(&video_id, key))?;
        }
        if let Some(youtube_id) = video_data
            .get("youtube_video_id")
            .filter(|id| super::nuxt::truthy(id))
        {
            let youtube_id = match youtube_id {
                Value::String(id) => id.clone(),
                Value::Number(id) => id.to_string(),
                _ => {
                    return Err(Error::UnsupportedHtml5(
                        "STAR TREK YouTube IDs that are not strings are not supported natively"
                            .into(),
                    ));
                }
            };
            return Ok(Some(super::follow_url(url, youtube_id)));
        }
        let series_id = each(video_data.get("series_and_movies"))
            .into_iter()
            .filter_map(|item| item.pointer("/series_or_movie/slug"))
            .find_map(|slug| slug.as_str().map(str::to_owned));
        let series = page_props
            .pointer("/queried/header/tab3/slices")
            .map(|slices| {
                each(Some(slices))
                    .into_iter()
                    .flat_map(|slice| each(slice.get("items")))
                    .filter(|item| {
                        item.pointer("/link/slug").and_then(Value::as_str) == series_id.as_deref()
                            && item.pointer("/link/slug").is_some()
                    })
                    .find_map(|item| {
                        item.get("link_copy")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
            })
            .unwrap_or_default();
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.into()),
            ("series", optional(series)),
            ("series_id", optional(series_id)),
        ];
        // The traversal dict drops `None` and empty results.
        if let Some(title) = first_text(video_data.get("title")) {
            fields.push(("title", title.into()));
        }
        if let Some(alt_title) = first_text(video_data.get("subhead")) {
            fields.push(("alt_title", alt_title.into()));
        }
        // `(..., {str.upper}, filter, all)`: `all` keeps an empty list.
        let categories: Vec<Value> = video_data
            .pointer("/category/data/category_name")
            .and_then(Value::as_str)
            .map(str::to_uppercase)
            .filter(|category| !category.is_empty())
            .into_iter()
            .map(Value::from)
            .collect();
        fields.push(("categories", Value::Array(categories)));
        let description = each(video_data.get("slices"))
            .into_iter()
            .filter_map(|slice| slice.pointer("/primary/content"))
            .flat_map(|content| each(Some(content)))
            .filter_map(|item| item.get("text"))
            .find_map(|text| text.as_str().map(crate::html::clean_html));
        if let Some(description) = description {
            fields.push(("description", description.into()));
        }
        if let Some(timestamp) = video_data
            .get("published")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.push(("release_timestamp", timestamp.into()));
        }
        let subtitle = url_or_none(video_data.get("legacy_subtitle_file"));
        if subtitle.is_some() {
            fields.push(("subtitles", Value::Null));
        }
        if let Some(thumbnail) = url_or_none(video_data.pointer("/poster_frame/url")) {
            // `update_url(query=None)`.
            let thumbnail = match Url::parse(&thumbnail) {
                Ok(mut parsed) => {
                    parsed.set_query(None);
                    parsed.to_string()
                }
                Err(_) => thumbnail,
            };
            fields.push(("thumbnail", thumbnail.into()));
        }
        let media = url_or_none(video_data.get("legacy_video_url"));
        let mut formats = Vec::new();
        if let Some(media) = &media {
            fields.push(("url", media.clone().into()));
            let ext = super::direct::determine_ext(media)
                .unwrap_or_else(|| "unknown_video".into())
                .to_lowercase();
            let mut format: crate::VideoFormat =
                serde_json::from_value(json!({"format_id": "", "url": media, "ext": ext}))
                    .map_err(|error| Error::Reported(format!("[{NAME}] {error}")))?;
            crate::info_order::mark_format(&mut format, "url-only");
            formats.push(format);
        }
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        // Without a `title` key, processing adds the generic one after
        // `fulltitle`.
        let untitled = if keys.contains(&"title") {
            ""
        } else {
            "-untitled"
        };
        let shape = format!("untyped-dict;{}{untitled}", keys.join(","));
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        if let Some(subtitle) = subtitle {
            info.subtitles.insert(
                Some("en".into()),
                vec![
                    serde_json::from_value(json!({
                        "url": subtitle,
                        "ext": super::direct::determine_ext(&subtitle).unwrap_or_default(),
                    }))
                    .map_err(|error| Error::Reported(format!("[{NAME}] {error}")))?,
                ],
            );
        }
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info, &shape,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.startrek.com/en-un/videos/official-trailer-star-trek-lower-decks-season-4"
        ));
        assert!(suitable("https://www.startrek.com/videos/x"));
        assert!(!suitable("https://www.startrek.com/news/x"));
    }
}
