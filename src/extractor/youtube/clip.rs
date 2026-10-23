//! Upstream `YoutubeClipIE` (`yt_dlp/extractor/youtube/_clip.py`): a clip page
//! names its video and loop section, which the `youtube` extractor downloads.
use super::tab::{at, extract_webpage};
use crate::{Error, ExtractResult, Result};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

const NAME: &str = "youtube:clip";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?youtube\.com/clip/(?P<id>[^/?#]+)").unwrap());

/// `YoutubeClipIE.suitable`.
pub(crate) fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

pub struct YoutubeClip;

impl super::super::Extractor for YoutubeClip {
    fn key(&self) -> &'static str {
        "youtube:clip"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<crate::VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let clip_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        super::initialize();
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let fail = |message: &str| Error::Reported(format!("[{NAME}] {clip_id}: {message}"));
        let (data, _) = extract_webpage(client, url, &clip_id, NAME, true)?;
        let data = data.unwrap_or(Value::Null);
        let video_id = at(&data, &["currentVideoEndpoint", "watchEndpoint", "videoId"])
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| fail("Unable to find video ID"))?
            .to_owned();
        let clip = clip_loop(&data).ok_or_else(|| fail("Unable to find clip section"))?;
        let milliseconds = |key: &str| {
            clip.get(key)
                .and_then(|value| match value {
                    Value::String(text) => text.trim().parse::<i64>().ok(),
                    Value::Number(number) => number.as_i64(),
                    _ => None,
                })
                .ok_or_else(|| fail(&format!("Unable to find clip {key}")))
        };
        let (start, end) = (milliseconds("startTimeMs")?, milliseconds("endTimeMs")?);
        let target = format!("https://www.youtube.com/watch?v={video_id}");
        let mut follow = super::super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        // `url_transparent` fields; https is preferred for FFmpeg's sections.
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            // With sections, upstream also keeps the clip's extractor.
            json!({"url": target, "fields": {
                "id": clip_id,
                "extractor": NAME,
                "extractor_key": "YoutubeClip",
                "media_type": "clip",
                "section_start": start as f64 / 1000.0,
                "section_end": end as f64 / 1000.0,
                "_format_sort_fields": [
                    "proto:https", "quality", "res", "fps", "hdr:12", "source",
                    "vcodec", "channels", "acodec", "lang",
                ],
            }}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

/// The clip panel's `loopCommand` (`startTimeMs`/`endTimeMs`), the first
/// found like `traverse_obj(..., get_all=False)`.
fn clip_loop(data: &Value) -> Option<&Value> {
    fn each(value: Option<&Value>) -> impl Iterator<Item = &Value> {
        value.and_then(Value::as_array).into_iter().flatten()
    }
    for panel in each(data.get("engagementPanels")) {
        let contents = at(
            panel,
            &[
                "engagementPanelSectionListRenderer",
                "content",
                "clipSectionRenderer",
                "contents",
            ],
        );
        for content in each(contents) {
            let commands = at(
                content,
                &[
                    "clipAttributionRenderer",
                    "onScrubExit",
                    "commandExecutorCommand",
                    "commands",
                ],
            );
            for command in each(commands) {
                let inner = at(
                    command,
                    &[
                        "openPopupAction",
                        "popup",
                        "notificationActionRenderer",
                        "actionButton",
                        "buttonRenderer",
                        "command",
                        "commandExecutorCommand",
                        "commands",
                    ],
                );
                if let Some(found) = each(inner).find_map(|inner| inner.get("loopCommand")) {
                    return Some(found);
                }
            }
        }
    }
    None
}
