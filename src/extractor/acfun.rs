//! Upstream `AcFunVideoIE` (`yt_dlp/extractor/acfun.py`): a video page's
//! `window.videoInfo` JSON, the `ksPlayJson` HLS representations and the
//! page's metadata (a multi-part video's part is in the title). The
//! `AcFunBangumiIE` class is not ported: its test URLs no longer resolve.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_master_optional, search_json};
use crate::info::PyNumber;
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct AcFunVideo;

const NAME: &str = "AcFunVideo";

/// The formats, subtitles and leading fields of `_extract_metadata`.
type Metadata = (
    Vec<VideoFormat>,
    crate::Subtitles,
    Vec<(&'static str, Value)>,
);

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://www\.acfun\.cn/v/ac(?P<id>[_\d]+)").unwrap());
static VIDEO_INFO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"window.videoInfo\s*=").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python's `str()` of a JSON value (`f'{title}'`).
fn python_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        other => other.to_string(),
    }
}

/// `float_or_none(value, scale)` as a number.
fn float_value(value: Option<&Value>, scale: f64) -> Option<f64> {
    super::clyp::float_or_none(value)
        .as_f64()
        .map(|number| number / scale)
}

/// `_extract_metadata(video_id, video_info)`: the formats and subtitles of
/// the `ksPlayJson` HLS representations, with their per-representation
/// details, and the duration, timestamp and referer.
fn metadata(client: &Client, id: &str, video_info: &Value) -> Result<Metadata> {
    let play_text = video_info
        .get("ksPlayJson")
        .and_then(Value::as_str)
        .ok_or_else(|| super::clyp::key_error(id, "ksPlayJson"))?;
    let play_json: Value = crate::json::from_str(play_text).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {id}: Failed to parse JSON (caused by {error})"
        ))
    })?;

    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    let representations = play_json
        .pointer("/adaptationSet/0/representation")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for video in representations {
        let url = video
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| super::clyp::key_error(id, "url"))?;
        let Ok(manifest) = Url::parse(url) else {
            continue;
        };
        let Some(stream) =
            extract_m3u8_master_optional(client, &manifest, NAME, id, None, true, false)
        else {
            continue;
        };
        let codecs_text = video.get("codecs").and_then(Value::as_str).unwrap_or("");
        let codecs = crate::codecs::parse_codecs(Some(codecs_text), &mut |message| {
            crate::screen::warning(message)
        });
        let fps = float_value(video.get("frameRate"), 1.0);
        let width = super::streamable::int_or_none(video.get("width")).as_u64();
        let height = super::streamable::int_or_none(video.get("height")).as_u64();
        let tbr = float_value(video.get("avgBitrate"), 1.0);
        for mut format in stream.formats {
            if let Some(fps) = fps {
                format.fps = Some(PyNumber::Float(fps));
                crate::info_order::mark_format_update(&mut format, "fps");
            }
            if let Some(width) = width {
                format.width = u32::try_from(width).ok();
                crate::info_order::mark_format_update(&mut format, "width");
            }
            if let Some(height) = height {
                format.height = u32::try_from(height).ok();
                crate::info_order::mark_format_update(&mut format, "height");
            }
            if let Some(tbr) = tbr {
                format.tbr = Some(tbr);
                crate::info_order::mark_format_update(&mut format, "tbr");
            }
            if let Some(codecs) = &codecs {
                format.vcodec = Some(codecs.vcodec.clone());
                format.acodec = Some(codecs.acodec.clone());
                crate::info_order::mark_format_update(&mut format, "vcodec");
                crate::info_order::mark_format_update(&mut format, "acodec");
            }
            formats.push(format);
        }
        super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
    }

    let timestamp = super::streamable::int_or_none(video_info.get("uploadTime"));
    let timestamp = timestamp
        .as_i64()
        .map_or(Value::Null, |ms| ms.div_euclid(1000).into());
    let duration =
        float_value(video_info.get("durationMillis"), 1000.0).map_or(Value::Null, Value::from);
    let fields = vec![
        ("duration", duration),
        ("timestamp", timestamp),
        (
            "http_headers",
            serde_json::json!({"Referer": "https://www.acfun.cn/"}),
        ),
    ];
    Ok((formats, subtitles, fields))
}

impl Extractor for AcFunVideo {
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
        let json_all = match search_json(&VIDEO_INFO, &webpage, false).flatten() {
            Some(value) => value,
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract videoInfo; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };

        let mut title = json_all.get("title").cloned().unwrap_or(Value::Null);
        let video_list = json_all
            .get("videoList")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let internal_id = json_all.pointer("/currentVideoInfo/id").cloned();
        if let Some(internal_id) = internal_id.filter(super::nuxt::truthy)
            && video_list.len() > 1
        {
            let (index, part) = video_list
                .iter()
                .enumerate()
                .find(|(_, part)| part.get("id") == Some(&internal_id))
                .ok_or_else(|| {
                    Error::UnsupportedHtml5(
                        "AcFun videos whose current part is not listed are not supported natively"
                            .into(),
                    )
                })?;
            let part_title = part
                .get("title")
                .ok_or_else(|| super::clyp::key_error(&video_id, "title"))?;
            title = Value::String(format!(
                "{} P{:02} {}",
                python_str(&title),
                index + 1,
                python_str(part_title)
            ));
        }

        let current = json_all
            .get("currentVideoInfo")
            .ok_or_else(|| super::clyp::key_error(&video_id, "currentVideoInfo"))?;
        let (formats, subtitles, mut fields) = metadata(client, &video_id, current)?;
        let mut leading = vec![
            ("id", Value::from(video_id.clone())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        leading.append(&mut fields);
        let mut fields = leading;
        let tags: Vec<Value> = json_all
            .get("tagList")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(|tag| tag.get("name").filter(|name| !name.is_null()).cloned())
                    .collect()
            })
            .unwrap_or_default();
        fields.push(("title", title));
        fields.push((
            "thumbnail",
            json_all.get("coverUrl").cloned().unwrap_or(Value::Null),
        ));
        fields.push((
            "description",
            json_all.get("description").cloned().unwrap_or(Value::Null),
        ));
        fields.push((
            "uploader",
            json_all
                .pointer("/user/name")
                .cloned()
                .unwrap_or(Value::Null),
        ));
        fields.push((
            "uploader_id",
            json_all
                .pointer("/user/href")
                .cloned()
                .unwrap_or(Value::Null),
        ));
        if json_all.get("tagList").is_some() {
            fields.push(("tags", tags.into()));
        }
        fields.push((
            "view_count",
            super::streamable::int_or_none(json_all.get("viewCount")),
        ));
        fields.push((
            "like_count",
            super::streamable::int_or_none(json_all.get("likeCountShow")),
        ));
        fields.push((
            "comment_count",
            super::streamable::int_or_none(json_all.get("commentCountShow")),
        ));
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.acfun.cn/v/ac35457073"));
        assert!(suitable("https://www.acfun.cn/v/ac35468952_2"));
        assert!(!suitable(
            "https://www.acfun.cn/bangumi/aa6002917_36188_1745457"
        ));
    }
}
