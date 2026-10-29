//! Upstream `BpbIE` (`yt_dlp/extractor/bpb.py`): Bundeszentrale für
//! politische Bildung media pages. The `bpb-player` element's Vue attributes
//! give the sources (their quality from the file name) and poster; the
//! opening header and intro, the page's meta tags and JSON-LD keywords give
//! the rest of the metadata.
use super::Extractor;
use super::dict_format;
use super::dict_info;
use super::download_webpage;
use super::json_ld::{url_or_none, yield_json_ld};
use crate::html_elements::{
    element_text_and_html_by_tag, elements_text_and_html_by_class, extract_attributes,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Bpb;

const NAME: &str = "Bpb";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.|m\.)?bpb\.de/(?:[^/?#]+/)*(?P<id>\d+)(?:[/?#]|$)").unwrap()
});
/// `_TITLE_RE`, matched at the start of the opening header's content.
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<title>[^<]*)<[^>]+>(?P<series>[^<]*)").unwrap());
static PLAYER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(<bpb-player(?:"[^"]*?"|[^>])*>)"#).unwrap());
static NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[_-]([a-z]+)\.[\da-z]+(?:$|\?)").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_regex` (fatal) for the player element.
fn missing(id: &str, name: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `find_element(cls=...)`: the first such element's content.
fn element_by_class(class: &str, html: &str) -> Option<String> {
    elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(content, _)| content)
}

/// `find_element(tag=...)`: the first such element's content.
fn element_by_tag(tag: &str, html: &str) -> Option<String> {
    element_text_and_html_by_tag(tag, html)
        .ok()
        .map(|(content, _)| content)
}

/// `_html_search_meta(name, webpage)`, warning when it is missing.
fn meta(html: &str, name: &str) -> Option<String> {
    let found = crate::html::html_search_meta(html, name);
    if found.is_none() {
        super::search_warning(NAME, name);
    }
    found
}

/// `_process_source`: one `bpb-player` source as a format.
fn process_source(
    source: &Map<String, Value>,
    video_id: &str,
) -> Result<Option<crate::VideoFormat>> {
    let Some(url) = source
        .get("src")
        .and_then(Value::as_str)
        .and_then(url_or_none)
    else {
        return Ok(None);
    };
    let source_type = source
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let extension = super::direct::mimetype2ext_or(source_type, None);
    let note = NOTE.captures(&url).map(|found| found[1].to_owned());
    let vcodec = if source_type.starts_with("video") {
        Value::Null
    } else {
        Value::from("none")
    };
    let format_id = [Some(extension.as_str()), note.as_deref()]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let label = source
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (width, height) = super::html5::parse_resolution(label);
    let mut fields = vec![
        ("url", Value::from(url)),
        ("ext", Value::from(extension.as_str())),
        ("vcodec", vcodec),
        (
            "quality",
            Value::from(if note.as_deref() == Some("high") {
                10
            } else {
                0
            }),
        ),
        ("format_note", note.clone().map_or(Value::Null, Value::from)),
        ("format_id", Value::from(format_id)),
    ];
    // `**parse_resolution(label)` only gives the sizes it found.
    if let Some(width) = width {
        fields.push(("width", width.into()));
    }
    if let Some(height) = height {
        fields.push(("height", height.into()));
    }
    Ok(Some(dict_format(fields, NAME, video_id)?))
}

impl Extractor for Bpb {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = found["id"].to_owned();
        let webpage = download_webpage(client, url, NAME, &video_id)?;

        let title_result = element_by_class("opening-header__title", &webpage)
            .and_then(|content| {
                TITLE.captures(&content).map(|found| {
                    let text = |group: &str| {
                        found
                            .name(group)
                            .map(|value| value.as_str().trim().to_owned())
                            .filter(|value| !value.is_empty())
                    };
                    (text("title"), text("series"))
                })
            })
            .unwrap_or_default();
        let json_lds = yield_json_ld(&webpage);

        // `join_nonempty(*traverse_obj(webpage, ...), delim='\n\n')`: the
        // opening intro, then the first accordion item's text, each cleaned.
        let intro = element_by_class("opening-intro", &webpage);
        let accordion_text = element_by_tag("bpb-accordion-item", &webpage)
            .and_then(|item| element_by_class("text-content", &item));
        let description: Vec<String> = [intro, accordion_text]
            .into_iter()
            .flatten()
            .map(|content| crate::html::clean_html(&content))
            .filter(|text| !text.is_empty())
            .collect();

        let mut fields: Vec<(&str, Value)> = vec![
            ("id", Value::from(video_id.as_str())),
            ("title", title_result.0.map_or(Value::Null, Value::from)),
            ("series", title_result.1.map_or(Value::Null, Value::from)),
            (
                "description",
                if description.is_empty() {
                    Value::Null
                } else {
                    Value::from(description.join("\n\n"))
                },
            ),
        ];
        // `traverse_obj(self._html_search_meta('author', webpage), all)`.
        let creators = meta(&webpage, "author").map(|author| vec![Value::from(author)]);
        fields.push(("creators", creators.map_or(Value::Null, Value::Array)));
        let uploader = meta(&webpage, "publisher");
        fields.push(("uploader", uploader.map_or(Value::Null, Value::from)));
        let release_date =
            meta(&webpage, "date").and_then(|date| crate::dates::unified_strdate(&date, false));
        fields.push((
            "release_date",
            release_date.map_or(Value::Null, Value::from),
        ));
        let tags: Vec<Value> = json_lds
            .iter()
            .filter_map(|entry| entry.get("keywords").and_then(Value::as_str))
            .flat_map(|keywords| keywords.split(',').map(Value::from).collect::<Vec<_>>())
            .collect();
        fields.push(("tags", Value::Array(tags)));

        // `_parse_vue_attributes('bpb-player', webpage, video_id)` (fatal).
        let player = PLAYER
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| missing(&video_id, "bpb-player"))?;
        let attributes = extract_attributes(&player);
        let mut sources = Value::Null;
        let mut poster = Value::Null;
        for (key, value) in &attributes {
            match key.as_str() {
                ":sources" => {
                    sources = value
                        .as_str()
                        .and_then(|text| {
                            serde_json::from_str::<Value>(&crate::js_to_json::js_to_json(text)).ok()
                        })
                        .unwrap_or(Value::Null);
                }
                "poster" => poster = value.clone(),
                _ => {}
            }
        }
        let mut formats = Vec::new();
        if let Some(sources) = sources.as_array() {
            for source in sources.iter().filter_map(Value::as_object) {
                if let Some(format) = process_source(source, &video_id)? {
                    formats.push(format);
                }
            }
        }
        fields.push(("formats", Value::Null));
        if let Some(poster) = poster.as_str().filter(|poster| !poster.is_empty()) {
            fields.push((
                "thumbnail",
                url.join(poster)
                    .map_or(Value::Null, |thumbnail| Value::from(thumbnail.to_string())),
            ));
        }
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_match_upstream_valid_url() {
        assert!(suitable(
            "http://www.bpb.de/mediathek/297/joachim-gauck-zu-1989-und-die-erinnerung-an-die-ddr"
        ));
        assert!(suitable(
            "https://www.bpb.de/mediathek/video/522184/krieg-flucht-und-falschmeldungen-wirstattdesinformation-2/"
        ));
        assert!(suitable(
            "https://m.bpb.de/517806/die-weltanschauung-der-neuen-rechten/"
        ));
        assert!(!suitable("https://www.bpb.de/mediathek/video/abc/"));
        assert!(!suitable("https://example.com/mediathek/297/"));
    }
}
