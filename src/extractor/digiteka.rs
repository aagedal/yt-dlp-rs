//! Upstream `DigitekaIE` (`yt_dlp/extractor/digiteka.py`): Digiteka and
//! Ultimedia videos from the player `getConf` JSON (non-fatal HLS and the
//! MP4 renditions); Ultimedia embed iframes and scripts on other pages are
//! followed.
use super::{Extractor, dict_format, dict_info, download_json_note, extract_m3u8_master_optional};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Digiteka;

pub(crate) const NAME: &str = "Digiteka";
pub(crate) const KEY: &str = "Digiteka";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?(?:digiteka\.net|ultimedia\.com)/(?:deliver/(?P<embed_type>generic|musique)(?:/[^/]+)*/(?:src|article)|default/index/video(?P<site_type>generic|music)/id)/(?P<id>[\d+a-z]+)",
    )
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<(?:iframe|script)[^>]+src=["'](?P<url>(?:https?:)?//(?:www\.)?ultimedia\.com/deliver/(?:generic|musique)(?:/[^/]+)*/(?:src|article)/[\d+a-z]+)"#,
    )
    .unwrap()
});

/// One static ID working for Ultimedia iframes.
const IFRAME_MD_ID: &str = "01836272";

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

impl Extractor for Digiteka {
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
        let api = Url::parse(&format!(
            "https://www.ultimedia.com/player/getConf/{IFRAME_MD_ID}/1/{video_id}"
        ))?;
        let config = download_json_note(
            client,
            &api,
            NAME,
            &video_id,
            "Downloading player configuration",
        )?;
        let video_info = config.get("video").ok_or_else(|| {
            Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('video')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        if let Some(hls_url) = video_info
            .pointer("/media_sources/hls/hls_auto")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            && let Some(stream) = extract_m3u8_master_optional(
                client,
                &Url::parse(&hls_url)?,
                NAME,
                &video_id,
                Some("hls"),
                true,
                false,
            )
        {
            formats.extend(stream.formats);
            subtitles = stream.subtitles;
        }
        if let Some(Value::Object(mp4)) = video_info.pointer("/media_sources/mp4") {
            for (format_id, mp4_url) in mp4 {
                if !super::nuxt::truthy(mp4_url) {
                    continue;
                }
                let height = format_id
                    .split_once('_')
                    .and_then(|(_, height)| height.trim().parse::<i64>().ok());
                formats.push(dict_format(
                    vec![
                        ("url", mp4_url.clone()),
                        ("format_id", format_id.clone().into()),
                        ("height", height.map_or(Value::Null, Value::from)),
                        ("ext", "mp4".into()),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        let text = |key: &str| {
            video_info
                .get(key)
                .filter(|value| value.is_string())
                .cloned()
        };
        for (key, value) in [
            ("title", text("title")),
            (
                "thumbnail",
                video_info
                    .get("image")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .map(Value::from),
            ),
            (
                "duration",
                int_or_none(video_info.get("duration")).map(Value::from),
            ),
            (
                "timestamp",
                int_or_none(video_info.get("creationDate")).map(Value::from),
            ),
            ("uploader_id", text("ownerId")),
        ] {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
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
            "https://www.ultimedia.com/default/index/videogeneric/id/3x5x55k"
        ));
        assert!(suitable(
            "https://www.ultimedia.com/deliver/generic/iframe/mdtk/01601930/src/lqm3kl/zone/1/showtitle/1"
        ));
        assert!(!suitable(
            "https://www.ultimedia.com/default/index/videogeneric"
        ));
    }
}
