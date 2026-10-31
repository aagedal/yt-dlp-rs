//! Upstream `NateIE` (`yt_dlp/extractor/nate.py`): a clip's
//! `tv.nate.com/api/v1/clip` JSON and its `smcUriList` MP4 renditions
//! (the quality is the URL's last two characters). `NateProgramIE` (a
//! program's paged clip ranking) is not ported.
use super::{Extractor, dict_format, dict_info, download_json, streamable::int_or_none};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Nate;

const NAME: &str = "Nate";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| super::url_regex(r"^https?://tv\.nate\.com/clip/(?P<id>[0-9]+)").unwrap());

/// `_QUALITY`: a rendition's height by its format code.
fn quality_height(code: &str) -> Option<i64> {
    match code {
        "36" => Some(2160),
        "35" => Some(1080),
        "34" => Some(720),
        "33" => Some(480),
        "32" => Some(360),
        "31" => Some(270),
        _ => None,
    }
}

/// `traverse_obj(video_data, *keys)`: the first truthy value.
fn first_truthy<'a>(data: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| data.get(*key))
        .find(|value| super::nuxt::truthy(value))
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => text.clone().into(),
        Some(Value::Number(number)) => number.to_string().into(),
        Some(Value::Bool(flag)) => if *flag { "True" } else { "False" }.into(),
        _ => Value::Null,
    }
}

/// `suitable` (`_VALID_URL`).
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Nate {
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
        let api = Url::parse(&format!("https://tv.nate.com/api/v1/clip/{video_id}"))?;
        let video_data = download_json(client, &api, NAME, &video_id)?;

        let mut formats = Vec::new();
        let renditions = video_data.get("smcUriList").and_then(Value::as_array);
        for f_url in renditions.into_iter().flatten() {
            let Value::String(f_url) = f_url else {
                return Err(Error::UnsupportedHtml5(
                    "Nate rendition URLs that are not strings are not supported natively".into(),
                ));
            };
            // `f_url[-2:]`
            let code: String = f_url
                .chars()
                .rev()
                .take(2)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let height = quality_height(&code).map_or(Value::Null, Value::from);
            let quality = int_or_none(Some(&Value::String(code.clone())));
            formats.push(dict_format(
                vec![
                    ("format_id", code.clone().into()),
                    ("url", f_url.clone().into()),
                    ("height", height),
                    ("quality", quality),
                ],
                NAME,
                &video_id,
            )?);
        }

        let program_title = video_data
            .get("programTitle")
            .cloned()
            .unwrap_or(Value::Null);
        let program_seq = str_or_none(video_data.get("programSeq"));
        let upload_date = match first_truthy(&video_data, &["broadDate", "regDate"]) {
            Some(Value::String(date)) => {
                crate::dates::unified_strdate(date, false).map_or(Value::Null, Value::from)
            }
            _ => Value::Null,
        };
        // `video_data['hashTag'].split(',')` when truthy.
        let tags = match video_data.get("hashTag") {
            Some(Value::String(tags)) if !tags.is_empty() => tags
                .split(',')
                .map(|tag| Value::from(tag.to_owned()))
                .collect::<Vec<_>>()
                .into(),
            _ => Value::Null,
        };

        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                (
                    "title",
                    video_data.get("clipTitle").cloned().unwrap_or(Value::Null),
                ),
                (
                    "description",
                    video_data.get("synopsis").cloned().unwrap_or(Value::Null),
                ),
                (
                    "thumbnail",
                    video_data.get("contentImg").cloned().unwrap_or(Value::Null),
                ),
                ("upload_date", upload_date),
                (
                    "age_limit",
                    video_data.get("targetAge").cloned().unwrap_or(Value::Null),
                ),
                (
                    "duration",
                    video_data.get("playTime").cloned().unwrap_or(Value::Null),
                ),
                ("formats", Value::Null),
                ("uploader", program_title.clone()),
                ("channel", program_title),
                ("channel_id", program_seq.clone()),
                ("uploader_id", program_seq),
                ("tags", tags),
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
        assert!(suitable("https://tv.nate.com/clip/4300566"));
        assert!(suitable("https://tv.nate.com/clip/1848976"));
        assert!(!suitable("https://tv.nate.com/program/clips/27987"));
        assert!(!suitable("https://tv.nate.com/"));
    }

    #[test]
    fn quality_codes_map_to_heights() {
        assert_eq!(quality_height("31"), Some(270));
        assert_eq!(quality_height("36"), Some(2160));
        assert_eq!(quality_height("37"), None);
    }
}
