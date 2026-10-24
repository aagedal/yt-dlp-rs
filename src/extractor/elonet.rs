//! Upstream `ElonetIE` (`yt_dlp/extractor/elonet.py`): a record page's
//! `video-data` sources (the first one's HLS or DASH manifest) and its
//! OpenGraph metadata.
use super::{
    Extractor, dict_info, download_webpage, extract_m3u8_master_optional, extract_mpd_optional,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Elonet;

const NAME: &str = "Elonet";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://elonet\.finna\.fi/Record/kavi\.elonet_elokuva_(?P<id>[0-9]+)").unwrap()
});
static SOURCES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"id='video-data'[^>]+data-video-sources="([^"]+)""#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Elonet {what} are not supported natively"))
}

fn bug(id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for Elonet {
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
        let json = SOURCES
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .ok_or_else(|| bug(&video_id, "Unable to extract json"))?;
        let sources: Value = crate::json::from_str(&json).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let src = match sources.get(0).and_then(|source| source.get("src")) {
            Some(Value::String(src)) => src.clone(),
            _ => return Err(unsupported("video sources without a string src")),
        };
        let ext = super::direct::determine_ext(&src).unwrap_or_default();
        let manifest = Url::parse(&src)?;
        let stream = match ext.as_str() {
            "m3u8" => {
                extract_m3u8_master_optional(client, &manifest, NAME, &video_id, None, true, false)
            }
            "mpd" => extract_mpd_optional(client, &manifest, NAME, &video_id, true),
            _ => return Err(bug(&video_id, &format!("Unknown streaming format {ext}"))),
        };
        let (formats, subtitles) = stream.map_or_else(Default::default, |stream| {
            (stream.formats, stream.subtitles)
        });
        let title = crate::html::og_search_property(&webpage, "title")
            .ok_or_else(|| bug(&video_id, "Unable to extract OpenGraph title"))?;
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            super::search_warning(NAME, "OpenGraph description");
        }
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            super::search_warning(NAME, "thumbnail URL");
        }
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("description", description.map_or(Value::Null, Value::from)),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://elonet.finna.fi/Record/kavi.elonet_elokuva_107867"
        ));
        assert!(!suitable("https://elonet.finna.fi/Record/other"));
    }
}
