//! Upstream `MySpassIE` (`yt_dlp/extractor/myspass.py`): MySpass.de videos
//! from the video metadata XML (its `url_flv` is the only format, with the
//! path's oversized ID-multiplied numbers divided back by the video ID).
use super::{Extractor, dict_info, download_xml};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct MySpass;

const NAME: &str = "MySpass";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?myspass\.de/(?:[^/]+/)*(?P<id>\d+)/?[^/]*$").unwrap()
});
static PATH_NUMBERS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/myspass2009/\d+/(\d+)/(\d+)/(\d+)/").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MySpass {what} are not supported natively"))
}

/// `metadata.find(tag)`: a direct child element.
fn find<'a, 'i>(node: roxmltree::Node<'a, 'i>, tag: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == tag)
}

/// `element.text`: `None` when the element starts with a child or is empty.
fn text(node: roxmltree::Node<'_, '_>) -> Option<String> {
    let text: String = node
        .children()
        .take_while(|child| !child.is_element())
        .filter_map(|child| child.text())
        .collect();
    (!text.is_empty()).then_some(text)
}

/// `xpath_text(metadata, tag, name, fatal=True)`.
fn xpath_text_fatal(
    root: roxmltree::Node<'_, '_>,
    tag: &str,
    name: &str,
    id: &str,
) -> Result<String> {
    let error = |what: &str| {
        Error::Reported(format!(
            "[{NAME}] {id}: Could not find XML element{what} {name}; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    };
    let element = find(root, tag).ok_or_else(|| error(""))?;
    text(element).ok_or_else(|| error("'s text"))
}

/// `int_or_none(text)`: Python `int(str)` (surrounding whitespace and
/// single underscores between digits allowed).
fn int_or_none(text: Option<&str>) -> Option<i64> {
    let text = text?.trim();
    let digits = text.trim_start_matches(['+', '-']);
    if digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    text.replace('_', "").parse().ok()
}

impl Extractor for MySpass {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let metadata_url = Url::parse(&format!(
            "http://www.myspass.de/myspass/includes/apps/video/getvideometadataxml.php?id={video_id}"
        ))?;
        let xml = download_xml(client, &metadata_url, NAME, &video_id)?;
        let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}'))
            .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        let metadata = document.root_element();
        let field = |tag: &str| find(metadata, tag).and_then(text);
        let title = xpath_text_fatal(metadata, "title", "title", &video_id)?;
        let mut video_url = xpath_text_fatal(metadata, "url_flv", "download url", &video_id)?;
        let video_id_int: u128 = video_id
            .parse()
            .map_err(|_| unsupported("video IDs that are not ASCII integers"))?;
        if let Some(groups) = PATH_NUMBERS.captures(&video_url) {
            let groups: Vec<String> = (1..=3).map(|index| groups[index].to_owned()).collect();
            for group in groups {
                let group_int: u128 = group
                    .parse()
                    .map_err(|_| unsupported("URL path numbers that are not ASCII integers"))?;
                if group_int > video_id_int {
                    if video_id_int == 0 {
                        return Err(unsupported("zero video IDs"));
                    }
                    video_url = video_url.replace(&group, &(group_int / video_id_int).to_string());
                }
            }
        }
        let opt = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let duration = field("duration")
            .and_then(|duration| crate::parse_duration(&duration))
            .map_or(Value::Null, |duration| json!(duration));
        let int = |tag: &str| int_or_none(field(tag).as_deref()).map_or(Value::Null, Value::from);
        let fields = vec![
            ("id", Value::from(video_id.clone())),
            ("url", video_url.clone().into()),
            ("title", title.clone().into()),
            ("thumbnail", opt(field("imagePreview"))),
            ("description", opt(field("description"))),
            ("duration", duration),
            ("series", opt(field("format"))),
            ("season_number", int("season")),
            ("season_id", opt(field("season_id"))),
            ("episode", title.into()),
            ("episode_number", int("episode")),
        ];
        let ext = super::direct::determine_ext(&video_url)
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        let mut format: crate::VideoFormat =
            serde_json::from_value(json!({"format_id": "", "url": video_url, "ext": ext}))
                .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "url-only");
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("untyped-dict;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(
            dict_info(url, (NAME, NAME), fields, vec![format]),
            &shape,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.myspass.de/myspass/shows/tvshows/absolute-mehrheit/Absolute-Mehrheit-vom-17022013-Die-Highlights-Teil-2--/11741/"
        ));
        assert!(suitable(
            "https://www.myspass.de/channels/tv-total-raabigramm/17033/20831/"
        ));
        assert!(!suitable("https://www.myspass.de/shows/tvshows/tv-total/"));
    }

    #[test]
    fn int_or_none_follows_python_int() {
        assert_eq!(int_or_none(Some(" 2 ")), Some(2));
        assert_eq!(int_or_none(Some("1_9")), Some(19));
        assert_eq!(int_or_none(Some("2.0")), None);
        assert_eq!(int_or_none(Some("_1")), None);
    }
}
