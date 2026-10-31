//! Upstream `ZoomClipsIE` (`yt_dlp/extractor/zoom.py`): zoom.us clip share
//! pages through the `zoomclips.zoom.us` share API's media info (title,
//! owner, creation time, duration, view count) and its `playUrl` HLS
//! manifest as an unfetched `m3u8_native` format. The recording (`ZoomIE`)
//! extractor is not ported: its passcode and share-redirect flows are not
//! supported natively.
use super::{
    Extractor, dict_format, dict_info, download_json_note, download_webpage_note, update_url_query,
};
use crate::{Error, ExtractResult, Result, SubtitleFormat, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct ZoomClips;
pub struct Zoom;

/// `ZoomIE.IE_NAME`.
const RECORDING_NAME: &str = "zoom";
const RECORDING_KEY: &str = "Zoom";

static RECORDING_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?P<base_url>https?://(?:[^.]+\.)?zoom\.us/)rec(?:ording)?/(?P<type>play|share)/(?P<id>[\w.-]+)",
    )
    .unwrap()
});
static PAGE_DATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"window\.__data__\s*=").unwrap());
static PASSWORD_FORM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<form\b[^>]*\bid=["']password_form["']"#).unwrap());
static RESOLUTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_(\d+x\d+)\.mp4").unwrap());

const NAME: &str = "zoom:clips";
const KEY: &str = "ZoomClips";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"https?://(?:[^.]+\.)?zoom\.us/clips/share/(?P<id>[\w.-]+)")
        .unwrap()
});

/// `_match_id`.
pub fn clips_suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `traverse_obj(..., {str})`: a string value, else `None`.
fn string_value(value: Option<&Value>) -> Value {
    value
        .filter(|value| value.is_string())
        .cloned()
        .unwrap_or(Value::Null)
}

/// `ZoomIE._VALID_URL`.
pub fn suitable(url: &str) -> bool {
    RECORDING_URL.is_match(url)
}

/// `_get_page_data`: the `window.__data__` object of a Zoom page.
fn page_data(webpage: &str, id: &str) -> Result<Value> {
    match super::search_json(&PAGE_DATA, webpage, true) {
        Some(Some(data)) => Ok(data),
        _ => Err(Error::Reported(format!(
            "[{RECORDING_NAME}] {id}: Unable to extract data; {}",
            crate::BUG_REPORTS_MESSAGE
        ))),
    }
}

/// The string at `key` of a Zoom JSON object, else upstream's `KeyError`.
fn required_str(data: &Value, key: &str, id: &str) -> Result<String> {
    match data.get(key).and_then(Value::as_str) {
        Some(text) => Ok(text.to_owned()),
        None => Err(super::clyp::key_error(id, key)),
    }
}

/// `int_or_none` of a JSON value.
fn int_value(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

/// `traverse_obj(data, (key, index))` as an `int_or_none`.
fn resolution_part(data: &Value, key: &str, index: usize) -> Value {
    int_value(data.get(key).and_then(|list| list.get(index)))
}

/// `_get_real_webpage`: a page; a passcode-protected one is not supported.
fn real_webpage(client: &Client, url: &Url, id: &str, url_type: &str) -> Result<String> {
    let webpage = download_webpage_note(
        client,
        url,
        RECORDING_NAME,
        id,
        &format!("Downloading {url_type} webpage"),
    )?;
    if PASSWORD_FORM.is_match(&webpage) {
        return Err(Error::UnsupportedHtml5(
            "passcode-protected Zoom recordings are not supported natively".into(),
        ));
    }
    Ok(webpage)
}

impl Extractor for Zoom {
    fn key(&self) -> &'static str {
        RECORDING_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = RECORDING_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let base_url = found["base_url"].to_owned();
        let url_type = found["type"].to_owned();
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(RECORDING_NAME);
        crate::screen::screen(&format!(
            "[{RECORDING_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let base = Url::parse(&base_url)?;

        // `start_params`: the last `startTime` of the URL, if any.
        let start_time = url
            .query_pairs()
            .filter(|(key, _)| key == "startTime")
            .last()
            .map(|(_, value)| value.into_owned());
        let start_params: Vec<(&str, &str)> = start_time
            .as_deref()
            .map(|value| vec![("startTime", value)])
            .unwrap_or_default();

        let mut page_url = url.clone();
        // The info query: `continueMode` for share links, then `start_params`.
        let mut query: Vec<(&str, &str)> = Vec::new();
        if url_type == "share" {
            let webpage = real_webpage(client, url, &video_id, "share")?;
            let meeting_id =
                required_str(&page_data(&webpage, &video_id)?, "meetingId", &video_id)?;
            let share_info = download_json_note(
                client,
                &base.join(&format!("nws/recording/1.0/play/share-info/{meeting_id}"))?,
                RECORDING_NAME,
                &video_id,
                "Downloading share info JSON",
            )?;
            let redirect_path = share_info
                .pointer("/result/redirectUrl")
                .and_then(Value::as_str)
                .ok_or_else(|| super::clyp::key_error(&video_id, "redirectUrl"))?;
            page_url = update_url_query(&base.join(redirect_path)?, &start_params);
            query.push(("continueMode", "true"));
        }

        let webpage = real_webpage(client, &page_url, &video_id, "play")?;
        let file_id = page_data(&webpage, &video_id)?
            .get("fileId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if file_id.is_empty() {
            // When things go wrong, file_id can be empty string.
            return Err(Error::Reported("Unable to extract file ID".into()));
        }

        query.extend(start_params.iter().copied());
        let info_url = update_url_query(
            &base.join(&format!("nws/recording/1.0/play/info/{file_id}"))?,
            &query,
        );
        let response = download_json_note(
            client,
            &info_url,
            RECORDING_NAME,
            &video_id,
            "Downloading play info JSON",
        )?;
        let data = response
            .get("result")
            .ok_or_else(|| super::clyp::key_error(&video_id, "result"))?;

        let mut subtitles = Subtitles::default();
        for track in ["transcript", "cc", "chapter"] {
            if let Some(path) = data.get(format!("{track}Url")).and_then(Value::as_str) {
                let track_url = base.join(path)?;
                subtitles.insert(
                    Some(track.to_owned()),
                    vec![SubtitleFormat {
                        name: None,
                        url: track_url,
                        ext: "vtt".into(),
                        protocol: None,
                        fragments: Vec::new(),
                        extra: Map::new(),
                    }],
                );
            }
        }

        let mut formats = Vec::new();
        if let Some(view) = data.get("viewMp4Url").and_then(Value::as_str) {
            let file_size = data
                .pointer("/recording/fileSizeInMB")
                .map(|value| match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                });
            let filesize_approx = file_size
                .as_deref()
                .and_then(super::xboxclips::parse_filesize)
                .map_or(Value::Null, Value::from);
            formats.push(dict_format(
                vec![
                    ("format_note", "Camera stream".into()),
                    ("url", view.into()),
                    ("width", resolution_part(data, "viewResolvtions", 0)),
                    ("height", resolution_part(data, "viewResolvtions", 1)),
                    ("format_id", "view".into()),
                    ("ext", "mp4".into()),
                    ("filesize_approx", filesize_approx),
                    ("preference", 0.into()),
                ],
                RECORDING_NAME,
                &video_id,
            )?);
        }
        if let Some(share) = data.get("shareMp4Url").and_then(Value::as_str) {
            formats.push(dict_format(
                vec![
                    ("format_note", "Screen share stream".into()),
                    ("url", share.into()),
                    ("width", resolution_part(data, "shareResolvtions", 0)),
                    ("height", resolution_part(data, "shareResolvtions", 1)),
                    ("format_id", "share".into()),
                    ("ext", "mp4".into()),
                    ("preference", (-1).into()),
                ],
                RECORDING_NAME,
                &video_id,
            )?);
        }
        if let Some(with_share) = data.get("viewMp4WithshareUrl").and_then(Value::as_str) {
            let (width, height) =
                match RESOLUTION.captures(&crate::info::url_basename(&Url::parse(with_share)?)) {
                    Some(found) => {
                        let mut parts = found[1].split('x');
                        let width = parts.next().and_then(|text| text.parse::<i64>().ok());
                        let height = parts.next().and_then(|text| text.parse::<i64>().ok());
                        (width.map(Value::from), height.map(Value::from))
                    }
                    None => (None, None),
                };
            formats.push(dict_format(
                vec![
                    ("width", width.unwrap_or(Value::Null)),
                    ("height", height.unwrap_or(Value::Null)),
                    ("format_note", "Screen share with camera".into()),
                    ("url", with_share.into()),
                    ("format_id", "view_with_share".into()),
                    ("ext", "mp4".into()),
                    ("preference", 1.into()),
                ],
                RECORDING_NAME,
                &video_id,
            )?);
        }

        let title = data
            .pointer("/meet/topic")
            .and_then(Value::as_str)
            .map_or(Value::Null, Value::from);
        let duration = int_value(data.get("duration"));
        let referer = base_url.clone();
        let mut info = dict_info(
            url,
            (RECORDING_NAME, RECORDING_KEY),
            vec![
                ("id", video_id.clone().into()),
                ("title", title),
                ("duration", duration),
                ("subtitles", Value::Null),
                ("formats", Value::Null),
                ("http_headers", json!({"Referer": referer})),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

impl Extractor for ZoomClips {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let clip_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        // The share API sets the CloudFront cookies the manifest needs; the
        // port fetches the manifest only when downloading, like upstream.
        let response = download_json_note(
            client,
            &Url::parse(&format!(
                "https://zoomclips.zoom.us/nws/marvel/2.0/clips/share/{clip_id}"
            ))?,
            NAME,
            &clip_id,
            "Downloading share info JSON",
        )?;
        let result = response
            .get("result")
            .ok_or_else(|| key_error(&clip_id, "result"))?;
        let media_info = result
            .get("mediaInfo")
            .ok_or_else(|| key_error(&clip_id, "mediaInfo"))?;
        let play_url = media_info
            .get("playUrl")
            .ok_or_else(|| key_error(&clip_id, "playUrl"))?;

        // `traverse_obj(media_info, {...})` drops the keys that are missing.
        let mut fields: Vec<(&str, Value)> = vec![("id", clip_id.as_str().into())];
        let optional = [
            ("title", string_value(media_info.get("mediaTopic"))),
            ("uploader", string_value(media_info.get("ownerName"))),
            (
                "release_timestamp",
                // `int_or_none(scale=1000)` of the creation time in milliseconds.
                media_info
                    .get("createdTime")
                    .and_then(Value::as_i64)
                    .map_or(Value::Null, |millis| Value::from(millis / 1000)),
            ),
            (
                "duration",
                super::streamable::int_or_none(media_info.get("mediaDuration")),
            ),
        ];
        fields.extend(optional.into_iter().filter(|(_, value)| !value.is_null()));
        fields.push((
            "view_count",
            super::streamable::int_or_none(result.pointer("/statisticsInfo/count")),
        ));
        let play_url = play_url.as_str().ok_or_else(|| {
            Error::UnsupportedHtml5(
                "ZoomClips play URLs that are not strings are not supported natively".into(),
            )
        })?;
        let format = dict_format(
            vec![
                ("url", play_url.into()),
                ("protocol", "m3u8_native".into()),
                ("ext", "mp4".into()),
            ],
            NAME,
            &clip_id,
        )?;
        fields.push(("formats", Value::Null));
        let info = super::dict_info(url, (NAME, KEY), fields, vec![format]);
        Ok(Some(ExtractResult::Video(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(clips_suitable(
            "https://zoom.us/clips/share/6YEG4i_qS0eiT7UH9zPeYw"
        ));
        assert!(clips_suitable(
            "https://zoom.us/clips/share/6YEG4i_qS0eiT7UH9zPeYw?x=1"
        ));
        assert!(!clips_suitable("https://zoom.us/rec/play/abc"));
    }

    #[test]
    fn recording_urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://cityofdetroit.zoom.us/rec/share/VjE-5kW3xmgbEYqR5KzRgZ1OFZvtMtiXk5HyRJo5kK4m5PYE6RF4rF_oiiO_9qaM.UTAg1MI7JSnF3ZjX"
        ));
        assert!(suitable(
            "https://ffgolf.zoom.us/rec/play/qhEhXbrxq1Zoucx8CMtHzq1Z_2YZRPVCqWK_K-2FkEGRsSLDeOX8Tu4P6jtjZcRry8QhIbvKZdtr4UNo.QcPn2debFskI9whJ"
        ));
        assert!(!suitable(
            "https://zoom.us/clips/share/6YEG4i_qS0eiT7UH9zPeYw"
        ));
    }
}
