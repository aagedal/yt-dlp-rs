//! Upstream `CanalAlphaIE` (`yt_dlp/extractor/canalalpha.py`): Canal Alpha
//! videos from the page's `__SERVER_STATE__`: its MP4 files and non-fatal
//! HLS and DASH manifests, with the title, description, poster, date and
//! duration.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct CanalAlpha;

const NAME: &str = "CanalAlpha";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?canalalpha\.ch/play/[^/]+/[^/]+/(?P<id>\d+)/?.*",
    )
    .unwrap()
});
static STATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__SERVER_STATE__\s?=\s?").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Canal Alpha {what} are not supported natively"))
}

/// The state object assigned to `__SERVER_STATE__` and followed by `;`.
fn server_state(webpage: &str, id: &str) -> Result<Value> {
    let fail = || {
        Error::Reported(format!(
            "[{NAME}] {id}: Unable to extract data_json; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    };
    let start = STATE.find(webpage).ok_or_else(fail)?;
    let rest = &webpage[start.end()..];
    if !rest.starts_with('{') {
        return Err(fail());
    }
    let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
    let value = stream
        .next()
        .and_then(std::result::Result::ok)
        .ok_or_else(|| unsupported("server states that do not parse"))?;
    let after = rest[stream.byte_offset()..].trim_start_matches(' ');
    if !after.starts_with(';') {
        return Err(fail());
    }
    Ok(value)
}

/// `dict_get(data, keys)`: the first truthy value.
fn dict_get<'a>(data: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| data.get(*key))
        .find(|value| super::nuxt::truthy(value))
}

impl Extractor for CanalAlpha {
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
        let state = server_state(&webpage, &video_id)?;
        let mut data = &state;
        for key in ["1", "data", "data"] {
            data = data.get(key).ok_or_else(|| key_error(&video_id, key))?;
        }
        let manifests = data
            .pointer("/video/manifests")
            .filter(|value| value.is_object());
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        if let Some(Value::Array(videos)) = data.pointer("/video/mp4") {
            for video in videos {
                let Some(source) = video.get("$url").filter(|value| super::nuxt::truthy(value))
                else {
                    continue;
                };
                let int = |key: &str| {
                    video
                        .pointer(&format!("/res/{key}"))
                        .filter(|value| value.is_i64() || value.is_u64())
                        .cloned()
                        .unwrap_or(Value::Null)
                };
                formats.push(dict_format(
                    vec![
                        ("url", source.clone()),
                        ("ext", "mp4".into()),
                        ("width", int("width")),
                        ("height", int("height")),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        let manifest = |key: &str| {
            manifests
                .and_then(|manifests| manifests.get(key))
                .filter(|value| super::nuxt::truthy(value))
                .and_then(Value::as_str)
                .and_then(|value| Url::parse(value).ok())
        };
        if let Some(hls) = manifest("hls")
            && let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &hls,
                NAME,
                &video_id,
                Some("hls"),
                true,
                false,
            )
        {
            formats.extend(stream.formats);
            super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
        }
        if let Some(dash) = manifest("dash")
            && let Some(stream) = super::extract_mpd_optional_note(
                client,
                &dash,
                NAME,
                &video_id,
                Some("dash"),
                "Downloading MPD manifest",
            )
        {
            formats.extend(stream.formats);
            super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
        }
        let title = match data.get("title") {
            Some(Value::String(title)) => title.trim().to_owned(),
            _ => return Err(unsupported("videos without a string title")),
        };
        let description = dict_get(data, &["longDesc", "shortDesc"])
            .and_then(Value::as_str)
            .map(crate::html::clean_html)
            .map_or(Value::Null, Value::from);
        let upload_date = dict_get(data, &["webPublishAt", "featuredAt", "diffusionDate"])
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_strdate(date, false))
            .map_or(Value::Null, Value::from);
        let duration = data
            .pointer("/video/duration")
            .filter(|value| value.is_i64() || value.is_u64())
            .cloned()
            .unwrap_or(Value::Null);
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("description", description),
                (
                    "thumbnail",
                    data.get("poster").cloned().unwrap_or(Value::Null),
                ),
                ("upload_date", upload_date),
                ("duration", duration),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.canalalpha.ch/play/le-journal/episode/24520/jeudi-28-octobre-2021"
        ));
        assert!(!suitable("https://www.canalalpha.ch/play/le-journal/"));
    }
}
