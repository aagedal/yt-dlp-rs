//! Upstream `AitubeKZVideoIE` (`yt_dlp/extractor/aitube.py`): Aitube.kz
//! videos from the static-access HLS playlist, with the Next.js `videoInfo`
//! metadata merged (`merge_dicts`) with the page's JSON-LD.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct AitubeKzVideo;

const NAME: &str = "AitubeKZVideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://aitube\.kz/(?:video|embed/)\?(?:[^\?]+)?id=(?P<id>[\w-]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_html_search_meta(names, webpage)`: the first name present, else a
/// warning naming the first.
fn search_meta(webpage: &str, names: &[&str]) -> Option<String> {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name));
    if found.is_none() {
        super::search_warning(NAME, names[0]);
    }
    found
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

impl Extractor for AitubeKzVideo {
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
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        let mut data = &next_data;
        for key in ["props", "pageProps", "videoInfo"] {
            data = data.get(key).ok_or_else(|| key_error(&video_id, key))?;
        }
        // `_search_json_ld(webpage, video_id)` (fatal).
        if super::json_ld::yield_json_ld(&webpage).is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract JSON-LD; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let json_ld = super::json_ld::search(&webpage);
        let manifest = Url::parse(&format!(
            "https://api-http.aitube.kz/kz.aitudala.aitube.staticaccess/video/{video_id}/video"
        ))?;
        let mut stream = extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, true)?;
        super::hls::without_ext(&mut stream);
        let get = |key: &str| data.get(key).filter(|value| !value.is_null()).cloned();
        let truthy = |key: &str| {
            data.get(key)
                .filter(|value| super::nuxt::truthy(value))
                .cloned()
        };
        let title = truthy("title")
            .or_else(|| search_meta(&webpage, &["name", "og:title"]).map(Value::from));
        let view_count = truthy("viewCount").or_else(|| {
            search_meta(&webpage, &["ya:ovs:views_total"])
                .and_then(|views| views.trim().parse::<i64>().ok())
                .map(Value::from)
        });
        // `merge_dicts`: `None` values are skipped, and JSON-LD fills the
        // missing keys (or replaces empty strings).
        let mut fields: Vec<(String, Value)> = Vec::new();
        for (key, value) in [
            ("id", Some(Value::from(video_id.clone()))),
            ("title", title),
            ("description", get("description")),
            ("formats", Some(Value::Null)),
            ("subtitles", Some(Value::Null)),
            ("view_count", view_count),
            ("like_count", get("likeCount")),
            ("channel", get("channelTitle")),
            ("channel_id", get("channelId")),
            ("thumbnail", get("coverUrl")),
            ("comment_count", get("commentCount")),
            (
                "channel_follower_count",
                int_or_none(data.get("channelSubscriberCount")).map(Value::from),
            ),
        ] {
            if let Some(value) = value {
                fields.push((key.to_owned(), value));
            }
        }
        for (key, value) in json_ld {
            match fields.iter_mut().find(|(name, _)| *name == key) {
                Some((_, old)) => {
                    if value.is_string() && old.as_str() == Some("") {
                        *old = value;
                    }
                }
                None => fields.push((key, value)),
            }
        }
        let fields: Vec<(&str, Value)> = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        let mut info = dict_info(url, (NAME, NAME), fields, stream.formats);
        info.subtitles = stream.subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://aitube.kz/video?id=9291d29b-c038-49a1-ad42-3da2051d353c&playlistId=x"
        ));
        assert!(suitable(
            "https://aitube.kz/embed/?id=9291d29b-c038-49a1-ad42-3da2051d353c"
        ));
        assert!(suitable("https://aitube.kz/video?playlistId=x&id=9291d29b"));
        assert!(!suitable("https://aitube.kz/channel?id=x"));
    }
}
