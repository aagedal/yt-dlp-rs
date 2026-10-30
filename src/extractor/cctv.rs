//! Upstream `CCTVIE` (`yt_dlp/extractor/cctv.py`, the 央视网 sites): a page's
//! video ID (one of several script variables), the `getHttpVideoInfo.do`
//! JSON with its sample MP4s and HLS master, and the page description.
use super::{
    Extractor, dict_format, dict_info, download_json, download_webpage,
    extract_m3u8_master_optional,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Cctv;

const NAME: &str = "CCTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:[^/]+)\.(?:cntv|cctv)\.(?:com|cn)|(?:www\.)?ncpa-classic\.com)/(?:[^/]+/)*?(?P<id>[^/?#&]+?)(?:/index)?(?:\.s?html|[?#&]|$)",
    )
    .unwrap()
});
static VIDEO_ID: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"var\s+guid\s*=\s*["']([\da-fA-F]+)"#,
        r#"videoCenterId(?:["']\s*,|:)\s*["']([\da-fA-F]+)"#,
        r#"changePlayer\s*\(\s*["']([\da-fA-F]+)"#,
        r#"load[Vv]ideo\s*\(\s*["']([\da-fA-F]+)"#,
        r#"var\s+initMyAray\s*=\s*["']([\da-fA-F]+)"#,
        r#"var\s+ids\s*=\s*\[["']([\da-fA-F]+)"#,
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});
static HLS_BITRATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"maxbr=\d+&?").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `try_get(video, lambda x: x[key][0]['url'], str)`.
fn chapter_url(video: &Value, key: &str) -> Option<String> {
    video
        .get(key)?
        .get(0)?
        .get("url")?
        .as_str()
        .map(str::to_owned)
}

impl Extractor for Cctv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let page_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &page_id)?;

        let video_id = VIDEO_ID
            .iter()
            .find_map(|pattern| pattern.captures(&webpage).map(|found| found[1].to_owned()))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {page_id}: Unable to extract video id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;

        let api = super::update_url_query(
            &Url::parse("http://vdn.apps.cntv.cn/api/getHttpVideoInfo.do")?,
            &[
                ("pid", video_id.as_str()),
                ("url", url.as_str()),
                ("idl", "32"),
                ("idlr", "32"),
                ("modifyed", "false"),
            ],
        );
        let data = download_json(client, &api, NAME, &video_id)?;
        let title = data
            .get("title")
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&video_id, "title"))?;

        let mut formats = Vec::new();
        if let Some(video) = data.get("video").filter(|video| video.is_object()) {
            for (quality, chapters_key) in ["lowChapters", "chapters"].into_iter().enumerate() {
                if let Some(video_url) =
                    chapter_url(video, chapters_key).filter(|url| !url.is_empty())
                {
                    formats.push(dict_format(
                        vec![
                            ("url", video_url.into()),
                            ("format_id", "http".into()),
                            ("quality", (quality as i64).into()),
                            // Sample clip.
                            ("preference", (-10).into()),
                        ],
                        NAME,
                        &video_id,
                    )?);
                }
            }
        }

        if let Some(hls_url) = data.get("hls_url").and_then(Value::as_str) {
            let hls_url = HLS_BITRATE.replace_all(hls_url, "");
            // An emptied query (`main.m3u8?`) is dropped, as upstream's
            // request URL has none.
            if let Ok(mut manifest) = Url::parse(&hls_url) {
                if manifest.query() == Some("") {
                    manifest.set_query(None);
                }
                if let Some(stream) = extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("hls"),
                    false,
                    false,
                ) {
                    formats.extend(stream.formats);
                }
            }
        }

        let uploader = data.get("editer_name").cloned().unwrap_or(Value::Null);
        let description = crate::html::html_search_meta_clean(&webpage, "description")
            .map_or(Value::Null, Value::from);
        let timestamp = data
            .get("f_pgmtime")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, true))
            .map_or(Value::Null, Value::from);
        let duration = data
            .get("video")
            .and_then(|video| video.get("totalLength"))
            .and_then(|length| super::clyp::float_or_none(Some(length)).as_f64())
            .map_or(Value::Null, Value::from);

        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", title),
                ("description", description),
                ("uploader", uploader),
                ("timestamp", timestamp),
                ("duration", duration),
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
            "http://sports.cntv.cn/2016/02/12/ARTIaBRxv4rTT1yWf1frW2wi160212.shtml"
        ));
        assert!(suitable(
            "https://news.cctv.com/2024/02/21/ARTIcU5tKIOIF2myEGCATkLo240221.shtml"
        ));
        assert!(suitable(
            "http://www.ncpa-classic.com/clt/more/416/index.shtml"
        ));
        assert!(suitable(
            "http://tv.cntv.cn/video/C39296/e0210d949f113ddfb38d31f00a4e5c44"
        ));
        assert!(!suitable("https://www.example.com/video"));
    }
}
