//! Upstream `MediaWorksNZVODIE` (`yt_dlp/extractor/mediaworksnz.py`):
//! MediaWorks NZ VOD assets from the published-asset API: its HLS playlist
//! (and audio-only playlist as AAC), DRM and content-type checks, and
//! metadata; JW player divs with the asset request URL are embeds.
use super::{Extractor, dict_info, download_json, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct MediaWorksNzVod;

pub(crate) const NAME: &str = "MediaWorksNZVOD";
pub(crate) const KEY: &str = "MediaWorksNZVOD";

const BASE: &str = r"https?://vodupload-api\.mediaworks\.nz/library/asset/published/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(r"^{BASE}(?P<id>[A-Za-z0-9-]+)")).unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"<div\s+\bid=["']Player-Attributes-JWID[^>]+\bdata-request-url=["']{BASE}["'][^>]+\bdata-asset-id=["'](?P<id>[A-Za-z0-9-]+)["']"#
    ))
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls`.
pub(crate) fn embed_urls(_url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Ok(embed) = Url::parse(&format!(
            "https://vodupload-api.mediaworks.nz/library/asset/published/{}",
            &captures["id"]
        )) else {
            continue;
        };
        if !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MediaWorks NZ {what} are not supported natively"))
}

impl Extractor for MediaWorksNzVod {
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
        let response = download_json(client, url, NAME, &video_id)?;
        let asset = response.get("asset").ok_or_else(|| {
            Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('asset')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        match asset.get("drm") {
            None | Some(Value::Null) => {}
            Some(Value::String(drm)) if drm == "NonDRM" => {}
            Some(_) => crate::no_formats::raise_expected(
                NAME,
                Some(&video_id),
                None,
                "This video is DRM protected",
            )?,
        }
        if let Some(content_type) = asset.get("type").filter(|value| super::nuxt::truthy(value))
            && content_type.as_str() != Some("video")
        {
            let text = match content_type {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: Unknown content type: {text}; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let manifest = match asset.get("streamingUrl") {
            Some(Value::String(manifest)) => {
                Url::parse(manifest).map_err(|_| unsupported("streaming URLs that are not URLs"))?
            }
            Some(_) => return Err(unsupported("streaming URLs that are not strings")),
            None => {
                return Err(Error::Reported(format!(
                    "{video_id}: An extractor error has occurred. (caused by KeyError('streamingUrl')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let mut stream = extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, true)?;
        super::hls::without_ext(&mut stream);
        let mut formats = stream.formats;
        let audio = ["palyoutPathAudio", "playoutpathaudio"]
            .iter()
            .find_map(|key| asset.get(*key).and_then(Value::as_str))
            .filter(|audio| !audio.is_empty())
            .and_then(|audio| Url::parse(audio).ok());
        if let Some(audio) = audio
            && let Some(audio_stream) = super::extract_m3u8_master_optional(
                client, &audio, NAME, &video_id, None, false, false,
            )
        {
            for mut format in audio_stream.formats {
                format.ext = "mp3".into();
                // `setdefault`: keys the parser did not set are added last.
                if format.vcodec.is_none() {
                    format.vcodec = Some("none".into());
                    crate::info_order::mark_format_update(&mut format, "+vcodec");
                }
                if format.acodec.is_none() {
                    format.acodec = Some("aac".into());
                    crate::info_order::mark_format_update(&mut format, "+acodec");
                }
                formats.push(format);
            }
        }
        let get = |key: &str| asset.get(key).cloned().unwrap_or(Value::Null);
        let duration = match asset.get("duration") {
            Some(Value::Number(number)) => number.as_f64().map(|value| json!(value)),
            Some(Value::String(text)) => text.trim().parse::<f64>().ok().map(|value| json!(value)),
            _ => None,
        }
        .unwrap_or(Value::Null);
        let timestamp = asset
            .get("dateadded")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, false))
            .map_or(Value::Null, Value::from);
        let thumbnails: Vec<Value> = match asset.get("thumbnails") {
            Some(Value::Array(thumbnails)) => thumbnails
                .iter()
                .map(|thumbnail| json!({"url": thumbnail}))
                .collect(),
            _ => Vec::new(),
        };
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", get("title")),
                ("description", get("description")),
                ("duration", duration),
                ("timestamp", timestamp),
                ("channel", get("brand")),
                ("thumbnails", Value::Array(thumbnails)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        info.subtitles = stream.subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=url");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://vodupload-api.mediaworks.nz/library/asset/published/VID00359"
        ));
        assert!(!suitable("https://vodupload-api.mediaworks.nz/library/"));
    }
}
