//! Upstream `ZoomClipsIE` (`yt_dlp/extractor/zoom.py`): zoom.us clip share
//! pages through the `zoomclips.zoom.us` share API's media info (title,
//! owner, creation time, duration, view count) and its `playUrl` HLS
//! manifest as an unfetched `m3u8_native` format. The recording (`ZoomIE`)
//! extractor is not ported: its passcode and share-redirect flows are not
//! supported natively.
use super::{Extractor, dict_format, download_json_note};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct ZoomClips;

const NAME: &str = "zoom:clips";
const KEY: &str = "ZoomClips";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:[^.]+\.)?zoom\.us/clips/share/(?P<id>[\w.-]+)").unwrap()
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
}
