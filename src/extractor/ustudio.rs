//! Upstream `UstudioIE` and `UstudioEmbedIE` (`yt_dlp/extractor/ustudio.py`):
//! a video's `v1.ustudio.com` config XML qualities with its page metadata,
//! and an embed's `app.ustudio.com` config JSON transcodes.
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_json, download_webpage, download_xml};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Ustudio;
pub struct UstudioEmbed;

const NAME: &str = "ustudio";
const KEY: &str = "Ustudio";
const EMBED_NAME: &str = "ustudio:embed";
const EMBED_KEY: &str = "UstudioEmbed";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www|v1)\.)?ustudio\.com/video/(?P<id>[^/]+)/(?P<display_id>[^/?#&]+)",
    )
    .unwrap()
});
static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:app|embed)\.)?ustudio\.com/embed/(?P<uid>[^/]+)/(?P<id>[^/]+)")
        .unwrap()
});
static UPLOAD_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)Uploaded by\s*.+?\s*on\s*<span>([^<]+)</span>").unwrap());
static UPLOADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Uploaded by\s*<a[^>]*>([^<]+)<").unwrap());

/// `UstudioIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `UstudioEmbedIE._match_valid_url`.
pub fn embed_suitable(url: &str) -> bool {
    EMBED_URL.is_match(url)
}

fn begin(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// A non-fatal `_search_regex`.
fn search(regex: &Regex, webpage: &str, name: &str) -> Option<String> {
    let found = regex.captures(webpage).map(|found| found[1].to_owned());
    if found.is_none() {
        super::search_warning(NAME, name);
    }
    found
}

impl Extractor for Ustudio {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        begin(NAME, url);
        let config = Url::parse(&format!(
            "http://v1.ustudio.com/embed/{id}/ustudio/config.xml"
        ))?;
        let text = download_xml(client, &config, NAME, &display_id)?;
        let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|error| Error::Reported(format!("[{NAME}] {display_id}: {error}")))?;
        // `config.findall(f'./qualities/quality/{kind}')` with a `url`.
        let extract = |kind: &str| -> Vec<Value> {
            document
                .root_element()
                .children()
                .filter(|node| node.has_tag_name("qualities"))
                .flat_map(|node| node.children().filter(|node| node.has_tag_name("quality")))
                .flat_map(|node| node.children().filter(move |node| node.has_tag_name(kind)))
                .filter_map(|item| {
                    let item_url = item.attribute("url").filter(|url| !url.is_empty())?;
                    let number =
                        |name: &str| int_or_none(item.attribute(name).map(Value::from).as_ref());
                    Some(json!({
                        "url": crate::html::unescape_html(item_url),
                        "width": number("width"),
                        "height": number("height"),
                    }))
                })
                .collect()
        };
        let formats = extract("video")
            .into_iter()
            .map(|format| {
                dict_format(
                    vec![
                        ("url", format["url"].clone()),
                        ("width", format["width"].clone()),
                        ("height", format["height"].clone()),
                    ],
                    NAME,
                    &id,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let thumbnails = extract("image");

        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            super::search_warning(NAME, "OpenGraph title");
        }
        let upload_date = search(&UPLOAD_DATE, &webpage, "upload date")
            .and_then(|date| crate::dates::unified_strdate(&date, true))
            .map_or(Value::Null, Value::from);
        let uploader = search(&UPLOADER, &webpage, "uploader").map_or(Value::Null, Value::from);
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            super::search_warning(NAME, "OpenGraph description");
        }
        let mut fields = vec![
            ("id", Value::from(id.clone())),
            ("display_id", display_id.into()),
        ];
        if let Some(title) = title {
            fields.push(("title", title.into()));
        }
        fields.extend([
            ("description", description.map_or(Value::Null, Value::from)),
            ("thumbnails", Value::Array(thumbnails)),
            ("upload_date", upload_date),
            ("uploader", uploader),
            ("formats", Value::Null),
        ]);
        let info = dict_info(url, (NAME, KEY), fields, formats);
        Ok(Some(with_thumbnail_shape(info, "url,width,height")))
    }
}

/// The thumbnails' dict key order.
fn with_thumbnail_shape(mut info: VideoInfo, keys: &str) -> VideoInfo {
    if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
        shape.push_str(&format!(";thumbnail={keys}"));
    }
    info
}

impl Extractor for UstudioEmbed {
    fn key(&self) -> &'static str {
        EMBED_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = EMBED_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let uploader_id = found["uid"].to_owned();
        let id = found["id"].to_owned();
        begin(EMBED_NAME, url);
        let config = Url::parse(&format!(
            "http://app.ustudio.com/embed/{uploader_id}/{id}/config.json"
        ))?;
        let config = download_json(client, &config, EMBED_NAME, &id)?;
        let video_data = config
            .get("videos")
            .ok_or_else(|| super::clyp::key_error(&id, "videos"))?
            .get(0)
            .cloned()
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "uStudio embed configs without a first video are not supported natively".into(),
                )
            })?;
        let title = video_data
            .get("name")
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&id, "name"))?;
        if !title.is_string() {
            return Err(Error::UnsupportedHtml5(
                "uStudio embed titles that are not strings are not supported natively".into(),
            ));
        }

        let mut formats = Vec::new();
        if let Some(transcodes) = video_data.get("transcodes").and_then(Value::as_object) {
            for (ext, qualities) in transcodes {
                for quality in qualities.as_array().into_iter().flatten() {
                    let Some(quality_url) = quality
                        .get("url")
                        .filter(|value| super::nuxt::truthy(value))
                    else {
                        continue;
                    };
                    let height = int_or_none(quality.get("height"));
                    let format_id = match height.as_i64() {
                        Some(height) if height != 0 => format!("{ext}-{height}p"),
                        _ => ext.clone(),
                    };
                    formats.push(dict_format(
                        vec![
                            ("format_id", format_id.into()),
                            ("url", quality_url.clone()),
                            ("width", int_or_none(quality.get("width"))),
                            ("height", height),
                        ],
                        EMBED_NAME,
                        &id,
                    )?);
                }
            }
        }
        let thumbnails: Vec<Value> = video_data
            .get("images")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|image| {
                image
                    .get("url")
                    .filter(|value| super::nuxt::truthy(value))
                    .map(|image_url| json!({"url": image_url}))
            })
            .collect();
        let mut fields = vec![
            ("id", id.clone().into()),
            ("title", title),
            (
                "description",
                video_data
                    .get("description")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            ("duration", int_or_none(video_data.get("duration"))),
            ("uploader_id", uploader_id.into()),
            (
                "tags",
                video_data.get("keywords").cloned().unwrap_or(Value::Null),
            ),
            ("thumbnails", Value::Array(thumbnails)),
            ("formats", Value::Null),
        ];
        // Processing drops a `None` duration.
        fields.retain(|(key, value)| *key != "duration" || !value.is_null());
        let info = dict_info(url, (EMBED_NAME, EMBED_KEY), fields, formats);
        Ok(Some(with_thumbnail_shape(info, "url")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://ustudio.com/video/Uxu2my9bgSph/san_francisco_golden_gate_bridge"
        ));
        assert!(!suitable("http://ustudio.com/video/Uxu2my9bgSph"));
        assert!(embed_suitable(
            "http://app.ustudio.com/embed/DeN7VdYRDKhP/Uw7G1kMCe65T"
        ));
        assert!(!embed_suitable("http://app.ustudio.com/embed/DeN7VdYRDKhP"));
    }
}
