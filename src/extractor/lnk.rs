//! Upstream `LnkIE` (`yt_dlp/extractor/lnk.py`): the video-config API's
//! `videoInfo` and its HLS (and non-DRM FairPlay HLS) playlists.
use super::{Extractor, dict_info, download_json, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Lnk;

const NAME: &str = "Lnk";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?lnk\.lt/[^/]+/(?P<id>\d+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python truthiness of a JSON value.
pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

impl Extractor for Lnk {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!("https://lnk.lt/api/video/video-config/{id}"))?;
        let response = download_json(client, &api, NAME, &id)?;
        let video = response
            .get("videoInfo")
            .filter(|video| video.is_object())
            .cloned()
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Lnk responses without a videoInfo object are not supported natively".into(),
                )
            })?;
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let mut add = |key: &str| -> Result<()> {
            let manifest = video.get(key).and_then(Value::as_str).ok_or_else(|| {
                Error::UnsupportedHtml5(format!(
                    "Lnk {key} values that are not strings are not supported natively"
                ))
            })?;
            let mut stream =
                extract_m3u8_fatal(client, &Url::parse(manifest)?, NAME, &id, None, true)?;
            super::hls::without_ext(&mut stream);
            formats.extend(stream.formats);
            super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            Ok(())
        };
        if truthy(video.get("videoUrl")) {
            add("videoUrl")?;
        }
        if truthy(video.get("videoFairplayUrl")) && !truthy(video.get("drm")) {
            add("videoFairplayUrl")?;
        }
        let get = |key: &str| video.get(key).cloned().unwrap_or(Value::Null);
        // `unified_strdate(video_json.get('airDate'))`.
        let upload_date = video
            .get("airDate")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_strdate(date, true))
            .map_or(Value::Null, Value::from);
        // `format_field(video_json, 'posterImage', TEMPLATE)`.
        let thumbnail = match video.get("posterImage") {
            Some(Value::String(text)) if !text.is_empty() => {
                Value::from(format!("https://lnk.lt/all-images/{text}"))
            }
            _ => Value::from(""),
        };
        let mut fields = vec![
            ("id", id.clone().into()),
            ("title", get("title")),
            ("description", get("description")),
            ("view_count", get("viewsCount")),
            ("duration", get("duration")),
            ("upload_date", upload_date),
            ("thumbnail", thumbnail),
            (
                "episode_number",
                super::streamable::int_or_none(video.get("episodeNumber")),
            ),
            ("series", get("programTitle")),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        // Processing drops a `None` duration.
        fields.retain(|(key, value)| *key != "duration" || !value.is_null());
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
        assert!(suitable("https://lnk.lt/varom-pierdom/27186"));
        assert!(suitable("https://www.lnk.lt/zinios/47289"));
        assert!(!suitable("https://lnk.lt/27186"));
    }
}
