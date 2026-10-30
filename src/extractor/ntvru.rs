//! Upstream `NTVRuIE` (`yt_dlp/extractor/ntvru.py`): an ntv.ru article or
//! video page's video id (its `ya:ovs:feed_url` meta), the `vi<id>` player
//! XML (progressive files, HLS and DASH playback) and the `exp/video/<id>`
//! metadata XML that overrides the player's title, description and duration.
use super::{Extractor, dict_format, dict_info, download_webpage, download_xml_note};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NtvRu;

const NAME: &str = "ntv.ru";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?ntv\.ru/(?:[^/#?]+/)*(?P<id>[^/?#&]+)")
        .unwrap()
});
static FEED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<meta property="ya:ovs:feed_url" content="https?://www\.ntv\.ru/(?:exp/)?video/(\d+)"#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// The child elements of `node` named `name` (namespace-agnostic, like
/// `{*}name` and `xpath_text`).
fn children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| child.is_element() && child.tag_name().name() == name)
        .collect()
}

/// `xpath_text(node, path)`: the text of the first element at `path`.
fn xpath_text(node: roxmltree::Node<'_, '_>, path: &[&str]) -> Option<String> {
    let mut current = node;
    for name in path {
        current = *children(current, name).first()?;
    }
    let text: String = current
        .children()
        .filter(roxmltree::Node::is_text)
        .filter_map(|child| child.text())
        .collect();
    (!text.is_empty()).then_some(text)
}

/// Every `text()` of the elements at `path` (`path/text()`, `...`).
fn xpath_texts(node: roxmltree::Node<'_, '_>, path: &[&str]) -> Vec<String> {
    let mut current = vec![node];
    for name in path {
        current = current
            .iter()
            .flat_map(|parent| children(*parent, name))
            .collect();
    }
    current
        .iter()
        .flat_map(|element| element.children().filter(roxmltree::Node::is_text))
        .filter_map(|text| text.text().map(str::to_owned))
        .collect()
}

/// `int_or_none` on a text value.
fn int_or_none(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}

/// `url_or_none` on a text value.
fn url_or_none(text: &str) -> Option<String> {
    let text = text.trim();
    (Url::parse(text).is_ok() && !text.contains(char::is_whitespace)).then(|| text.to_owned())
}

/// Sets `key` in `fields` like a Python dict assignment (an existing key keeps
/// its position).
fn set(fields: &mut Vec<(&'static str, Value)>, key: &'static str, value: Value) {
    match fields.iter_mut().find(|(name, _)| *name == key) {
        Some((_, existing)) => *existing = value,
        None => fields.push((key, value)),
    }
}

impl Extractor for NtvRu {
    fn key(&self) -> &'static str {
        "NTVRu"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        // `_html_search_regex(..., 'video id')` (fatal).
        let video_id = FEED_URL
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract video id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;

        let player_url = Url::parse(&format!("http://www.ntv.ru/vi{video_id}/"))?;
        let player_xml = download_xml_note(
            client,
            &player_url,
            NAME,
            &video_id,
            "Downloading video XML",
        )?;
        let player_doc = roxmltree::Document::parse(&player_xml)
            .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        let player = player_doc.root_element();
        let video = children(player, "data")
            .into_iter()
            .flat_map(|data| children(data, "video"))
            .next();

        let mut formats = Vec::new();
        if let Some(video) = video {
            for (format_id, size_key) in [("", "size"), ("hi", "hisize"), ("webm", "webmsize")] {
                let file_key = format!("{format_id}file");
                let Some(video_url) =
                    xpath_text(video, &[file_key.as_str()]).and_then(|text| url_or_none(&text))
                else {
                    continue;
                };
                let filesize = xpath_text(video, &[size_key]).and_then(|text| int_or_none(&text));
                formats.push(dict_format(
                    vec![
                        ("url", video_url.into()),
                        ("filesize", filesize.map_or(Value::Null, Value::from)),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
            if let Some(hls) = xpath_text(video, &["playback", "hls"])
                && let Ok(manifest) = Url::parse(&hls)
                && let Some(stream) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("hls"),
                    false,
                    false,
                )
            {
                formats.extend(stream.formats);
            }
            if let Some(dash) = xpath_text(video, &["playback", "dash"])
                && let Ok(manifest) = Url::parse(&dash)
                && let Some(stream) = super::extract_mpd_optional_note(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("dash"),
                    "Downloading MPD manifest",
                )
            {
                formats.extend(stream.formats);
            }
        }

        // `traverse_obj(player, {...})`: the first truthy value of each field.
        let mut fields: Vec<(&'static str, Value)> =
            vec![("id", video_id.clone().into()), ("formats", Value::Null)];
        let first_text = |texts: Vec<String>| texts.into_iter().find(|text| !text.is_empty());
        let player_title = first_text(xpath_texts(player, &["data", "title"]))
            .map(|text| crate::html::unescape_html(&text));
        let player_description = first_text(xpath_texts(player, &["data", "description"]))
            .map(|text| crate::html::unescape_html(&text));
        let player_duration = xpath_texts(player, &["data", "video", "totaltime"])
            .iter()
            .find_map(|text| int_or_none(text));
        let player_views = xpath_texts(player, &["data", "video", "views"])
            .iter()
            .find_map(|text| int_or_none(text));
        // `url_or_none` takes protocol-relative URLs, which processing makes `http:`.
        let player_thumbnail = xpath_texts(player, &["data", "video", "splash"])
            .iter()
            .find_map(|text| {
                let text = text.trim();
                (text.starts_with("//") || Url::parse(text).is_ok())
                    .then(|| super::proto_relative_http(text))
            });
        for (key, value) in [
            ("title", player_title.map(Value::from)),
            ("description", player_description.map(Value::from)),
            ("duration", player_duration.map(Value::from)),
            ("view_count", player_views.map(Value::from)),
            ("thumbnail", player_thumbnail.map(Value::from)),
        ] {
            if let Some(value) = value {
                set(&mut fields, key, value);
            }
        }

        // `_download_xml(..., fatal=False)`: the metadata XML overrides the player's values.
        let metadata_url = Url::parse(&format!("https://www.ntv.ru/exp/video/{video_id}"))?;
        match download_xml_note(
            client,
            &metadata_url,
            NAME,
            &video_id,
            "Downloading XML metadata",
        ) {
            Ok(text) => {
                let doc = roxmltree::Document::parse(&text)
                    .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
                let root = doc.root_element();
                let metadata_title = first_text(xpath_texts(root, &["title"]))
                    .map(|text| crate::html::unescape_html(&text));
                let metadata_description = first_text(xpath_texts(root, &["description"]))
                    .map(|text| crate::html::unescape_html(&text));
                let metadata_duration = xpath_texts(root, &["duration"])
                    .iter()
                    .find_map(|text| int_or_none(text));
                let timestamp = |path: &str| {
                    xpath_texts(root, &[path])
                        .iter()
                        .find_map(|text| crate::dates::parse_iso8601(text))
                };
                let tags: Vec<String> = xpath_texts(root, &["tag"])
                    .iter()
                    .flat_map(|text| text.split(','))
                    .map(|tag| tag.trim().to_owned())
                    .filter(|tag| !tag.is_empty())
                    .collect();
                let metadata_views = xpath_texts(root, &["stats", "views_total"])
                    .iter()
                    .find_map(|text| int_or_none(text));
                let comments = xpath_texts(root, &["stats", "comments"])
                    .iter()
                    .find_map(|text| int_or_none(text));
                for (key, value) in [
                    ("title", metadata_title.map(Value::from)),
                    ("description", metadata_description.map(Value::from)),
                    ("duration", metadata_duration.map(Value::from)),
                    ("timestamp", timestamp("create_date").map(Value::from)),
                    (
                        "release_timestamp",
                        timestamp("upload_date").map(Value::from),
                    ),
                    (
                        "modified_timestamp",
                        timestamp("modify_date").map(Value::from),
                    ),
                    ("tags", (!tags.is_empty()).then(|| json!(tags))),
                    ("view_count", metadata_views.map(Value::from)),
                    ("comment_count", comments.map(Value::from)),
                ] {
                    if let Some(value) = value {
                        set(&mut fields, key, value);
                    }
                }
            }
            Err(_) => crate::screen::warning(&format!(
                "[{NAME}] {video_id}: Unable to download XML metadata"
            )),
        }

        Ok(Some(dict_info(url, (NAME, "NTVRu"), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.ntv.ru/peredacha/svoya_igra/m58980/o818800"
        ));
        assert!(suitable("http://www.ntv.ru/novosti/863142/"));
        assert!(suitable("https://www.ntv.ru/video/1797442"));
        assert!(!suitable("https://example.com/video/1797442"));
    }
}
