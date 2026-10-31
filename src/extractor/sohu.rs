//! Upstream `SohuIE` and `SohuVIE` (`yt_dlp/extractor/sohu.py`): a page's
//! `var vid` play API (`my.tv` or `tv` form) with its part CDN redirects, as
//! single-part videos, and the base64 `tv.sohu.com/v/` link as a followed
//! result. Multi-part videos are not ported.
use super::{Extractor, dict_format, dict_info, download_json_note, download_webpage, follow_url};
use crate::{Error, ExtractResult, Result, VideoFormat, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Sohu;
pub struct SohuV;

const NAME: &str = "Sohu";
const V_NAME: &str = "SohuV";

/// `my.tv.sohu.com/.../ID.shtml` (the `(?(mytv)|n)` conditional in two
/// patterns, since the regex crate has no conditionals).
static MYTV_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://my\.tv\.sohu\.com/.+?/(?P<id>\d+)\.shtml").unwrap());
static TV_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://tv\.sohu\.com/.+?/n(?P<id>\d+)\.shtml").unwrap());
static V_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://tv\.sohu\.com/v/(?P<id>[\w=-]+)\.html(?:$|[#?])").unwrap()
});
static VIDEO_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var vid ?= ?["'](\d+)["']"#).unwrap());
static PUBLISH_TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"publishTime:\s*["'](\d+-\d+-\d+ \d+:\d+)["']"#).unwrap());
static TITLE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"( - 高清正版在线观看)? - 搜狐视频$").unwrap());
static V_SUBDIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+/n\d+\.shtml").unwrap());

/// `SohuIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    MYTV_URL.is_match(url) || TV_URL.is_match(url)
}

/// `SohuVIE._match_valid_url`.
pub fn v_suitable(url: &str) -> bool {
    V_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Sohu {what} are not supported natively"))
}

fn int_value(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|v| v as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `_fetch_data(vid_id, mytv)`.
fn fetch_data(client: &Client, id: &str, vid: &str, mytv: bool) -> Result<Value> {
    let base = if mytv {
        "http://my.tv.sohu.com/play/videonew.do?vid="
    } else {
        "http://hot.vrs.sohu.com/vrs_flash.action?vid="
    };
    let url = Url::parse(&format!("{base}{vid}"))?;
    // `headers=self.geo_verification_headers()`.
    download_json_note(
        &super::geo_verification_client(client),
        &url,
        NAME,
        id,
        &format!("Downloading JSON data for {vid}"),
    )
}

impl Extractor for Sohu {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let (mytv, video_id) = if let Some(found) = MYTV_URL.captures(url.as_str()) {
            (true, found["id"].to_owned())
        } else if let Some(found) = TV_URL.captures(url.as_str()) {
            (false, found["id"].to_owned())
        } else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let og_title = crate::html::og_search_property(&webpage, "title");
        if og_title.is_none() {
            super::search_warning(NAME, "OpenGraph title");
        }
        let title = og_title
            .map(|title| TITLE_SUFFIX.replace(&title, "").into_owned())
            .ok_or_else(|| unsupported("pages without an OpenGraph title"))?;

        let vid = VIDEO_PATH
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract video path; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let vid_data = fetch_data(client, &video_id, &vid, mytv)?;
        if vid_data.get("play").and_then(Value::as_i64) != Some(1) {
            if vid_data.get("status").and_then(Value::as_i64) == Some(12) {
                return Err(Error::Reported(format!(
                    "[{NAME}] {NAME} said: There's something wrong in the video."
                )));
            }
            return Err(Error::Reported(format!(
                "[{NAME}] {NAME} said: The video is only licensed to users in Mainland China."
            )));
        }
        let data = vid_data
            .get("data")
            .ok_or_else(|| key_error(&video_id, "data"))?;

        // The per-format play data (the same response for the page's own ID).
        let mut formats_json: Vec<(&str, Value)> = Vec::new();
        for format_id in ["nor", "high", "super", "ori", "h2644k", "h2654k"] {
            let vid_id = match data.get(format!("{format_id}Vid")) {
                Some(Value::String(text)) if !text.is_empty() => text.clone(),
                Some(Value::Number(number)) => number.to_string(),
                _ => continue,
            };
            let format_data = if vid_id == vid {
                vid_data.clone()
            } else {
                fetch_data(client, &video_id, &vid_id, mytv)?
            };
            formats_json.push((format_id, format_data));
        }

        let part_count = int_value(data.get("totalBlocks")).unwrap_or(0);
        if part_count != 1 {
            return Err(unsupported("multi-part videos"));
        }
        let mut formats = Vec::new();
        for (format_id, format_data) in &formats_json {
            let allot = format_data
                .get("allot")
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(&video_id, "allot"))?;
            let media = format_data
                .get("data")
                .ok_or_else(|| key_error(&video_id, "data"))?;
            let clip_url = ["clipsURL", "mp4PlayUrl"]
                .iter()
                .find_map(|key| {
                    media
                        .get(*key)
                        .and_then(|list| list.get(0))
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                })
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] {video_id}: Unable to extract url for clip 0"
                    ))
                })?;
            let su = media
                .get("su")
                .and_then(|list| list.get(0))
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(&video_id, "su"))?;

            let mut video_url = "newflv.sohu.ccgslb.net".to_owned();
            let mut cdn_id: Option<String> = None;
            let mut retries = 0;
            while video_url.contains("newflv.sohu.ccgslb.net") {
                let mut params = vec![
                    ("prot", "9".to_owned()),
                    ("file", clip_url.clone()),
                    ("new", su.to_owned()),
                    ("prod", "h5n".to_owned()),
                    ("rb", "1".to_owned()),
                ];
                if let Some(cdn_id) = &cdn_id {
                    params.push(("idc", cdn_id.clone()));
                }
                let part_url = super::update_url_query(
                    &Url::parse(&format!("http://{allot}/"))?,
                    &params
                        .iter()
                        .map(|(key, value)| (*key, value.as_str()))
                        .collect::<Vec<_>>(),
                );
                let note = format!("Downloading {format_id} video URL part 1 of 1");
                let part_info = super::download_json_note(
                    client,
                    &part_url,
                    NAME,
                    &video_id,
                    &if retries > 0 {
                        format!("{note} (retry #{retries})")
                    } else {
                        note
                    },
                )?;
                video_url = part_info
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| key_error(&video_id, "url"))?
                    .to_owned();
                cdn_id = part_info.get("nid").and_then(|nid| match nid {
                    Value::String(text) => Some(text.clone()),
                    Value::Number(number) => Some(number.to_string()),
                    _ => None,
                });
                retries += 1;
                if retries > 5 {
                    return Err(Error::Reported(format!(
                        "[{NAME}] {video_id}: Failed to get video URL; {}",
                        crate::BUG_REPORTS_MESSAGE
                    )));
                }
            }

            let filesize = int_value(media.get("clipsBytes").and_then(|list| list.get(0)));
            formats.push(format_entry(
                &video_id, format_id, &video_url, filesize, media,
            )?);
        }

        let duration = data
            .get("clipsDuration")
            .and_then(|list| list.get(0))
            .cloned()
            .unwrap_or(Value::Null);
        let publish_time = if mytv {
            PUBLISH_TIME
                .captures(&webpage)
                .and_then(|found| crate::dates::unified_timestamp(&found[1], false))
        } else {
            vid_data
                .get("tv_application_time")
                .and_then(Value::as_str)
                .and_then(|text| crate::dates::unified_timestamp(text, false))
        };
        let timestamp = publish_time.map(|time| time - 8 * 3600);

        let uploader = vid_data
            .get("wm_data")
            .and_then(|wm| wm.get("wm_username"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let tags = data.get("tag").and_then(Value::as_str).map(|tags| {
            tags.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });
        let cover = data
            .get("coverImg")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let alt_title = data
            .get("subName")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let mut fields = vec![("timestamp", timestamp.map_or(Value::Null, Value::from))];
        if let Some(alt_title) = alt_title {
            fields.push(("alt_title", json!(alt_title)));
        }
        if let Some(uploader) = uploader {
            fields.push(("uploader", json!(uploader)));
        }
        if let Some(cover) = cover {
            fields.push(("thumbnail", json!(cover)));
        }
        if let Some(tags) = tags {
            fields.push(("tags", json!(tags)));
        }
        fields.extend([
            ("id", json!(video_id)),
            ("title", json!(title)),
            ("duration", duration),
            ("formats", Value::Null),
        ]);
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

/// One part's `formats` entry: the CDN URL, `format_id`, `filesize` and the
/// `width`/`height`/`fps` of the play data.
fn format_entry(
    video_id: &str,
    format_id: &str,
    video_url: &str,
    filesize: Option<i64>,
    media: &Value,
) -> Result<VideoFormat> {
    dict_format(
        vec![
            ("url", json!(video_url)),
            ("format_id", json!(format_id)),
            ("filesize", json!(filesize)),
            ("width", json!(int_value(media.get("width")))),
            ("height", json!(int_value(media.get("height")))),
            ("fps", json!(int_value(media.get("fps")))),
        ],
        NAME,
        video_id,
    )
}

impl Extractor for SohuV {
    fn key(&self) -> &'static str {
        V_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = V_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let encoded = &found["id"];
        let decoded = base64::engine::general_purpose::URL_SAFE
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{V_NAME}] {encoded}: Unable to decode the video path"
                ))
            })?;
        let subdomain = if V_SUBDIR.is_match(&decoded) {
            "tv"
        } else {
            "my.tv"
        };
        let target = Url::parse(&format!("http://{subdomain}.sohu.com/"))?.join(&decoded)?;
        Ok(Some(follow_url(url, target.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://tv.sohu.com/20130724/n382479172.shtml#super"
        ));
        assert!(suitable("http://tv.sohu.com/20150305/n409385080.shtml"));
        assert!(suitable(
            "http://my.tv.sohu.com/us/232799889/78693464.shtml"
        ));
        assert!(suitable("http://my.tv.sohu.com/pl/8384802/78910339.shtml"));
        assert!(!suitable(
            "http://tv.sohu.com/v/dXMvMjMyNzk5ODg5Lzc4NjkzNDY0LnNodG1s.html"
        ));
        assert!(v_suitable(
            "https://tv.sohu.com/v/dXMvMjMyNzk5ODg5Lzc4NjkzNDY0LnNodG1s.html"
        ));
    }
}
