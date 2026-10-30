//! Upstream `IchinanaLiveIE`, `IchinanaLiveClipIE` and `IchinanaLiveVODIE`
//! (`yt_dlp/extractor/ichinanalive.py`, IE names `17live`, `17live:clip` and
//! `17live:vod`): a live's `enter` check and `lives/ID` RTMP/web URLs, a
//! clip's direct MP4 URLs, and a VOD's HLS master from `wap-api`.
use super::{
    Extractor, dict_format, dict_info, download_json, download_json_headers,
    extract_m3u8_fatal_master, post_json_errnote_expecting,
};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct IchinanaLive;
pub struct IchinanaLiveClip;
pub struct IchinanaLiveVod;

const LIVE_NAME: &str = "17live";
const LIVE_KEY: &str = "IchinanaLive";
const CLIP_NAME: &str = "17live:clip";
const CLIP_KEY: &str = "IchinanaLiveClip";
const VOD_NAME: &str = "17live:vod";
const VOD_KEY: &str = "IchinanaLiveVOD";
const API: &str = "https://api-dsa.17app.co/api/v1";

static LIVE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?17\.live/(?:[^/]+/)*(?:live|profile/r)/(?P<id>\d+)").unwrap()
});
static CLIP_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?17\.live/(?:[^/]+/)*profile/r/(?P<uploader_id>\d+)/clip/(?P<id>[^/]+)")
        .unwrap()
});
static VOD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?17\.live/ja/vod/[^/?#]+/(?P<id>[^/?#]+)").unwrap()
});

/// `IchinanaLiveIE.suitable`: the live pattern, minus the clip URLs.
pub fn live_suitable(url: &str) -> bool {
    !clip_suitable(url) && LIVE_URL.is_match(url)
}

/// `_VALID_URL` of the clips.
pub fn clip_suitable(url: &str) -> bool {
    CLIP_URL.is_match(url)
}

/// `_VALID_URL` of the VODs.
pub fn vod_suitable(url: &str) -> bool {
    VOD_URL.is_match(url)
}

/// `traverse_obj(data, (path, {str}))` for a string field.
fn text(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| value.is_string()).cloned()
}

/// A value that is not null.
fn present(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| !value.is_null()).cloned()
}

/// `int_or_none` as a value.
fn int_value(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Number(number) => number.as_i64().map(Value::from),
        Value::String(text) => text.trim().parse::<i64>().ok().map(Value::from),
        _ => None,
    }
}

/// `url_or_none` as a value.
fn url_value(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map(Value::from)
}

fn live_json(client: &Client, url: &str, id: &str, referer: &str) -> Result<Value> {
    download_json_headers(
        client,
        &Url::parse(url)?,
        LIVE_NAME,
        id,
        &[("Referer", referer)],
    )
}

impl Extractor for IchinanaLive {
    fn key(&self) -> &'static str {
        LIVE_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !live_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = LIVE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(LIVE_NAME);
        crate::screen::screen(&format!(
            "[{LIVE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = format!("https://17.live/live/{video_id}");
        let referer = [("Referer", page.as_str())];

        // `_download_json(..., fatal=False, expected_status=420, data=b'\0')`.
        let enter = match post_json_errnote_expecting(
            client,
            &Url::parse(&format!("{API}/lives/{video_id}/enter"))?,
            (LIVE_NAME, &video_id),
            (
                "Downloading JSON metadata",
                "Unable to download JSON metadata",
            ),
            &referer,
            "\0".to_owned(),
            &[420],
        ) {
            Ok(enter) => Some(enter),
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{LIVE_NAME}] {video_id}: Unable to download JSON metadata: {}",
                    crate::upstream_error_text(&source)
                ));
                None
            }
            Err(error) => return Err(error),
        };
        if enter
            .as_ref()
            .and_then(|enter| enter.get("message"))
            .and_then(Value::as_str)
            == Some("ended")
        {
            return Err(Error::Reported(format!(
                "[{LIVE_NAME}] {video_id}: This live has ended."
            )));
        }

        let view = live_json(client, &format!("{API}/lives/{video_id}"), &video_id, &page)?;
        let uploader = ["displayName", "openID"]
            .into_iter()
            .find_map(|key| text(view.pointer(&format!("/userInfo/{key}"))));
        let Some(rtmp) = view
            .get("rtmpUrls")
            .and_then(Value::as_array)
            .and_then(|urls| urls.first())
            .and_then(Value::as_object)
        else {
            return Err(Error::Reported(format!(
                "[{LIVE_NAME}] {video_id}: unable to extract live URL information; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let formats = live_formats(&video_id, rtmp, &page)?;

        let mut fields: Vec<(&str, Value)> = vec![
            ("id", Value::from(video_id.as_str())),
            (
                "title",
                uploader
                    .clone()
                    .unwrap_or_else(|| Value::from(video_id.as_str())),
            ),
            ("formats", Value::Null),
            ("is_live", Value::Bool(true)),
        ];
        let optional = [
            ("uploader", uploader),
            ("uploader_id", Some(Value::from(video_id.as_str()))),
            ("like_count", present(view.get("receivedLikeCount"))),
            ("view_count", present(view.get("viewerCount"))),
            ("thumbnail", present(view.get("coverPhoto"))),
            ("description", present(view.get("caption"))),
            ("timestamp", present(view.get("beginTime"))),
        ];
        for (key, value) in optional {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (LIVE_NAME, LIVE_KEY),
            fields,
            formats,
        ))))
    }
}

/// The RTMP and web URLs of `rtmpUrls[0]`, best quality last.
fn live_formats(
    video_id: &str,
    rtmp: &Map<String, Value>,
    referer: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let mut formats = Vec::new();
    for (name, value) in rtmp {
        let Some(url) = value.as_str().filter(|url| url.starts_with("http")) else {
            continue;
        };
        let mut quality: i64 = -1;
        if name.contains("web") {
            quality -= 1;
        }
        if name.contains("High") {
            quality += 4;
        }
        if name.contains("Low") {
            quality -= 2;
        }
        formats.push(dict_format(
            vec![
                ("format_id", Value::from(name.as_str())),
                ("url", Value::from(url)),
                ("quality", Value::from(quality)),
                ("http_headers", serde_json::json!({"Referer": referer})),
                ("ext", Value::from("flv")),
                ("vcodec", Value::from("h264")),
                ("acodec", Value::from("aac")),
            ],
            LIVE_NAME,
            video_id,
        )?);
    }
    Ok(formats)
}

impl Extractor for IchinanaLiveClip {
    fn key(&self) -> &'static str {
        CLIP_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CLIP_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let uploader_id = found["uploader_id"].to_owned();
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(CLIP_NAME);
        crate::screen::screen(&format!(
            "[{CLIP_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = format!("https://17.live/profile/r/{uploader_id}/clip/{video_id}");
        let view = download_json_headers(
            client,
            &Url::parse(&format!("{API}/clips/{video_id}"))?,
            CLIP_NAME,
            &video_id,
            &[("Referer", page.as_str())],
        )?;
        let uploader = ["displayName", "name"]
            .into_iter()
            .find_map(|key| text(view.pointer(&format!("/userInfo/{key}"))));

        let mut formats = Vec::new();
        let candidates = [
            ("videoURL", "video", -1),
            ("transcodeURL", "transcode", -1),
            ("srcVideoURL", "srcVideo", 1),
        ];
        for (field, format_id, quality) in candidates {
            let Some(media) = view
                .get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            formats.push(dict_format(
                vec![
                    ("format_id", Value::from(format_id)),
                    ("url", Value::from(media)),
                    ("quality", Value::from(quality as i64)),
                    ("ext", Value::from("mp4")),
                    ("protocol", Value::from("https")),
                    ("vcodec", Value::from("h264")),
                    ("acodec", Value::from("aac")),
                    ("http_headers", serde_json::json!({"Referer": page})),
                ],
                CLIP_NAME,
                &video_id,
            )?);
        }

        let mut fields: Vec<(&str, Value)> = vec![
            ("id", Value::from(video_id.as_str())),
            (
                "title",
                uploader
                    .clone()
                    .unwrap_or_else(|| Value::from(video_id.as_str())),
            ),
            ("formats", Value::Null),
        ];
        let optional = [
            ("uploader", uploader),
            ("uploader_id", Some(Value::from(uploader_id.as_str()))),
            ("like_count", present(view.get("likeCount"))),
            ("view_count", present(view.get("viewCount"))),
            ("thumbnail", present(view.get("imageURL"))),
            ("duration", present(view.get("duration"))),
            ("description", present(view.get("caption"))),
            (
                "upload_date",
                view.get("createdAt")
                    .map(|created| match created {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    })
                    .and_then(|date| crate::dates::unified_strdate(&date, false))
                    .map(Value::from),
            ),
        ];
        for (key, value) in optional {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (CLIP_NAME, CLIP_KEY),
            fields,
            formats,
        ))))
    }
}

impl Extractor for IchinanaLiveVod {
    fn key(&self) -> &'static str {
        VOD_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VOD_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(VOD_NAME);
        crate::screen::screen(&format!(
            "[{VOD_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let data = download_json(
            client,
            &Url::parse(&format!("https://wap-api.17app.co/api/v1/vods/{video_id}"))?,
            VOD_NAME,
            &video_id,
        )?;
        let mut formats = Vec::new();
        let mut info = None;
        if let Some(manifest) = data.get("vodURL").and_then(Value::as_str) {
            let manifest = Url::parse(manifest)?;
            info = Some(extract_m3u8_fatal_master(
                client, &manifest, VOD_NAME, &video_id, None, false,
            )?);
        }
        if let Some(parsed) = info {
            formats = parsed.formats;
        }
        let mut fields: Vec<(&str, Value)> = Vec::new();
        let optional = [
            ("id", text(data.get("vodID"))),
            ("title", text(data.get("title"))),
        ];
        for (key, value) in optional {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        fields.push(("formats", Value::Null));
        let rest = [
            ("uploader", text(data.pointer("/userInfo/displayName"))),
            (
                "uploader_id",
                data.pointer("/userInfo/roomID")
                    .and_then(|room| match room {
                        Value::Number(number) => Some(Value::from(number.to_string())),
                        Value::String(text) => Some(Value::from(text.as_str())),
                        _ => None,
                    }),
            ),
            ("channel", text(data.pointer("/userInfo/name"))),
            ("channel_id", text(data.pointer("/userInfo/userID"))),
            ("like_count", int_value(data.get("likeCount"))),
            ("view_count", int_value(data.get("viewCount"))),
            ("thumbnail", url_value(data.get("imageURL"))),
            ("duration", int_value(data.get("duration"))),
            ("description", text(data.get("description"))),
            ("timestamp", int_value(data.get("createdAt"))),
        ];
        for (key, value) in rest {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (VOD_NAME, VOD_KEY),
            fields,
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_match_upstream_valid_urls() {
        assert!(live_suitable("https://17.live/live/3773096"));
        assert!(live_suitable("https://17.live/ja/live/3773096"));
        assert!(!live_suitable(
            "https://17.live/profile/r/1789280/clip/1bHQSK8KUieruFXaCH4A4upCzlN"
        ));
        assert!(clip_suitable(
            "https://17.live/ja/profile/r/1789280/clip/1bHQSK8KUieruFXaCH4A4upCzlN"
        ));
        assert!(vod_suitable(
            "https://17.live/ja/vod/27323042/2cf84520-e65e-4b22-891e-1d3a00b0f068"
        ));
        assert!(!vod_suitable("https://17.live/live/3773096"));
    }
}
