//! Upstream `JojIE` (`yt_dlp/extractor/joj.py`): the `media.joj.sk` embed
//! page's `videoTitle`, its `src`/`bitrates` MP4 list (through
//! `js_to_json`), else the `Video.php` XML playlist's files, with the
//! OpenGraph thumbnail and `videoDuration`.
use super::{Extractor, dict_format, dict_info, download_webpage, download_xml};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Joj;

const NAME: &str = "Joj";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:joj:|https?://media\.joj\.sk/embed/)(?P<id>[^/?#^]+)").unwrap()
});
static VIDEO_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:videoTitle\s*:)\s*(?P<json>["'].+["'])\s*"#).unwrap());
static BITRATES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)(?:src|bitrates)\s*=\s*(\{.+?\});").unwrap());
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)[pP]|(pal)\.").unwrap());
static HEIGHT_P: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)[pP]").unwrap());
static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"videoDuration\s*:\s*(\d+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("JOJ {what} are not supported natively"))
}

/// `int_or_none(text)` of a regex group.
fn int(text: &str) -> Option<i64> {
    text.parse().ok()
}

/// The `videoTitle` JSON string (`_search_json(..., default=None)`): the
/// first JSON value of the quoted match, `None` when it does not parse.
fn video_title(webpage: &str) -> Result<Option<String>> {
    let Some(found) = VIDEO_TITLE.captures(webpage) else {
        return Ok(None);
    };
    match serde_json::Deserializer::from_str(&found["json"])
        .into_iter::<Value>()
        .next()
    {
        Some(Ok(Value::String(title))) => Ok(Some(title)),
        Some(Ok(Value::Null)) | Some(Err(_)) | None => Ok(None),
        Some(Ok(_)) => Err(unsupported("video titles that are not strings")),
    }
}

/// The `Video.php` playlist's `./files/file` formats.
fn xml_formats(body: &str, video_id: &str) -> Result<Vec<crate::VideoFormat>> {
    let document = roxmltree::Document::parse(body.trim_start_matches('\u{feff}'))
        .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
    let mut formats = Vec::new();
    let files = document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name("files"))
        .flat_map(|files| files.children().filter(|node| node.has_tag_name("file")));
    for file in files {
        let Some(path) = file.attribute("path").filter(|path| !path.is_empty()) else {
            continue;
        };
        let format_id = file
            .attribute("id")
            .filter(|id| !id.is_empty())
            .or_else(|| file.attribute("label"));
        let height = HEIGHT_P
            .captures(format_id.filter(|id| !id.is_empty()).unwrap_or(path))
            .and_then(|found| int(&found[1]));
        formats.push(dict_format(
            vec![
                (
                    "url",
                    format!("http://n16.joj.sk/storage/{}", path.replacen("dat/", "", 1)).into(),
                ),
                ("format_id", format_id.map_or(Value::Null, Value::from)),
                ("height", height.map_or(Value::Null, Value::from)),
            ],
            NAME,
            video_id,
        )?);
    }
    Ok(formats)
}

impl Extractor for Joj {
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
        let embed = Url::parse(&format!("https://media.joj.sk/embed/{video_id}"))?;
        let webpage = download_webpage(client, &embed, NAME, &video_id)?;

        let title = match video_title(&webpage)?.filter(|title| !title.is_empty()) {
            Some(title) => Some(title),
            None => match crate::html::html_extract_title(&webpage).filter(|t| !t.is_empty()) {
                Some(title) => Some(title),
                None => {
                    let title = crate::html::og_search_property(&webpage, "title");
                    if title.is_none() {
                        super::search_warning(NAME, "OpenGraph title");
                    }
                    title
                }
            },
        };

        let source = BITRATES
            .captures(&webpage)
            .map_or_else(|| "{}".to_owned(), |found| found[1].to_owned());
        let json = crate::js_to_json::js_to_json(&source);
        let bitrates = match crate::json::python_decode_error_text(&json) {
            Some(error) => {
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Failed to parse JSON: {error}"
                ));
                None
            }
            None => crate::json::from_str::<Value>(&json).ok(),
        };

        let mut formats = Vec::new();
        if let Some(Value::Array(urls)) = bitrates.as_ref().and_then(|bitrates| bitrates.get("mp4"))
        {
            for format_url in urls.iter().filter_map(Value::as_str) {
                let height = HEIGHT.captures(format_url).and_then(|found| {
                    match (found.get(1), found.get(2)) {
                        (Some(height), _) => Some(height.as_str().to_owned()),
                        (None, Some(_)) => Some("576".to_owned()),
                        _ => None,
                    }
                });
                formats.push(dict_format(
                    vec![
                        ("url", format_url.into()),
                        (
                            "format_id",
                            height
                                .as_ref()
                                .map_or("".into(), |h| format!("{h}p").into()),
                        ),
                        (
                            "height",
                            height
                                .as_deref()
                                .and_then(int)
                                .map_or(Value::Null, Value::from),
                        ),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        if formats.is_empty() {
            let playlist = Url::parse(&format!(
                "https://media.joj.sk/services/Video.php?clip={video_id}"
            ))?;
            let body = download_xml(client, &playlist, NAME, &video_id)?;
            formats = xml_formats(&body, &video_id)?;
        }

        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            super::search_warning(NAME, "thumbnail URL");
        }
        let duration = DURATION.captures(&webpage).and_then(|found| int(&found[1]));
        if !DURATION.is_match(&webpage) {
            super::search_warning(NAME, "duration");
        }

        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.map_or(Value::Null, Value::from)),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
                ("duration", duration.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
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
        assert!(suitable(
            "https://media.joj.sk/embed/a388ec4c-6019-4a4a-9312-b1bee194e932"
        ));
        assert!(suitable("joj:9i1cxv"));
        assert!(!suitable("https://media.joj.sk/"));
    }

    #[test]
    fn titles_and_playlist_files() {
        assert_eq!(
            video_title("videoTitle: \"A \\u00e9\",\n")
                .unwrap()
                .as_deref(),
            Some("A é")
        );
        assert_eq!(video_title("videoTitle: 'bad',").unwrap(), None);
        let formats = xml_formats(
            r#"<playlist><files><file id="720p" path="dat/a/b.mp4"/><file label="x" path="c/360P.mp4"/><file path=""/></files></playlist>"#,
            "x",
        )
        .unwrap();
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].url.as_str(), "http://n16.joj.sk/storage/a/b.mp4");
        assert_eq!(formats[0].height, Some(720));
        assert_eq!(formats[1].format_id, "x");
        assert_eq!(formats[1].height, None);
    }
}
