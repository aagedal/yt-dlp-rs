//! Upstream `VTVGoIE` and `VTVIE` (`yt_dlp/extractor/vtv.py`): VTVGo videos
//! from the page's player `index.m3u8`, and VTV.vn videos from the
//! highlight box's `data-vid`, both as fatal HLS with OpenGraph metadata.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct VtvGo;
pub struct Vtv;

const GO_NAME: &str = "VTVGo";
const NAME: &str = "VTV";

static GO_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(
            r"^https?://(?:www\.)?vtvgo\.vn/(kho-video|tin-tuc)/[\w.-]*?(?P<id>\d+)(?:\.[a-z]+|/)?(?:$|[?#])",
        )
        .unwrap(),
        Regex::new(r"^https?://(?:www\.)?vtvgo\.vn/digital/detail\.php\?(?:[^#]+&)?content_id=(?P<id>\d+)")
            .unwrap(),
    ]
});
static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?vtv\.vn/video/[\w-]*?(?P<id>\d+)\.htm")
        .unwrap()
});
static M3U8: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:var\s+link\s*=\s*|addPlayer\()["'](https://[^"']+/index\.m3u8)["']"#).unwrap()
});

fn go_id(url: &str) -> Option<String> {
    GO_URLS
        .iter()
        .find_map(|regex| regex.captures(url))
        .map(|found| found["id"].to_owned())
}

/// `VTVGoIE._match_id`.
pub fn go_suitable(url: &str) -> bool {
    go_id(url).is_some()
}

/// `VTVIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// The shared result: OpenGraph title, description and image (silent
/// `default=None`) and the fatal HLS master playlist.
fn result(
    client: &Client,
    url: &Url,
    (name, id): (&'static str, &str),
    webpage: &str,
    m3u8_url: &str,
) -> Result<VideoInfo> {
    let stream = extract_m3u8_fatal_master(
        client,
        &Url::parse(m3u8_url).map_err(|_| {
            Error::UnsupportedHtml5(format!(
                "{name} playlist URLs that are not URLs are not supported natively"
            ))
        })?,
        name,
        id,
        None,
        false,
    )?;
    let og = |property: &str| {
        crate::html::og_search_property(webpage, property).map_or(Value::Null, Value::from)
    };
    Ok(dict_info(
        url,
        (name, name),
        vec![
            ("id", id.into()),
            ("title", og("title")),
            ("description", og("description")),
            ("thumbnail", og("image")),
            ("formats", Value::Null),
        ],
        stream.formats,
    ))
}

impl Extractor for VtvGo {
    fn key(&self) -> &'static str {
        GO_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(video_id) = go_id(url.as_str()) else {
            return Ok(None);
        };
        start(GO_NAME, url);
        let webpage = download_webpage(client, url, GO_NAME, &video_id)?;
        let m3u8_url = M3U8
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{GO_NAME}] {video_id}: Unable to extract m3u8 url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        result(client, url, (GO_NAME, &video_id), &webpage, &m3u8_url).map(Some)
    }
}

/// `get_element_html_by_class(class, html)`.
fn html_by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()
        .and_then(|found| found.into_iter().next())
        .map(|(_, whole)| whole)
}

impl Extractor for Vtv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let element = html_by_class("video-highlight-box", &webpage)
            .and_then(|box_html| html_by_class("VCSortableInPreviewMode", &box_html))
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "VTV pages without a highlighted video are not supported natively".into(),
                )
            })?;
        let attributes = crate::html_elements::extract_attributes(&element);
        let data_vid = attributes
            .get("data-vid")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "{video_id}: An extractor error has occurred. (caused by KeyError('data-vid')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let path = data_vid
            .strip_prefix("vtv.mediacdn.vn/")
            .unwrap_or(data_vid);
        let m3u8_url = format!("https://cdn-videos.vtv.vn/{path}/master.m3u8");
        result(client, url, (NAME, &video_id), &webpage, &m3u8_url).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(go_suitable(
            "https://vtvgo.vn/kho-video/bep-vtv-vit-chao-rieng-so-24-888456.html"
        ));
        assert!(go_suitable(
            "https://vtvgo.vn/digital/detail.php?digital_id=168&content_id=1234"
        ));
        assert!(suitable(
            "https://vtv.vn/video/thoi-su-20h-vtv1-12-6-2024-680411.htm"
        ));
        assert!(!suitable("https://vtv.vn/thoi-su.htm"));
    }
}
