//! Upstream `CAM4IE` (`yt_dlp/extractor/cam4.py`): a channel's
//! `streamInfo` `cdnURL` as live HLS formats.
use super::{Extractor, download_json, extract_m3u8_fatal_live};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Cam4;

const NAME: &str = "CAM4";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:[^/]+\.)?cam4\.com/(?P<id>[a-z0-9_]+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Cam4 {
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
        let api = Url::parse(&format!(
            "https://www.cam4.com/rest/v1.0/profile/{id}/streamInfo"
        ))?;
        let data = download_json(client, &api, NAME, &id)?;
        let manifest = data
            .get("cdnURL")
            .and_then(Value::as_str)
            .and_then(|text| Url::parse(text).ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(format!(
                    "CAM4 stream info without an absolute cdnURL is not supported natively: {id}"
                ))
            })?;
        let mut info = extract_m3u8_fatal_live(client, &manifest, NAME, &id, Some("hls"), false)?;
        // `ext='mp4'`.
        for format in &mut info.formats {
            format.ext = "mp4".into();
        }
        info.ext = "mp4".into();
        info.id = id.clone();
        info.title = id.clone();
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.age_limit = Some(18);
        info.metadata.thumbnail = Some(format!("https://snapshots.xcdnpro.com/thumbnails/{id}"));
        crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(true));
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,is_live,age_limit,formats,thumbnail",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.cam4.com/foxynesss"));
        assert!(suitable("https://cam4.com/some_user"));
        assert!(!suitable("https://www.cam4.com/"));
    }
}
