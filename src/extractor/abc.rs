//! Upstream `ABCIE` (`yt_dlp/extractor/abc.py`, IE_NAME `abc.net.au`): ABC
//! news and listen pages whose audio or video renditions come from the
//! page's download link, its JSON `sources`/`files`/`renditions` arrays, or
//! its `inline*Data.push(...)` script. Embedded YouTube players (which upstream
//! turns into a playlist of `url_result`s) and the `iview` API classes
//! (`ABCIViewIE`, `ABCIViewShowSeriesIE`) are not supported natively.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Abc;

const NAME: &str = "abc.net.au";
const KEY: &str = "ABC";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?abc\.net\.au/(?:news|btn|listen)/(?:[^/?#]+/){1,4}(?P<id>\d{5,})",
    )
    .unwrap()
});
static DOWNLOAD_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<a\s+href="(?P<url>[^"]+)"\s+data-duration="\d+"\s+title="Download audio directly">"#,
    )
    .unwrap()
});
static YOUTUBE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a href="(?P<url>http://www\.youtube\.com/watch\?v=[^"]+)"><span><strong>External Link:</strong>"#)
        .unwrap()
});
static YOUTUBE_IFRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<iframe width="100%" src="(?P<url>//www\.youtube-nocookie\.com/embed/[^?"]+)"#)
        .unwrap()
});
static JSON_SOURCES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""(?:sources|files|renditions)":\s*(\[[^\]]+\])"#).unwrap());
static INLINE_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"inline(Video|Audio|YouTube)Data\.push\(([^)]+)\);").unwrap());
static EXPIRED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)class="expired-(?:video|audio)".+?<span>(.+?)</span>"#).unwrap()
});
static RENDITION_SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_(?:(\d+)|(\d+)k)\.mp4$").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none`.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `traverse_obj(urls_info, (0, ('contentType', 'MIMEType')), get_all=False)`.
fn first_content_type(urls: &[Value]) -> Option<&str> {
    let first = urls.first()?;
    ["contentType", "MIMEType"]
        .iter()
        .find_map(|key| first.get(*key).filter(|value| !value.is_null()))
        .and_then(Value::as_str)
}

fn failure(video_id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {video_id}: {message}"))
}

impl Extractor for Abc {
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

        // `urls_info` (a dict or a list) with the `youtube` and `video` flags.
        let mut youtube = false;
        let mut video = false;
        let urls: Vec<Value> = if let Some(link) = DOWNLOAD_LINK.captures(&webpage) {
            vec![serde_json::json!({"url": &link["url"]})]
        } else if let Some(link) = YOUTUBE_LINK
            .captures(&webpage)
            .or_else(|| YOUTUBE_IFRAME.captures(&webpage))
        {
            youtube = true;
            video = true;
            vec![serde_json::json!({"url": &link["url"]})]
        } else {
            let (json_data, inline_type) = if let Some(found) = JSON_SOURCES.captures(&webpage) {
                (found[1].to_owned(), None)
            } else if let Some(found) = INLINE_DATA.captures(&webpage) {
                (found[2].to_owned(), Some(found[1].to_owned()))
            } else {
                let expired = EXPIRED
                    .captures(&webpage)
                    .map(|found| crate::html::clean_html(&found[1]))
                    .filter(|text| !text.is_empty());
                return Err(match expired {
                    Some(message) => failure(&video_id, &format!("{NAME} said: {message}")),
                    None => Error::Reported(format!(
                        "[{NAME}] {video_id}: Unable to extract video urls; {}",
                        crate::BUG_REPORTS_MESSAGE
                    )),
                });
            };
            let parsed = crate::js_to_json::js_to_json(&json_data);
            let parsed: Value = serde_json::from_str(&parsed)
                .map_err(|_| failure(&video_id, "Failed to parse JSON"))?;
            youtube = inline_type.as_deref() == Some("YouTube");
            let urls = match parsed {
                Value::Array(items) => items,
                other => vec![other],
            };
            video = inline_type.as_deref() == Some("Video")
                || first_content_type(&urls) == Some("video/mp4");
            urls
        };

        if youtube {
            return Err(Error::UnsupportedHtml5(
                "ABC pages with YouTube players (upstream playlist of url results) are not supported natively"
                    .into(),
            ));
        }

        let mut formats = Vec::new();
        for url_info in &urls {
            let Some(media) = url_info.get("url").and_then(Value::as_str) else {
                return Err(failure(
                    &video_id,
                    "An extractor error has occurred. (caused by KeyError('url'))",
                ));
            };
            let mut height = int_or_none(url_info.get("height"));
            let mut bitrate = int_or_none(url_info.get("bitrate"));
            let mut width = int_or_none(url_info.get("width"));
            let mut format_id = None;
            if let Some(found) = RENDITION_SIZE.captures(media) {
                if let Some(size) = found.get(1) {
                    let size = size.as_str().parse::<i64>().ok();
                    height = height.or(size);
                    width = width.or(int_or_none(url_info.get("label")));
                } else {
                    bitrate = bitrate.or(found.get(2).and_then(|k| k.as_str().parse().ok()));
                    format_id = str_or_none(url_info.get("label"));
                }
            }
            let vcodec = if video {
                url_info.get("codec").cloned().unwrap_or(Value::Null)
            } else {
                Value::from("none")
            };
            let mut format = dict_format(
                vec![
                    ("url", Value::from(media)),
                    ("vcodec", vcodec),
                    ("width", width.map_or(Value::Null, Value::from)),
                    ("height", height.map_or(Value::Null, Value::from)),
                    ("tbr", bitrate.map_or(Value::Null, Value::from)),
                    (
                        "filesize",
                        int_or_none(url_info.get("filesize")).map_or(Value::Null, Value::from),
                    ),
                    ("format_id", format_id.map_or(Value::Null, Value::from)),
                ],
                NAME,
                &video_id,
            )?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }

        let og = |property: &str| {
            crate::html::og_search_property(&webpage, property).map_or(Value::Null, Value::from)
        };
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.clone().into()),
                ("title", og("title")),
                // Placeholder keeping upstream's key order (formats after the title).
                ("formats", Value::Null),
                ("description", og("description")),
                ("thumbnail", og("image")),
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
            "http://www.abc.net.au/news/2014-11-05/australia-to-staff-ebola-treatment-centre-in-sierra-leone/5868334"
        ));
        assert!(suitable(
            "https://www.abc.net.au/listen/programs/the-followers-madness-of-two/presents-followers-madness-of-two/105697646"
        ));
        assert!(!suitable("https://www.abc.net.au/news/2015-10-19/abc"));
    }

    #[test]
    fn rendition_names_give_sizes_and_bitrates() {
        assert!(RENDITION_SIZE.is_match("https://x/video_720.mp4"));
        assert!(RENDITION_SIZE.is_match("https://x/video_128k.mp4"));
        assert!(!RENDITION_SIZE.is_match("https://x/audio.mp3"));
    }
}
