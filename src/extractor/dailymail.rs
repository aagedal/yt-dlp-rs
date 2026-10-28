//! Upstream `DailyMailIE` (`yt_dlp/extractor/dailymail.py`): Daily Mail
//! videos and embeds from the page's `data-opts` player options and their
//! `video-sources.json` renditions; embed iframes on other pages are
//! followed.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct DailyMail;

pub(crate) const NAME: &str = "DailyMail";
pub(crate) const KEY: &str = "DailyMail";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?dailymail\.co\.uk/(?:video/[^/]+/video-|embed/video/)(?P<id>[0-9]+)",
    )
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe\b[^>]+\bsrc=["'](?P<url>(?:https?:)?//(?:www\.)?dailymail\.co\.uk/embed/video/\d+\.html)"#,
    )
    .unwrap()
});
static DATA_OPTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"data-opts='(\{.+?\})'").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Ok(embed) = url.join(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Daily Mail {what} are not supported natively"))
}

/// `int_or_none(value, scale)`: Python `int(value) // scale`.
fn int_or_none(value: Option<&Value>, scale: i64) -> Option<i64> {
    let value = match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))?,
        Value::String(text) => text.trim().parse().ok()?,
        Value::Bool(value) => i64::from(*value),
        _ => return None,
    };
    Some(value.div_euclid(scale))
}

/// `determine_protocol({'url': url})`.
fn determine_protocol(url: &str) -> String {
    let url = if url.starts_with("//") {
        format!("http:{url}")
    } else {
        url.to_owned()
    };
    for scheme in ["rtmp", "mms", "rtsp"] {
        if url.starts_with(scheme) {
            return scheme.into();
        }
    }
    match super::direct::determine_ext(&url).as_deref() {
        Some("m3u8") => "m3u8_native".into(),
        Some("f4m") => "f4m".into(),
        _ => url
            .split_once(':')
            .map_or("", |(scheme, _)| scheme)
            .to_lowercase(),
    }
}

impl Extractor for DailyMail {
    fn key(&self) -> &'static str {
        KEY
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
        let opts = DATA_OPTS.captures(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract video data; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let video_data: Value = crate::json::from_str(&opts[1])
            .map_err(|_| unsupported("player options that do not parse"))?;
        let title = match video_data.get("title") {
            Some(Value::String(title)) => crate::html::unescape_html(title),
            Some(_) => return Err(unsupported("titles that are not strings")),
            None => return Err(key_error(&video_id, "title")),
        };
        let sources_url = video_data
            .pointer("/plugins/sources/url")
            .and_then(Value::as_str)
            .or_else(|| video_data.pointer("/sources/url").and_then(Value::as_str))
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!("http://www.dailymail.co.uk/api/player/{video_id}/video-sources.json")
            });
        let sources_url = url
            .join(&sources_url)
            .map_err(|_| unsupported("source URLs that are not URLs"))?;
        let mut video_sources = download_json(client, &sources_url, NAME, &video_id)?;
        if let Some(body) = video_sources
            .get("body")
            .filter(|body| super::nuxt::truthy(body))
            .cloned()
        {
            video_sources = body;
        }
        let renditions = match video_sources.get("renditions") {
            Some(Value::Array(renditions)) => renditions.clone(),
            Some(_) => return Err(unsupported("rendition lists that are not lists")),
            None => return Err(key_error(&video_id, "renditions")),
        };
        let mut formats = Vec::new();
        for rendition in &renditions {
            let Some(rendition_url) = rendition.get("url").filter(|url| super::nuxt::truthy(url))
            else {
                continue;
            };
            let Some(rendition_url) = rendition_url.as_str() else {
                return Err(unsupported("rendition URLs that are not strings"));
            };
            let tbr = int_or_none(rendition.get("encodingRate"), 1000);
            let container = rendition
                .get("videoContainer")
                .cloned()
                .unwrap_or(Value::Null);
            let is_hls = container.as_str() == Some("M2TS");
            let protocol = if is_hls {
                "m3u8_native".to_owned()
            } else {
                determine_protocol(rendition_url)
            };
            let format_id = [
                Some(if is_hls {
                    "hls".to_owned()
                } else {
                    protocol.clone()
                })
                .filter(|part| !part.is_empty()),
                tbr.filter(|tbr| *tbr != 0).map(|tbr| tbr.to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("-");
            let opt = |value: Option<i64>| value.map_or(Value::Null, Value::from);
            let mut format = dict_format(
                vec![
                    ("format_id", format_id.into()),
                    // Processing's `sanitize_url`.
                    ("url", super::proto_relative_http(rendition_url).into()),
                    ("width", opt(int_or_none(rendition.get("frameWidth"), 1))),
                    ("height", opt(int_or_none(rendition.get("frameHeight"), 1))),
                    ("tbr", opt(tbr)),
                    (
                        "vcodec",
                        rendition.get("videoCodec").cloned().unwrap_or(Value::Null),
                    ),
                    ("container", container),
                    ("protocol", protocol.into()),
                    ("ext", if is_hls { "mp4".into() } else { Value::Null }),
                ],
                NAME,
                &video_id,
            )?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
        let description = match video_data.get("descr") {
            Some(Value::String(text)) => Value::from(crate::html::unescape_html(text)),
            _ => Value::Null,
        };
        let thumbnail = video_data
            .get("poster")
            .filter(|value| super::nuxt::truthy(value))
            .or_else(|| video_data.get("thumbnail"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("description", description),
                ("thumbnail", thumbnail),
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
            "http://www.dailymail.co.uk/video/tvshowbiz/video-1295863/The-Mountain-appears-sparkling-water-ad-Heavy-Bubbles.html"
        ));
        assert!(suitable(
            "http://www.dailymail.co.uk/embed/video/1295863.html"
        ));
        assert!(!suitable("http://www.dailymail.co.uk/news/index.html"));
    }

    #[test]
    fn protocols_follow_determine_protocol() {
        assert_eq!(determine_protocol("https://a.test/v.mp4"), "https");
        assert_eq!(determine_protocol("//a.test/v.m3u8"), "m3u8_native");
        assert_eq!(determine_protocol("rtmpe://a.test/v"), "rtmp");
    }
}
