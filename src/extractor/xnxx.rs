//! Upstream `XNXXIE` (`yt_dlp/extractor/xnxx.py`): XNXX videos from the
//! page's `setVideoUrlLow`/`setVideoUrlHigh`/`setVideoHLS` player calls, with
//! the OpenGraph (or player) title and thumbnail, OpenGraph duration and the
//! view count.
use super::{Extractor, dict_format, dict_info, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Xnxx;

const NAME: &str = "XNXX";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:video|www)\.xnxx3?\.com/video-?(?P<id>[0-9a-z]+)/").unwrap()
});
// `(?P<q>["'])(?P<url>(?:https?:)?//.+?)(?P=q)` without a backreference.
static VIDEO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"setVideo(?:Url(?P<id>Low|High)|HLS)\s*\(\s*(?:"(?P<dq>(?:https?:)?//[^\n]+?)"|'(?P<sq>(?:https?:)?//[^\n]+?)')"#,
    )
    .unwrap()
});
static VIEWS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"id=["']nb-views-number[^>]+>([\d,.]+)"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// The page's `get(meta)`: `setMETA("VALUE")`.
fn get(webpage: &str, meta: &str) -> Option<String> {
    let pattern = format!(
        r#"set{}\s*\(\s*(?:"([^"\n]+)"|'([^'\n]+)')"#,
        regex::escape(meta)
    );
    let found = Regex::new(&pattern).ok()?.captures(webpage)?;
    found
        .get(1)
        .or_else(|| found.get(2))
        .map(|value| value.as_str().to_owned())
}

/// `get(meta, fatal=False)`: a warning when missing.
fn get_optional(webpage: &str, meta: &str) -> Option<String> {
    let value = get(webpage, meta);
    if value.is_none() {
        search_warning(NAME, meta);
    }
    value
}

fn regex_error(video_id: &str, name: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {video_id}: Unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for Xnxx {
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
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let title = match crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
        {
            Some(title) => title,
            None => {
                get(&webpage, "VideoTitle").ok_or_else(|| regex_error(&video_id, "VideoTitle"))?
            }
        };
        let mut formats = Vec::new();
        for found in VIDEO.captures_iter(&webpage) {
            let Some(raw) = found.name("dq").or_else(|| found.name("sq")) else {
                continue;
            };
            let format_url = raw.as_str();
            // Processing's `sanitize_url`: protocol-relative URLs are HTTP.
            let target = super::proto_relative_http(format_url);
            if super::direct::determine_ext(format_url).as_deref() == Some("m3u8") {
                if let Ok(manifest) = Url::parse(&target)
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
                    for mut format in stream.formats {
                        format.quality = Some(1.0);
                        crate::info_order::mark_int_quality(&mut format);
                        formats.push(format);
                    }
                }
                continue;
            }
            let format_id = found.name("id").map(|id| id.as_str().to_lowercase());
            let quality = if format_id.as_deref() == Some("low") {
                -1
            } else {
                0
            };
            formats.push(dict_format(
                vec![
                    ("url", target.into()),
                    ("format_id", format_id.map_or(Value::Null, Value::from)),
                    ("quality", quality.into()),
                ],
                NAME,
                &video_id,
            )?);
        }
        let thumbnail = crate::html::og_search_property(&webpage, "image")
            .filter(|thumbnail| !thumbnail.is_empty())
            .or_else(|| get_optional(&webpage, "ThumbUrl"))
            .or_else(|| get_optional(&webpage, "ThumbUrl169"));
        // `int_or_none(self._og_search_property('duration', webpage))`.
        let duration = crate::html::og_search_property(&webpage, "duration")
            .ok_or_else(|| regex_error(&video_id, "OpenGraph duration"))?
            .trim()
            .parse::<i64>()
            .ok();
        let view_count = VIEWS.captures(&webpage).and_then(|found| {
            found[1]
                .chars()
                .filter(|c| !matches!(c, ',' | '.' | '+'))
                .collect::<String>()
                .parse::<i64>()
                .ok()
        });
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
                ("duration", duration.map_or(Value::Null, Value::from)),
                ("view_count", view_count.map_or(Value::Null, Value::from)),
                ("age_limit", 18.into()),
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
            "http://www.xnxx.com/video-55awb78/skyrim_test_video"
        ));
        assert!(suitable(
            "http://video.xnxx.com/video1135332/lida_naked_funny_actress_5_"
        ));
        assert!(suitable("http://www.xnxx3.com/video-55awb78/"));
        assert!(!suitable("http://www.xnxx.com/video-55awb78"));
    }
}
