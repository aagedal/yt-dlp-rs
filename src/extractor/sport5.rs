//! Upstream `Sport5IE` (`yt_dlp/extractor/sport5.py`): Sport5 clips from the
//! page's `clipId` and the clip's metadata XML: its error, title,
//! description, duration, posters, categories and MP4 playback links.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Sport5;

const NAME: &str = "Sport5";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www|vod)?\.sport5\.co\.il/.*\b(?:Vi|docID)=(?P<id>\d+)").unwrap()
});
static CLIP_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"clipId=([\w-]+)").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Sport5 {what} are not supported natively"))
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, tag: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == tag)
}

fn children<'a, 'i>(
    node: roxmltree::Node<'a, 'i>,
    tag: &'a str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> + 'a {
    node.children()
        .filter(move |child| child.is_element() && child.tag_name().name() == tag)
}

/// `element.text` (`None` for an empty element).
fn text(node: roxmltree::Node<'_, '_>) -> Value {
    node.text()
        .filter(|text| !text.is_empty())
        .map_or(Value::Null, Value::from)
}

/// Python's `int(element.get(key))`.
fn int_attr(node: roxmltree::Node<'_, '_>, key: &str) -> Result<i64> {
    node.attribute(key)
        .and_then(|value| value.trim().parse().ok())
        .ok_or_else(|| unsupported("attributes that are not integers"))
}

impl Extractor for Sport5 {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let media_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &media_id)?;
        let video_id = CLIP_ID
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {media_id}: Unable to extract video id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let api = Url::parse(&format!(
            "http://sport5-metadata-rr-d.nsacdn.com/vod/vod/{video_id}/HDS/metadata.xml"
        ))?;
        let xml = super::download_xml(client, &api, NAME, &video_id)?;
        let document = roxmltree::Document::parse(&xml)
            .map_err(|_| unsupported("metadata that are not XML"))?;
        let root = document.root_element();
        if let Some(error) = child(root, "Error") {
            let part = |tag: &str| {
                child(error, tag)
                    .map(|node| node.text().unwrap_or("None").to_owned())
                    .ok_or_else(|| unsupported("errors without a name or description"))
            };
            return Err(Error::Reported(format!(
                "[{NAME}] {media_id}: {NAME} returned error: {} - {}",
                part("Name")?,
                part("Description")?
            )));
        }
        let required = |tag: &str| {
            child(root, tag).ok_or_else(|| unsupported(&format!("metadata without {tag}")))
        };
        let title = text(required("Title")?);
        let description = text(required("Description")?);
        let duration = required("Duration")?
            .text()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .ok_or_else(|| unsupported("durations that are not integers"))?;
        let mut thumbnails = Vec::new();
        if let Some(posters) = child(root, "PosterLinks") {
            for poster in children(posters, "PosterIMG") {
                thumbnails.push(json!({
                    "url": text(poster),
                    "width": int_attr(poster, "width")?,
                    "height": int_attr(poster, "height")?,
                }));
            }
        }
        let categories: Vec<Value> = child(root, "Categories")
            .map(|categories| {
                children(categories, "Category")
                    .map(|category| category.attribute("name").map_or(Value::Null, Value::from))
                    .collect()
            })
            .unwrap_or_default();
        let mut formats = Vec::new();
        if let Some(links) = child(root, "PlaybackLinks") {
            for link in children(links, "FileURL") {
                let mut format = dict_format(
                    vec![
                        ("url", text(link)),
                        ("ext", "mp4".into()),
                        ("vbr", int_attr(link, "bitrate")?.into()),
                        ("width", int_attr(link, "width")?.into()),
                        ("height", int_attr(link, "height")?.into()),
                    ],
                    NAME,
                    &video_id,
                )?;
                crate::info_order::mark_int_rates(&mut format);
                formats.push(format);
            }
        }
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("description", description),
                ("thumbnails", Value::Array(thumbnails)),
                ("duration", duration.into()),
                ("categories", Value::Array(categories)),
                ("formats", Value::Null),
            ],
            formats,
        );
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=url,width,height");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://vod.sport5.co.il/?Vc=147&Vi=176331&Page=1"));
        assert!(suitable(
            "http://www.sport5.co.il/articles.aspx?FolderID=3075&docID=176372&lang=HE"
        ));
        assert!(!suitable("http://www.sport5.co.il/"));
    }
}
