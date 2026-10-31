//! Upstream `SapoIE` (`yt_dlp/extractor/sapo.py`): a video's
//! `rd3.videos.sapo.pt` RSS item (MRSS fields) as its SD and HD MP4 files.
use super::{Extractor, dict_format, dict_info, download_xml};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Sapo;

const NAME: &str = "Sapo";
const SAPO_NS: &str = "http://videos.sapo.pt/mrss/";
const MRSS_NS: &str = "http://search.yahoo.com/mrss/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:v2|www)\.)?videos\.sapo\.(?:pt|cv|ao|mz|tl)/(?P<id>[\da-zA-Z]{20})",
    )
    .unwrap()
});
static MOV_1: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/mov/1$").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!(
        "SAPO RSS items without {what} are not supported natively"
    ))
}

/// `element.find(path)` for a direct child (namespace URI or none).
fn child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    namespace: Option<&str>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children().find(|child| {
        child.is_element()
            && child.tag_name().name() == name
            && child.tag_name().namespace() == namespace
    })
}

/// `item.find(path).text`: the text before the first child element, `None`
/// when empty; a missing element is an unsupported item.
fn text(
    item: roxmltree::Node<'_, '_>,
    namespace: Option<&str>,
    name: &str,
) -> Result<Option<String>> {
    let element = child(item, namespace, name).ok_or_else(|| unsupported(name))?;
    let mut text = String::new();
    for node in element.children() {
        if node.is_element() {
            break;
        }
        text.push_str(node.text().unwrap_or_default());
    }
    Ok((!text.is_empty()).then_some(text))
}

/// Python `int(text)`.
fn int(value: Option<String>, name: &str) -> Result<i64> {
    value
        .and_then(|value| value.trim().replace('_', "").parse::<i64>().ok())
        .ok_or_else(|| unsupported(&format!("an integer {name}")))
}

impl Extractor for Sapo {
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
        let rss = Url::parse(&format!("http://rd3.videos.sapo.pt/{id}/rss2"))?;
        let body = download_xml(client, &rss, NAME, &id)?;
        let document = roxmltree::Document::parse(body.trim_start_matches('\u{feff}'))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
        let item = child(document.root_element(), None, "channel")
            .and_then(|channel| child(channel, None, "item"))
            .ok_or_else(|| unsupported("a channel item"))?;

        let title = text(item, None, "title")?;
        let description = text(item, Some(SAPO_NS), "synopse")?;
        let thumbnail = child(item, Some(MRSS_NS), "content")
            .ok_or_else(|| unsupported("media:content"))?
            .attribute("url")
            .map(str::to_owned);
        let duration = text(item, Some(SAPO_NS), "time")?
            .and_then(|time| crate::parse_duration(&time))
            .map(|duration| match duration {
                crate::info::PyNumber::Int(value) => Value::from(value),
                crate::info::PyNumber::Float(value) => Value::from(value),
            });
        let uploader = text(item, Some(SAPO_NS), "author")?;
        let upload_date = text(item, None, "pubDate")?
            .and_then(|date| crate::dates::unified_strdate(&date, true));
        let view_count = int(text(item, Some(SAPO_NS), "views")?, "views")?;
        let comment_count = int(text(item, Some(SAPO_NS), "comment_count")?, "comment_count")?;
        let categories: Vec<Value> = text(item, Some(SAPO_NS), "tags")?
            .map(|tags| tags.split_whitespace().map(Value::from).collect())
            .unwrap_or_default();
        let age_limit = if text(item, Some(SAPO_NS), "m18")?.as_deref() == Some("true") {
            18
        } else {
            0
        };
        let video_url =
            text(item, Some(SAPO_NS), "videoFile")?.ok_or_else(|| unsupported("a videoFile"))?;
        let video_size =
            text(item, Some(SAPO_NS), "videoSize")?.ok_or_else(|| unsupported("a videoSize"))?;
        let mut size = video_size.split('x');
        let width = int(size.next().map(str::to_owned), "width")?;
        let height = int(size.next().map(str::to_owned), "height")?;

        let mut formats = vec![dict_format(
            vec![
                ("url", video_url.clone().into()),
                ("ext", "mp4".into()),
                ("format_id", "sd".into()),
                ("width", width.into()),
                ("height", height.into()),
            ],
            NAME,
            &id,
        )?];
        if text(item, Some(SAPO_NS), "HD")?.as_deref() == Some("true") {
            formats.push(dict_format(
                vec![
                    (
                        "url",
                        MOV_1.replace(&video_url, "/mov/39").into_owned().into(),
                    ),
                    ("ext", "mp4".into()),
                    ("format_id", "hd".into()),
                    ("width", 1280.into()),
                    ("height", 720.into()),
                ],
                NAME,
                &id,
            )?);
        }

        let mut fields = vec![("id", Value::from(id.clone()))];
        if let Some(title) = title {
            fields.push(("title", title.into()));
        }
        fields.extend([
            ("description", description.map_or(Value::Null, Value::from)),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
        ]);
        // Processing drops a `None` duration.
        if let Some(duration) = duration {
            fields.push(("duration", duration));
        }
        fields.extend([
            ("uploader", uploader.map_or(Value::Null, Value::from)),
            ("upload_date", upload_date.map_or(Value::Null, Value::from)),
            ("view_count", view_count.into()),
            ("comment_count", comment_count.into()),
            ("categories", Value::Array(categories)),
            ("age_limit", age_limit.into()),
            ("formats", Value::Null),
        ]);
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://videos.sapo.pt/UBz95kOtiWYUMTA5Ghfi"));
        assert!(suitable("http://v2.videos.sapo.pt/yLqjzPtbTimsn2wWBKHz"));
        assert!(!suitable("http://videos.sapo.pt/short"));
    }
}
