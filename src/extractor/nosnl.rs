//! Upstream `NOSNLArticleIE` (`yt_dlp/extractor/nosnl.py`): a nos.nl article
//! or video page's Next.js `pageProps.data` as a playlist of its `video`
//! items (each HLS playlist, duration and ratio thumbnails) and `audio`
//! items (an MP3 URL), with the page's title, description, keywords,
//! timestamps, share image and categories.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NosNlArticle;

const NAME: &str = "NOSNLArticle";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://nos\.nl/(?P<type>video|(\w+/)?\w+)/?\d+-(?P<display_id>[\w-]+)").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `KeyError`s carry no ID: `_VALID_URL` has no `id` group, so
/// `get_temp_id` is `None`.
fn key_error(_id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("NOS {what} are not supported natively"))
}

/// `str(item['id'])` of a JSON string or integer.
fn id_text(item: &Value, display_id: &str) -> Result<String> {
    match item.get("id") {
        Some(Value::String(id)) => Ok(id.clone()),
        Some(Value::Number(id)) if id.is_i64() || id.is_u64() => Ok(id.to_string()),
        Some(_) => Err(unsupported("IDs that are not strings or integers")),
        None => Err(key_error(display_id, "id")),
    }
}

/// `item.get(key)` that must be a string or `None`.
fn text(item: &Value, key: &str) -> Result<Value> {
    match item.get(key) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::String(text)) => Ok(text.clone().into()),
        Some(_) => Err(unsupported(&format!("{key}s that are not strings"))),
    }
}

/// `_html_search_meta(names, webpage)`: the first name present.
fn search_meta(webpage: &str, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name))
}

fn parse_iso8601(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map_or(Value::Null, Value::from)
}

/// `[{url: traverse_obj(image, ('url', ...), get_all=False), width,
/// height} for image in traverse_obj(item, ('imagesByRatio', ...))[0]]`.
fn thumbnails(item: &Value) -> Result<Value> {
    let ratios: Vec<&Value> = match item.get("imagesByRatio") {
        Some(Value::Array(items)) => items.iter().filter(|item| !item.is_null()).collect(),
        Some(Value::Object(items)) => items.values().filter(|item| !item.is_null()).collect(),
        _ => Vec::new(),
    };
    let Some(first) = ratios.first() else {
        return Err(
            // An uncaught `IndexError` is reported as its message.
            Error::Reported("list index out of range".into()),
        );
    };
    let images: Vec<&Value> = match first {
        Value::Array(items) => items.iter().collect(),
        _ => return Err(unsupported("image ratio lists that are not lists")),
    };
    let mut out = Vec::new();
    for image in images {
        let Some(image) = image.as_object() else {
            return Err(unsupported("images that are not objects"));
        };
        let url = match image.get("url") {
            Some(Value::Object(urls)) => urls.values().find(|url| !url.is_null()).cloned(),
            Some(Value::Array(urls)) => urls.iter().find(|url| !url.is_null()).cloned(),
            _ => None,
        };
        let mut thumbnail = Map::new();
        thumbnail.insert("url".into(), url.unwrap_or(Value::Null));
        for key in ["width", "height"] {
            thumbnail.insert(key.into(), image.get(key).cloned().unwrap_or(Value::Null));
        }
        out.push(Value::Object(thumbnail));
    }
    Ok(Value::Array(out))
}

/// `_entries`.
fn entries(
    client: &Client,
    page: &Url,
    items: &[&Value],
    display_id: &str,
) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("video") => {
                let source = item
                    .get("source")
                    .and_then(|source| source.get("url"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| unsupported("videos without a source URL"))?;
                let manifest = Url::parse(source)
                    .map_err(|_| unsupported("video sources that are not URLs"))?;
                let stream =
                    extract_m3u8_fatal_master(client, &manifest, NAME, display_id, None, true)?;
                let id = id_text(item, display_id)?;
                let duration = item
                    .get("duration")
                    .and_then(Value::as_str)
                    .and_then(crate::parse_duration)
                    .map_or(Value::Null, |duration| json!(duration));
                let fields = vec![
                    ("id", Value::from(id)),
                    ("title", text(item, "title")?),
                    ("description", text(item, "description")?),
                    ("formats", Value::Null),
                    ("subtitles", Value::Null),
                    ("duration", duration),
                    ("thumbnails", thumbnails(item)?),
                ];
                let mut info = dict_info(page, (NAME, NAME), fields, stream.formats);
                info.subtitles = stream.subtitles;
                if let Some(Value::String(shape)) =
                    info.extra.get_mut(crate::info_order::INFO_SHAPE)
                {
                    shape.push_str(";thumbnail=url,width,height");
                }
                entries.push(info);
            }
            Some("audio") => {
                let id = id_text(item, display_id)?;
                let src = item
                    .get("media")
                    .and_then(|media| media.get("src"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| unsupported("audio items without a source"))?;
                let mut format: crate::VideoFormat =
                    serde_json::from_value(json!({"format_id": "", "url": src, "ext": "mp3"}))
                        .map_err(|error| Error::Reported(format!("{id}: {error}")))?;
                crate::info_order::mark_format(&mut format, "url-only");
                let fields = vec![
                    ("id", Value::from(id)),
                    ("title", text(item, "title")?),
                    ("url", src.into()),
                    ("ext", "mp3".into()),
                ];
                let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
                let shape = format!("untyped-dict;{}", keys.join(","));
                entries.push(crate::info_order::mark_info(
                    dict_info(page, (NAME, NAME), fields, vec![format]),
                    &shape,
                ));
            }
            _ => {}
        }
    }
    Ok(entries)
}

impl Extractor for NosNlArticle {
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
        let site_type = found["type"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        // Without a temporary ID the error has no `ID: ` prefix.
        let next_data = super::search_nextjs_data(&webpage, NAME, &display_id).map_err(
            |error| match error {
                Error::Reported(message) => Error::Reported(message.replacen(
                    &format!("[{NAME}] {display_id}: "),
                    &format!("[{NAME}] "),
                    1,
                )),
                error => error,
            },
        )?;
        let mut data = &next_data;
        for key in ["props", "pageProps", "data"] {
            data = data.get(key).ok_or_else(|| key_error(&display_id, key))?;
        }
        let items: Vec<&Value> = if site_type == "video" {
            vec![
                data.get("video")
                    .ok_or_else(|| key_error(&display_id, "video"))?,
            ]
        } else {
            match data.get("items") {
                Some(Value::Array(items)) => items.iter().collect(),
                Some(_) => return Err(unsupported("item lists that are not lists")),
                None => return Err(key_error(&display_id, "items")),
            }
        };
        let entries = entries(client, url, &items, &display_id)?;
        let id = id_text(data, &display_id)?;
        let or_meta = |key: &str, names: &[&str]| -> Result<Value> {
            match text(data, key)? {
                Value::String(text) if !text.is_empty() => Ok(text.into()),
                _ => Ok(search_meta(&webpage, names).map_or(Value::Null, Value::from)),
            }
        };
        let title = or_meta("title", &["title", "og:title", "twitter:title"])?;
        let description = or_meta(
            "description",
            &["description", "twitter:description", "og:description"],
        )?;
        let thumbnail = or_meta("shareImageSrc", &["og:image", "twitter:image"])?;
        let categories: Vec<Value> = match data.get("categories") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.get("label"))
                .filter(|label| !label.is_null())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let tags = data.get("keywords").cloned().unwrap_or(Value::Null);
        let fields = [
            ("title", title.clone()),
            ("description", description),
            ("tags", tags),
            ("modified_timestamp", parse_iso8601(data.get("modifiedAt"))),
            ("thumbnail", thumbnail),
            ("timestamp", parse_iso8601(data.get("publishedAt"))),
            ("categories", Value::Array(categories)),
        ];
        let mut extra = Map::new();
        let mut after = vec!["id"];
        if title.is_null() {
            extra.insert("title".into(), Value::Null);
        }
        for (key, value) in fields {
            if key == "title" {
                after.push(key);
                continue;
            }
            after.push(key);
            extra.insert(key.into(), value);
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;;after={}", after.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id,
            title: title.as_str().unwrap_or_default().to_owned(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://nos.nl/nieuwsuur/artikel/2440353-verzakking-door-droogte-dreigt-tot-een-miljoen-kwetsbare-huizen"
        ));
        assert!(suitable(
            "https://nos.nl/artikel/2440409-vannacht-sliepen-weer-enkele-honderden-asielzoekers-in-ter-apel-buiten"
        ));
        assert!(suitable(
            "https://nos.nl/video/2452718-xi-en-trudeau-botsen-voor-de-camera-op-g20-top-je-hebt-gelekt"
        ));
        assert!(!suitable("https://nos.nl/artikel/"));
    }
}
