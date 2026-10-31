//! Upstream `ZapiksIE` (`yt_dlp/extractor/zapiks.py`): a media page's embed
//! (followed when it is not a Zapiks page), the `video-responsive` player's
//! data (a media URL to follow, or its `sources`) and the page's metadata.
use super::{Extractor, dict_format, dict_info, download_webpage, follow_url};
use crate::html_elements::{
    element_text_and_html_by_tag, elements_text_and_html_by_class, extract_attributes,
};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Zapiks;

const NAME: &str = "Zapiks";

static VALID_URL: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        crate::extractor::url_regex(r"^https?://(?:www\.)?zapiks\.(?:com|fr)/(?P<id>[\w-]+)\.html")
            .unwrap(),
        crate::extractor::url_regex(
            r"^https?://(?:www\.)?zapiks\.fr/index\.php\?(?:[^#]+&)?media_id=(?P<id>\d+)",
        )
        .unwrap(),
    ]
});
static UPLOADER_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/pro(?:fil)?/([^/?#]+)/?$").unwrap());
static VIEW_COUNT_NOISE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:vues|views|\s+)").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.iter().any(|pattern| pattern.is_match(url))
}

/// `find_element(cls=class)` (its content) or `find_element(cls=class,
/// html=True)` (the whole element).
fn element_by_class(html: &str, class: &str, whole: bool) -> Option<String> {
    elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(text, element)| if whole { element } else { text })
}

/// `find_elements(cls=class, html=True)`.
fn elements_by_class(html: &str, class: &str) -> Vec<String> {
    elements_text_and_html_by_class(class, html)
        .unwrap_or_default()
        .into_iter()
        .map(|(_, element)| element)
        .collect()
}

/// `clean_html(text)`, then `filter` (a non-empty string).
fn cleaned(text: &str) -> Option<String> {
    Some(crate::html::clean_html(text)).filter(|text| !text.is_empty())
}

fn number_value(number: crate::info::PyNumber) -> Value {
    match number {
        crate::info::PyNumber::Int(value) => value.into(),
        crate::info::PyNumber::Float(value) => value.into(),
    }
}

/// `traverse_obj(data, ..., any)`: the first truthy item of a list (or a
/// dict's values).
fn first_truthy(data: Value) -> Option<Value> {
    let items: Vec<Value> = match data {
        Value::Array(items) => items,
        Value::Object(map) => map.into_values().collect(),
        _ => Vec::new(),
    };
    items.into_iter().find(super::nuxt::truthy)
}

impl Extractor for Zapiks {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        let display_id = VALID_URL
            .iter()
            .find_map(|pattern| pattern.captures(url.as_str()))
            .map(|found| found["id"].to_owned())
            .unwrap_or_default();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;

        // An embed that is not a Zapiks page is followed.
        let embed = element_by_class(&webpage, "embed-container", false).and_then(|container| {
            let (_, iframe) = element_text_and_html_by_tag("iframe", &container).ok()?;
            let src = extract_attributes(&iframe).get("src")?.as_str()?.to_owned();
            super::json_ld::url_or_none(&super::proto_relative_http(&src))
        });
        if let Some(embed) = embed
            && !suitable(&embed)
        {
            return Ok(Some(follow_url(url, embed)));
        }

        let video_responsive = element_by_class(&webpage, "video-responsive", true)
            .map(|element| extract_attributes(&element))
            .unwrap_or_default();
        if let Some(media_url) = video_responsive
            .get("data-media-url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            && url.path() == "/index.php"
        {
            return Ok(Some(follow_url(url, media_url)));
        }

        let data_playlist = match video_responsive
            .get("data-playlist")
            .and_then(Value::as_str)
        {
            Some(text) => {
                let parsed: Value = crate::json::from_str(text).map_err(|error| {
                    Error::Reported(format!(
                        "[{NAME}] {display_id}: Failed to parse JSON (caused by {error})"
                    ))
                })?;
                first_truthy(parsed)
            }
            None => None,
        };

        let mut formats = Vec::new();
        let sources = data_playlist
            .as_ref()
            .and_then(|playlist| playlist.get("sources"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for source in sources {
            let Some(file) = source
                .get("file")
                .and_then(Value::as_str)
                .filter(|file| super::json_ld::url_or_none(file).is_some())
            else {
                continue;
            };
            let format_id = source
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_owned);
            // `**parse_resolution(format_id)`: only the dimensions found.
            let (width, height) =
                super::html5::parse_resolution(format_id.as_deref().unwrap_or(""));
            let mut fields = vec![
                (
                    "format_id",
                    format_id.clone().map_or(Value::Null, Value::from),
                ),
                ("url", file.into()),
            ];
            if let Some(width) = width {
                fields.push(("width", width.into()));
            }
            if let Some(height) = height {
                fields.push(("height", height.into()));
            }
            formats.push(dict_format(fields, NAME, &display_id)?);
        }

        let duration = crate::parse_duration(
            &crate::html::html_search_meta_clean(&webpage, "duration").unwrap_or_default(),
        )
        .map_or(Value::Null, number_value);
        let timestamp = crate::html::html_search_meta_clean(&webpage, "uploadDate")
            .and_then(|date| crate::dates::unified_timestamp(&date, true))
            .map_or(Value::Null, Value::from);

        let mut fields: Vec<(&str, Value)> = vec![
            ("display_id", display_id.clone().into()),
            ("duration", duration),
            ("formats", Value::Null),
            ("timestamp", timestamp),
        ];
        if let Some(description) =
            element_by_class(&webpage, "description-text", false).and_then(|text| cleaned(&text))
        {
            fields.push(("description", description.into()));
        }
        let tags: Vec<Value> = elements_by_class(&webpage, "bs-label")
            .iter()
            .filter_map(|element| {
                extract_attributes(element)
                    .get("title")
                    .and_then(Value::as_str)
                    .and_then(cleaned)
            })
            .map(Value::from)
            .collect();
        if !tags.is_empty() {
            fields.push(("tags", tags.into()));
        }
        if let Some(views) = element_by_class(&webpage, "video-content-view-counter", false)
            .and_then(|text| {
                let cleaned_text = crate::html::clean_html(&text);
                let stripped = VIEW_COUNT_NOISE.replace_all(&cleaned_text, "").into_owned();
                stripped.parse::<i64>().ok()
            })
        {
            fields.push(("view_count", views.into()));
        }
        if let Some(user) = element_by_class(&webpage, "video-content-user-link", true) {
            if let Some(uploader) = cleaned(&user) {
                fields.push(("uploader", uploader.into()));
            }
            let href = extract_attributes(&user)
                .get("href")
                .and_then(Value::as_str)
                .and_then(|href| UPLOADER_ID.captures(href))
                .map(|found| found[1].to_owned());
            if let Some(uploader_id) = href {
                fields.push(("uploader_id", uploader_id.into()));
            }
        }
        if let Some(playlist) = &data_playlist {
            if let Some(media_id) = playlist.get("mediaid").filter(|value| !value.is_null()) {
                let id = match media_id {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                fields.push(("id", id.into()));
            }
            if let Some(title) = playlist
                .get("title")
                .and_then(Value::as_str)
                .and_then(cleaned)
            {
                fields.push(("title", title.into()));
            }
            if let Some(thumbnail) = playlist
                .get("image")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            {
                fields.push(("thumbnail", thumbnail.into()));
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
            "https://www.zapiks.fr/ep2s3-bon-appetit-eh-be-viva.html"
        ));
        assert!(suitable(
            "https://www.zapiks.com/live-fwt18-vallnord-arcalis-.html"
        ));
        assert!(suitable(
            "https://www.zapiks.fr/index.php?action=playerIframe&media_id=164049"
        ));
        assert!(!suitable("https://www.zapiks.fr/"));
    }
}
