//! Upstream `ElTreceTVIE` (`yt_dlp/extractor/eltrecetv.py`): El Trece TV
//! episodes from the page's `Fusion.globalContent` player config: its HLS
//! playlist plus progressive copies of the `tracks-v1a1` variants below
//! 1080p, with the config's title and thumbnail.
use super::{Extractor, dict_format, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct ElTreceTv;

const NAME: &str = "ElTreceTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?eltrecetv\.com\.ar/[\w-]+/capitulos/temporada-\d+/(?P<id>[\w-]+)",
    )
    .unwrap()
});
static CONTENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Fusion.globalContent\s*=\s*").unwrap());
static VIDEO_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/(\w+)\.m3u8").unwrap());
const TRACKS: &str = "/tracks-v1a1/index.m3u8";

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("El Trece TV {what} are not supported natively"))
}

impl Extractor for ElTreceTv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let slug = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &slug)?;
        let content = super::search_json(&CONTENT, &webpage, false)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {slug}: Unable to extract content; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .ok_or_else(|| unsupported("contents that do not parse"))?;
        let mut config = &content;
        for key in ["promo_items", "basic", "embed", "config"] {
            config = config.get(key).ok_or_else(|| {
                Error::Reported(format!(
                    "{slug}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        }
        let video_url = match config.get("m3u8") {
            Some(Value::String(video_url)) => video_url.clone(),
            Some(_) => return Err(unsupported("playlist URLs that are not strings")),
            None => {
                return Err(Error::Reported(format!(
                    "{slug}: An extractor error has occurred. (caused by KeyError('m3u8')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let video_id = VIDEO_ID
            .captures(&video_url)
            .map_or_else(|| slug.clone(), |found| found[1].to_owned());
        let manifest =
            Url::parse(&video_url).map_err(|_| unsupported("playlist URLs that are not URLs"))?;
        let stream =
            extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, Some("hls"), true)?;
        let mut formats = stream.formats;
        let mut progressive = Vec::new();
        for format in &formats {
            let variant = format.url.as_str();
            if !variant.ends_with(TRACKS) || format.height == Some(1080) {
                continue;
            }
            let null_or = |value: Option<u32>| value.map_or(Value::Null, Value::from);
            progressive.push(dict_format(
                vec![
                    ("url", variant[..variant.len() - TRACKS.len()].into()),
                    ("format_id", format.format_id.replace("hls", "http").into()),
                    ("width", null_or(format.width)),
                    ("height", null_or(format.height)),
                ],
                NAME,
                &video_id,
            )?);
        }
        formats.extend(progressive);
        let get = |key: &str| config.get(key).cloned().unwrap_or(Value::Null);
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", get("title")),
                ("thumbnail", get("thumbnail")),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
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
            "https://www.eltrecetv.com.ar/ahora-caigo/capitulos/temporada-2023/programa-del-061023/"
        ));
        assert!(!suitable("https://www.eltrecetv.com.ar/ahora-caigo/"));
    }
}
