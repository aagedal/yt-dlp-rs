//! Upstream `AppleConnectIE` (`yt_dlp/extractor/appleconnect.py`): an Apple
//! Music Connect post's uploaded video, from the amp-api catalog with the
//! `WebPlayKid` token of `AppleBaseIE`.
use super::applepodcasts;
use super::{
    Extractor, dict_format, dict_info, download_json_headers, download_webpage, update_url_query,
};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct AppleConnect;

const NAME: &str = "apple:music:connect";
const KEY: &str = "AppleConnect";
const BASE_URL: &str = "https://music.apple.com";
const JWT_KEY_ID: &str = "WebPlayKid";

/// `_QUALITIES`, in order: a format's `quality` is its position.
const QUALITIES: [(&str, Option<i64>); 6] = [
    ("provisionalUploadVideo", None),
    ("sdVideo", Some(480)),
    ("sdVideoWithPlusAudio", Some(480)),
    ("sd480pVideo", Some(480)),
    ("720pHdVideo", Some(720)),
    ("1080pHdVideo", Some(1080)),
];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://music\.apple\.com/[\w-]+/post/(?P<id>\d+)").unwrap()
});
static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\s*[xX×,]\s*(\d+)").unwrap());
static WIDTH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\D)(\d{2,5})w(?:$|[^a-zA-Z0-9])").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `parse_resolution(text, lenient=True)`: the `width` and `height` found.
fn lenient_resolution(text: &str) -> (Option<u32>, Option<u32>) {
    if let Some(found) = SIZE.captures(text) {
        return (found[1].parse().ok(), found[2].parse().ok());
    }
    (
        WIDTH.captures(text).and_then(|found| found[1].parse().ok()),
        None,
    )
}

/// The `formats` entries of `assetTokens` (with a URL value), in order.
fn formats(assets: &Value, id: &str) -> Result<Vec<VideoFormat>> {
    let Some(tokens) = assets.get("assetTokens").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut formats = Vec::new();
    for (format_id, source) in tokens {
        let Some(source) = source.as_str().and_then(super::json_ld::url_or_none) else {
            continue;
        };
        let known = QUALITIES.iter().find(|(name, _)| *name == format_id);
        let height = known.and_then(|(_, height)| *height);
        let quality = QUALITIES
            .iter()
            .position(|(name, _)| name == format_id)
            .map_or(-1, |position| position as i64);
        let mut fields: Vec<(&str, Value)> = vec![
            ("ext", "m4v".into()),
            ("format_id", format_id.as_str().into()),
            ("height", height.map_or(Value::Null, Value::from)),
            ("quality", quality.into()),
            ("url", source.clone().into()),
        ];
        let plain = match Url::parse(&source) {
            Ok(mut plain) => {
                plain.set_query(None);
                plain.to_string()
            }
            Err(_) => source.clone(),
        };
        let (width, resolved_height) = lenient_resolution(&plain);
        if let Some(width) = width {
            fields.push(("width", width.into()));
        }
        if let Some(resolved_height) = resolved_height {
            // The dict's `height` key keeps its place.
            for (key, value) in &mut fields {
                if *key == "height" {
                    *value = resolved_height.into();
                }
            }
        }
        formats.push(dict_format(fields, NAME, id)?);
    }
    Ok(formats)
}

/// `_html_search_meta(['og:image', 'og:image:secure_url', 'twitter:image'], ...)`.
fn thumbnail(webpage: &str) -> Value {
    ["og:image", "og:image:secure_url", "twitter:image"]
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name))
        .map_or(Value::Null, Value::from)
}

impl Extractor for AppleConnect {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<crate::ExtractResult>> {
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
        let token =
            applepodcasts::get_token(client, &webpage, &video_id, NAME, BASE_URL, JWT_KEY_ID)?;
        let api = update_url_query(
            &Url::parse("https://amp-api.music.apple.com/v1/catalog/us/uploaded-videos")?,
            &[("ids", video_id.as_str()), ("l", "en-US")],
        );
        let authorization = format!("Bearer {token}");
        let videos = download_json_headers(
            client,
            &api,
            NAME,
            &video_id,
            &[("Authorization", &authorization), ("Origin", BASE_URL)],
        )?;
        // `('data', ..., 'attributes', any, {require(...)})`.
        let attributes = videos
            .get("data")
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find_map(|item| {
                    item.get("attributes")
                        .filter(|attributes| super::nuxt::truthy(attributes))
                })
            })
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract video information; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let formats = formats(attributes, &video_id)?;
        let text = |key: &str| {
            attributes
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.as_str().into()),
            ("formats", Value::Null),
            ("thumbnail", thumbnail(&webpage)),
        ];
        if let Some(title) = text("name") {
            fields.push(("title", title.into()));
        }
        if let Some(milliseconds) = attributes
            .get("durationInMilliseconds")
            .and_then(Value::as_f64)
        {
            fields.push(("duration", (milliseconds / 1000.0).into()));
        }
        if let Some(date) =
            text("uploadDate").and_then(|date| crate::dates::unified_strdate(&date, false))
        {
            fields.push(("upload_date", date.into()));
        }
        if let Some(uploader) = ["artistName", "uploadingArtistName"]
            .iter()
            .find_map(|key| text(key))
        {
            fields.push(("uploader", uploader.into()));
        }
        if let Some(page) = text("postUrl").and_then(|page| super::json_ld::url_or_none(&page)) {
            fields.push(("webpage_url", page.into()));
        }
        Ok(Some(crate::ExtractResult::Video(dict_info(
            url,
            (NAME, KEY),
            fields,
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://music.apple.com/us/post/1018290019"));
        assert!(suitable("https://music.apple.com/gb/post/1016746627"));
        assert!(!suitable("https://music.apple.com/us/album/1018290019"));
    }

    #[test]
    fn lenient_resolution_reads_sizes_and_widths() {
        assert_eq!(
            lenient_resolution("a/mzvf_1.640x480.h264lc.m4v"),
            (Some(640), Some(480))
        );
        assert_eq!(
            lenient_resolution("a/mzvf_8.1920w.h264lc.m4v"),
            (Some(1920), None)
        );
        assert_eq!(lenient_resolution("a/plain.m4v"), (None, None));
    }
}
