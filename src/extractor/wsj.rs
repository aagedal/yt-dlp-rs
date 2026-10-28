//! Upstream `WSJIE` and `WSJArticleIE` (`yt_dlp/extractor/wsj.py`): Wall
//! Street Journal and Barron's videos from the `find_all_videos` API (HDS,
//! HLS and MP4 renditions), and articles' embedded video IDs as `wsj:ID`
//! results.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Wsj;
pub struct WsjArticle;

const NAME: &str = "WSJ";
const ARTICLE_NAME: &str = "WSJArticle";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:https?://video-api\.wsj\.com/api-video/player/iframe\.html\?.*?\bguid=|https?://(?:www\.)?(?:wsj|barrons)\.com/video/(?:[^/]+/)+|wsj:)(?P<id>[a-fA-F0-9-]{36})",
    )
    .unwrap()
});
static ARTICLE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^https?://(?:www\.)?wsj\.com/(?:articles|opinion)/(?P<id>[^/?#&]+)").unwrap()
});
static VIDEO_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:id=["']video|video-|iframe\.html\?guid=|data-src=["'])([a-fA-F0-9-]{36})"#)
        .unwrap()
});

const FIELDS: &str = "type,hls,videoMP4List,thumbnailList,author,description,name,duration,videoURL,titletag,formattedCreationDate,keywords,editor";

/// `WSJIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `WSJArticleIE._match_id`.
pub fn article_suitable(url: &str) -> bool {
    ARTICLE_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("WSJ {what} are not supported natively"))
}

fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

impl Extractor for Wsj {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);
        let api = super::update_url_query(
            &Url::parse("http://video-api.wsj.com/api-video/find_all_videos.asp")?,
            &[
                ("type", "guid"),
                ("count", "1"),
                ("query", video_id.as_str()),
                ("fields", FIELDS),
            ],
        );
        let response = download_json(client, &api, NAME, &video_id)?;
        let info = match response.get("items") {
            // An uncaught `IndexError` is reported as its message.
            Some(Value::Array(items)) => items
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            Some(_) => return Err(unsupported("item lists that are not lists")),
            None => {
                return Err(Error::Reported(format!(
                    "{video_id}: An extractor error has occurred. (caused by KeyError('items')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        // `info.get('name', info.get('titletag'))`.
        let title = match info.get("name") {
            Some(name) => name.clone(),
            None => info.get("titletag").cloned().unwrap_or(Value::Null),
        };
        let mut formats = Vec::new();
        if let Some(f4m_url) = info
            .get("videoURL")
            .filter(|value| super::nuxt::truthy(value))
        {
            let manifest = f4m_url
                .as_str()
                .and_then(|url| Url::parse(url).ok())
                .ok_or_else(|| unsupported("HDS URLs that are not URLs"))?;
            formats.extend(super::extract_f4m_optional(
                client,
                NAME,
                &manifest,
                &video_id,
                Some("hds"),
            )?);
        }
        if let Some(m3u8_url) = info.get("hls").filter(|value| super::nuxt::truthy(value)) {
            let manifest = m3u8_url
                .as_str()
                .and_then(|url| Url::parse(url).ok())
                .ok_or_else(|| unsupported("HLS URLs that are not URLs"))?;
            if let Some(stream) = super::extract_m3u8_master_optional(
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
        let mp4_list = match info.get("videoMP4List") {
            None => Vec::new(),
            Some(Value::Array(items)) => items.clone(),
            Some(_) => return Err(unsupported("MP4 lists that are not lists")),
        };
        for item in &mp4_list {
            let Some(mp4_url) = item.get("url").filter(|value| super::nuxt::truthy(value)) else {
                continue;
            };
            let tbr = int_or_none(item.get("bitrate"));
            let fps = match item.get("fps") {
                Some(Value::Number(number)) => number.as_f64(),
                Some(Value::String(text)) => text.trim().parse().ok(),
                _ => None,
            };
            let opt = |value: Option<i64>| value.map_or(Value::Null, Value::from);
            let mut format = dict_format(
                vec![
                    ("url", mp4_url.clone()),
                    (
                        "format_id",
                        tbr.filter(|tbr| *tbr != 0)
                            .map_or_else(|| "http".to_owned(), |tbr| format!("http-{tbr}"))
                            .into(),
                    ),
                    ("tbr", opt(tbr)),
                    ("width", opt(int_or_none(item.get("width")))),
                    ("height", opt(int_or_none(item.get("height")))),
                    ("fps", fps.map_or(Value::Null, |fps| json!(fps))),
                ],
                NAME,
                &video_id,
            )?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
        let get = |key: &str| info.get(key).cloned().unwrap_or(Value::Null);
        // The API's thumbnail dicts keep their own key order.
        let mut thumbnail_keys: Vec<String> = Vec::new();
        if let Some(Value::Array(thumbnails)) = info.get("thumbnailList") {
            for key in thumbnails
                .iter()
                .filter_map(Value::as_object)
                .flat_map(|map| map.keys())
            {
                if !thumbnail_keys.contains(key) {
                    thumbnail_keys.push(key.clone());
                }
            }
        }
        let mut result = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("formats", Value::Null),
                ("thumbnails", get("thumbnailList")),
                ("creator", get("author")),
                ("uploader_id", get("editor")),
                (
                    "duration",
                    int_or_none(info.get("duration")).map_or(Value::Null, Value::from),
                ),
                (
                    "upload_date",
                    info.get("formattedCreationDate")
                        .and_then(Value::as_str)
                        .and_then(|date| crate::dates::unified_strdate(date, false))
                        .map_or(Value::Null, Value::from),
                ),
                ("title", title),
                ("categories", get("keywords")),
            ],
            formats,
        );
        if !thumbnail_keys.is_empty()
            && let Some(Value::String(shape)) = result.extra.get_mut(crate::info_order::INFO_SHAPE)
        {
            shape.push_str(&format!(";thumbnail={}", thumbnail_keys.join(",")));
        }
        Ok(Some(result))
    }
}

impl Extractor for WsjArticle {
    fn key(&self) -> &'static str {
        ARTICLE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ARTICLE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let article_id = found["id"].to_owned();
        start(ARTICLE_NAME, url);
        // `_download_webpage(..., impersonate=True)`.
        let webpage = {
            let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
                crate::screen::warning_once(&format!("[{ARTICLE_NAME}] {message}"));
            });
            download_webpage(client, url, ARTICLE_NAME, &article_id)?
        };
        let video_id = VIDEO_ID
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{ARTICLE_NAME}] {article_id}: Unable to extract video id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // `url_result(f'wsj:{video_id}', WSJIE.ie_key(), video_id)`.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.id = video_id.clone();
        follow
            .extra
            .insert(crate::FOLLOW_URL.into(), format!("wsj:{video_id}").into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "http://video-api.wsj.com/api-video/player/iframe.html?guid=1BD01A4C-BFE8-40A5-A42F-8A8AF9898B1A",
            "http://www.wsj.com/video/can-alphabet-build-a-smarter-city/359DDAA8-9AC1-489C-82E6-0429C1E430E0.html",
            "wsj:A5A9E4C6-3F2D-4A6B-9E0F-1234567890AB",
            "https://www.barrons.com/video/capitals-ceo/3B8C5D6E-1234-4ABC-9DEF-0123456789AB",
        ] {
            assert!(suitable(url), "{url}");
        }
        assert!(article_suitable(
            "https://www.wsj.com/articles/dont-like-china-no-pandas-for-you-1490366939"
        ));
        assert!(!article_suitable("https://www.wsj.com/news/"));
    }
}
