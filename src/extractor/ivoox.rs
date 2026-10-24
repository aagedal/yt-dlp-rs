//! Upstream `IvooxIE` (`yt_dlp/extractor/ivoox.py`): the page's
//! `window.__NUXT__` audio data (`_search_nuxt_data`, non-fatal), the
//! non-fatal direct download API, and the page's `data-prm-*` attributes,
//! OpenGraph and JSON-LD metadata.
use super::nuxt::{Step, search_nuxt_data};
use super::{Extractor, download_json_note_headers, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Ivoox;

const NAME: &str = "Ivoox";

static VALID_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"^https?://(?:www\.)?ivoox\.com/(?:\w{2}/)?[^/?#]+_rf_(?P<id>[0-9]+)_1\.html")
            .unwrap(),
        Regex::new(r"^https?://go\.ivoox\.com/rf/(?P<id>[0-9]+)").unwrap(),
    ]
});

/// `_match_id` over both `_VALID_URL`s.
fn match_id(url: &str) -> Option<String> {
    VALID_URLS
        .iter()
        .find_map(|pattern| pattern.captures(url))
        .map(|found| found["id"].to_owned())
}

pub fn suitable(url: &str) -> bool {
    match_id(url).is_some()
}

/// `_html_search_regex(r'data-prm-NAME="([^"]+)"', webpage, default=None)`.
fn prm(webpage: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(r#"data-prm-{name}="([^"]+)""#)).expect("static pattern");
    pattern
        .captures(webpage)
        .map(|found| crate::html::clean_html(&found[1]))
}

/// A Python truthy string.
fn nonempty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// `_download_json(..., fatal=False)`: `None` after upstream's warning.
fn download_optional(client: &Client, api: &Url, id: &str, referer: &str) -> Result<Option<Value>> {
    match download_json_note_headers(
        client,
        api,
        NAME,
        id,
        "Fetching direct download link",
        &[("Referer", referer)],
    ) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(Error::Reported(message)) if message.contains("Failed to parse JSON") => {
            Err(Error::UnsupportedHtml5(
                "Ivoox download responses that are not JSON are not supported natively".into(),
            ))
        }
        Err(error) => Err(error),
    }
}

impl Extractor for Ivoox {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(media_id) = match_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &media_id)?;
        let data = search_nuxt_data(
            &webpage,
            NAME,
            Some(&media_id),
            "__NUXT__",
            false,
            &[
                Step::Key("data"),
                Step::Index(0),
                Step::Key("data"),
                Step::Key("audio"),
            ],
        )?;
        let api = Url::parse(&format!(
            "https://vcore-web.ivoox.com/v1/public/audios/{media_id}/download-url"
        ))?;
        let direct = download_optional(client, &api, &media_id, url.as_str())?;
        // A set: this port keeps first-seen order (Python's string hash
        // order is randomized per process).
        let mut paths: Vec<&str> = Vec::new();
        let candidates = [
            direct
                .as_ref()
                .and_then(|direct| direct.get("data"))
                .and_then(|data| nonempty_str(data.get("downloadUrl"))),
            nonempty_str(data.get("downloadUrl")),
            nonempty_str(data.get("mediaUrl")),
        ];
        for path in candidates.into_iter().flatten() {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        let base = Url::parse("https://ivoox.com")?;
        let mut formats = Vec::new();
        for path in paths {
            let Ok(f_url) = base.join(path) else {
                continue;
            };
            let ext = super::direct::determine_ext(f_url.as_str())
                .unwrap_or_else(|| "unknown_video".into());
            let mut format: crate::VideoFormat = serde_json::from_value(json!({
                "url": f_url.as_str(),
                "http_headers": {"Referer": url.as_str()},
                "ext": ext,
                "format_id": "",
            }))
            .map_err(|error| Error::Reported(format!("[{NAME}] {media_id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "dict:url,http_headers");
            formats.push(format);
        }
        if formats.len() > 1 {
            return Err(Error::UnsupportedHtml5(
                "Ivoox audios with several distinct download URLs are not supported natively"
                    .into(),
            ));
        }
        let mut fields: Map<String, Value> = Map::new();
        let opt = |value: Option<String>| value.map_or(Value::Null, Value::from);
        fields.insert("uploader".into(), opt(prm(&webpage, "author")));
        fields.insert(
            "timestamp".into(),
            prm(&webpage, "pubdate")
                .as_deref()
                .and_then(crate::dates::parse_iso8601)
                .map_or(Value::Null, Value::from),
        );
        fields.insert("channel".into(), opt(prm(&webpage, "podname")));
        fields.insert("title".into(), opt(prm(&webpage, "title")));
        fields.insert(
            "thumbnail".into(),
            opt(crate::html::og_search_property(&webpage, "image")),
        );
        fields.insert(
            "description".into(),
            opt(crate::html::og_search_property(&webpage, "description")),
        );
        for (key, value) in super::json_ld::search(&webpage) {
            fields.insert(key, value);
        }
        let mut traversed = Map::new();
        if let Some(title) = data.get("title").filter(|value| value.is_string()) {
            traversed.insert("title".into(), title.clone());
        }
        if let Some(description) = data.get("description").filter(|value| value.is_string()) {
            traversed.insert("description".into(), description.clone());
        }
        if let Some(image) = data
            .get("image")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            traversed.insert("thumbnail".into(), image.into());
        }
        if let Some(timestamp) = data
            .get("uploadDate")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601_space)
        {
            traversed.insert("timestamp".into(), timestamp.into());
        }
        let duration = super::streamable::int_or_none(data.get("duration"));
        if !duration.is_null() {
            traversed.insert("duration".into(), duration);
        }
        if let Some(Value::Array(tags)) = data.get("tags") {
            let names: Vec<Value> = tags
                .iter()
                .filter_map(|tag| tag.get("name"))
                .filter(|name| name.is_string())
                .cloned()
                .collect();
            if !names.is_empty() {
                traversed.insert("tags".into(), Value::Array(names));
            }
        } else if let Some(Value::Object(tags)) = data.get("tags") {
            let names: Vec<Value> = tags
                .values()
                .filter_map(|tag| tag.get("name"))
                .filter(|name| name.is_string())
                .cloned()
                .collect();
            if !names.is_empty() {
                traversed.insert("tags".into(), Value::Array(names));
            }
        }
        fields.extend(traversed);
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = media_id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        let mut keys = vec!["id".to_owned(), "formats".to_owned()];
        for (key, value) in &fields {
            keys.push(key.clone());
            match key.as_str() {
                "title" => match value.as_str() {
                    Some(title) => info.title = title.to_owned(),
                    None => {
                        info.extra
                            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
                    }
                },
                _ if value.is_null() => {}
                _ => crate::metadata_parser::set_field(&mut info, key, value.clone()),
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("dict;{}", keys.join(",")),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert_eq!(
            match_id("https://www.ivoox.com/dex-08x30-rostros-del-mal-los-asesinos-en-audios-mp3_rf_143594959_1.html")
                .as_deref(),
            Some("143594959")
        );
        assert!(suitable("https://go.ivoox.com/rf/143594959"));
        assert!(suitable(
            "https://www.ivoox.com/en/campodelgas-28-03-2025-audios-mp3_rf_144036942_1.html"
        ));
        assert!(!suitable("https://www.ivoox.com/podcast_sq_1_1.html"));
    }
}
