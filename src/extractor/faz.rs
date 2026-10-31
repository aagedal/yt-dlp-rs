//! Upstream `FazIE` (`yt_dlp/extractor/faz.py`): faz.net videos from the
//! page's `data-videojs-media` XML configuration (its `LOW`, `HIGH` and `HQ`
//! encodings by quality), with the OpenGraph title and description.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Faz;

const NAME: &str = "faz.net";
const KEY: &str = "Faz";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?faz\.net/(?:[^/]+/)*.*?-(?P<id>\d+)\.html")
        .unwrap()
});
static MEDIA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"data-videojs-media='([^']+)").unwrap());
static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)x(\d+)_(\d+)\.mp4").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn regex_error(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `element.find(path)` through direct children.
fn find<'a, 'i>(node: roxmltree::Node<'a, 'i>, path: &str) -> Option<roxmltree::Node<'a, 'i>> {
    path.split('/').try_fold(node, |node, tag| {
        node.children()
            .find(|child| child.is_element() && child.tag_name().name() == tag)
    })
}

/// `xpath_text(node, path)`: the element's leading text.
fn text(node: roxmltree::Node<'_, '_>, path: &str) -> Option<String> {
    let node = find(node, path)?;
    let text: String = node
        .children()
        .take_while(|child| !child.is_element())
        .filter_map(|child| child.text())
        .collect();
    (!text.is_empty()).then_some(text)
}

/// Python `int(text)` (surrounding whitespace allowed).
fn int_or_none(text: &str) -> Option<i64> {
    text.trim().replace('_', "").parse().ok()
}

impl Extractor for Faz {
    fn key(&self) -> &'static str {
        KEY
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
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `_og_search_description(webpage)` (non-fatal).
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract OpenGraph description; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let media = MEDIA
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .ok_or_else(|| regex_error(&video_id, "media"))?;
        if media == "extern" {
            return Err(Error::UnsupportedHtml5(
                "faz.net Perform Group players are not supported natively".into(),
            ));
        }
        let document = roxmltree::Document::parse(&media)
            .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        let config = document.root_element();
        let encodings = find(config, "ENCODINGS").ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Could not find XML element encodings; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let mut formats = Vec::new();
        for (pref, code) in ["LOW", "HIGH", "HQ"].into_iter().enumerate() {
            let Some(encoding) = find(encodings, code) else {
                continue;
            };
            let Some(encoding_url) = text(encoding, "FILENAME") else {
                continue;
            };
            let mut tbr = text(encoding, "AVERAGEBITRATE")
                .and_then(|tbr| int_or_none(&tbr.replace(',', ".")));
            let mut fields = vec![
                ("url", Value::from(encoding_url.clone())),
                ("format_id", code.to_lowercase().into()),
                ("quality", Value::from(pref as i64)),
                ("tbr", tbr.map_or(Value::Null, Value::from)),
                (
                    "vcodec",
                    text(encoding, "CODEC").map_or(Value::Null, Value::from),
                ),
            ];
            if let Some(size) = SIZE.captures(&encoding_url) {
                tbr = tbr.filter(|tbr| *tbr != 0).or_else(|| size[3].parse().ok());
                fields[3].1 = tbr.map_or(Value::Null, Value::from);
                fields.push((
                    "width",
                    size[1].parse::<i64>().map_or(Value::Null, Value::from),
                ));
                fields.push((
                    "height",
                    size[2].parse::<i64>().map_or(Value::Null, Value::from),
                ));
            }
            let mut format = dict_format(fields, NAME, &video_id)?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
        // `_og_search_title(webpage)` (non-fatal).
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract OpenGraph title; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", title.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
                (
                    "description",
                    description
                        .filter(|description| !description.is_empty())
                        .map_or(Value::Null, |description| description.trim().into()),
                ),
                (
                    "thumbnail",
                    text(config, "STILL/STILL_BIG").map_or(Value::Null, Value::from),
                ),
                (
                    "duration",
                    text(config, "DURATION")
                        .and_then(|duration| int_or_none(&duration))
                        .map_or(Value::Null, Value::from),
                ),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "http://www.faz.net/multimedia/videos/stockholm-chemie-nobelpreis-fuer-drei-amerikanische-forscher-12610585.html",
            "http://www.faz.net/-13659345.html",
            "http://www.faz.net/aktuell/politik/-13659345.html",
        ] {
            assert!(suitable(url), "{url}");
        }
        assert!(!suitable("http://www.faz.net/aktuell/"));
    }
}
