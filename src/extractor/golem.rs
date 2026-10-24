//! Upstream `GolemIE` (`yt_dlp/extractor/golem.py`): the video's XML
//! config, one format per child element with a URL and its teaser
//! thumbnails.
use super::{Extractor, download_xml};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Golem;

const NAME: &str = "Golem";
const PREFIX: &str = "http://video.golem.de";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://video\.golem\.de/.+?/(?P<id>.+?)/").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python's `repr` of an optional string.
fn repr(value: Option<&str>) -> String {
    match value {
        None => "None".into(),
        Some(text) if text.contains('\'') && !text.contains('"') => format!("\"{text}\""),
        Some(text) => format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'")),
    }
}

/// `self._int(v, name)`: a warning when the value is not an int.
fn int(value: Option<&str>, name: &str) -> Value {
    match value.and_then(|text| text.trim().parse::<i64>().ok()) {
        Some(number) => number.into(),
        None => {
            crate::screen::warning(&format!(
                "[{NAME}] Failed to extract {name}: Could not parse value {}",
                repr(value)
            ));
            Value::Null
        }
    }
}

/// `self._float(v, name)`.
fn float(value: Option<&str>, name: &str) -> Value {
    match value.and_then(|text| text.trim().parse::<f64>().ok()) {
        Some(number) => json!(number),
        None => {
            crate::screen::warning(&format!(
                "[{NAME}] Failed to extract {name}: Could not parse value {}",
                repr(value)
            ));
            Value::Null
        }
    }
}

/// ElementTree's `.text`: the text before the first child element.
fn text(node: roxmltree::Node<'_, '_>) -> String {
    node.children()
        .take_while(|child| !child.is_element())
        .filter_map(|child| child.text())
        .collect()
}

/// `element.findtext('./TAG')`: `None` without such a child.
fn find_text(node: roxmltree::Node<'_, '_>, tag: &str) -> Option<String> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == tag)
        .map(text)
}

/// `urllib.parse.urljoin(_PREFIX, url)`.
fn join(url: &str) -> Result<Url> {
    Ok(Url::parse(PREFIX)?.join(url)?)
}

impl Extractor for Golem {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let config = Url::parse(&format!("https://video.golem.de/xml/{id}.xml"))?;
        let xml = download_xml(client, &config, NAME, &id)?;
        let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}'))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
        let root = document.root_element();
        let title = find_text(root, "title").unwrap_or_else(|| "golem".into());
        let duration = float(find_text(root, "playtime").as_deref(), "duration");
        let mut formats = Vec::new();
        for element in root.children().filter(roxmltree::Node::is_element) {
            let Some(media) = find_text(element, "url").filter(|media| !media.is_empty()) else {
                continue;
            };
            let mut map = Map::new();
            map.insert("format_id".into(), element.tag_name().name().into());
            map.insert("url".into(), join(&media)?.to_string().into());
            map.insert("height".into(), int(element.attribute("height"), "height"));
            map.insert("width".into(), int(element.attribute("width"), "width"));
            map.insert(
                "filesize".into(),
                int(find_text(element, "filesize").as_deref(), "filesize"),
            );
            let ext = find_text(element, "filename")
                .as_deref()
                .and_then(super::direct::determine_ext)
                .unwrap_or_else(|| "unknown_video".into());
            map.insert("ext".into(), ext.into());
            let keys: Vec<String> = map.keys().cloned().collect();
            map.retain(|_, value| !value.is_null());
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            formats.push(format);
        }
        let mut thumbnails = Vec::new();
        for element in root
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "teaser")
        {
            let Some(thumbnail) = find_text(element, "url").filter(|media| !media.is_empty())
            else {
                continue;
            };
            thumbnails.push(json!({
                "url": join(&thumbnail)?.to_string(),
                "width": int(element.attribute("width"), "thumbnail width"),
                "height": int(element.attribute("height"), "thumbnail height"),
            }));
        }
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        if formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        // `No video formats found!` comes before the generic title.
        let generic_title = title.is_empty() && !formats.is_empty();
        info.formats = formats;
        info.id = id;
        if generic_title {
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        // Processing drops a `None` duration.
        let keys = if duration.is_null() {
            "id,title,formats,thumbnails"
        } else {
            "id,title,duration,formats,thumbnails"
        };
        if !duration.is_null() {
            crate::metadata_parser::set_field(&mut info, "duration", duration);
        }
        crate::metadata_parser::set_field(&mut info, "thumbnails", Value::Array(thumbnails));
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("dict;{keys};thumbnail=url,width,height"),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://video.golem.de/handy/14095/iphone-6-und-6-plus-test.html"
        ));
        assert!(!suitable("http://video.golem.de/handy/14095"));
        assert!(!suitable("http://www.golem.de/handy/14095/x.html"));
    }

    #[test]
    fn python_reprs() {
        assert_eq!(repr(None), "None");
        assert_eq!(repr(Some("a b")), "'a b'");
        assert_eq!(repr(Some("it's")), "\"it's\"");
    }
}
