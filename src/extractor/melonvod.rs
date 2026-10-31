//! Upstream `MelonVODIE` (`yt_dlp/extractor/melonvod.py`): Melon music
//! videos from the player info (title, artists) and streaming info (HLS
//! playlist, image, play time and open date) JSON.
use super::{Extractor, dict_info, download_json_note, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MelonVod;

const NAME: &str = "MelonVOD";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://vod\.melon\.com/video/detail2\.html?\?.*?mvId=(?P<id>[0-9]+)",
    )
    .unwrap()
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

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Melon {what} are not supported natively"))
}

impl Extractor for MelonVod {
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
        let player = super::update_url_query(
            &Url::parse("http://vod.melon.com/video/playerInfo.json")?,
            &[("mvId", video_id.as_str())],
        );
        let play_info = download_json_note(
            client,
            &player,
            NAME,
            &video_id,
            "Downloading player info JSON",
        )?;
        let title = play_info
            .get("mvInfo")
            .ok_or_else(|| key_error(&video_id, "mvInfo"))?
            .get("MVTITLE")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "MVTITLE"))?;
        let streaming = super::update_url_query(
            &Url::parse("http://vod.melon.com/delivery/streamingInfo.json")?,
            &[("contsId", video_id.as_str()), ("contsType", "VIDEO")],
        );
        let info = download_json_note(
            client,
            &streaming,
            NAME,
            &video_id,
            "Downloading streaming info JSON",
        )?;
        let stream_info = info
            .get("streamingInfo")
            .ok_or_else(|| key_error(&video_id, "streamingInfo"))?;
        let manifest = match stream_info.get("encUrl") {
            Some(Value::String(manifest)) => {
                Url::parse(manifest).map_err(|_| unsupported("playlist URLs that are not URLs"))?
            }
            Some(_) => return Err(unsupported("playlist URLs that are not strings")),
            None => return Err(key_error(&video_id, "encUrl")),
        };
        let stream =
            extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, Some("hls"), false)?;
        let artist = match play_info.get("artistList") {
            Some(Value::Array(artists)) => {
                let mut names = Vec::new();
                for artist in artists {
                    match artist.get("ARTISTNAMEWEBLIST") {
                        Some(name) if super::nuxt::truthy(name) => match name {
                            Value::String(name) => names.push(name.clone()),
                            _ => return Err(unsupported("artist names that are not strings")),
                        },
                        _ => {}
                    }
                }
                Value::from(names.join(", "))
            }
            _ => Value::Null,
        };
        let thumbnail = stream_info
            .get("imgPath")
            .and_then(Value::as_str)
            .and_then(|path| {
                super::videopress::urljoin(info.get("staticDomain").and_then(Value::as_str), path)
            })
            .map_or(Value::Null, Value::from);
        let duration = match stream_info.get("playTime") {
            Some(Value::Number(number)) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
            Some(Value::String(text)) => text.trim().parse().ok(),
            _ => None,
        }
        .map_or(Value::Null, Value::from);
        // `stream_info.get('mvSvcOpenDt', '')[:8] or None`.
        let upload_date = match stream_info.get("mvSvcOpenDt") {
            None => Value::Null,
            Some(Value::String(date)) => {
                let date: String = date.chars().take(8).collect();
                if date.is_empty() {
                    Value::Null
                } else {
                    date.into()
                }
            }
            Some(_) => return Err(unsupported("open dates that are not strings")),
        };
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("artist", artist),
                ("thumbnail", thumbnail),
                ("upload_date", upload_date),
                ("duration", duration),
                ("formats", Value::Null),
            ],
            stream.formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://vod.melon.com/video/detail2.htm?mvId=50158734"
        ));
        assert!(!suitable("http://vod.melon.com/video/detail2.htm?x=1"));
    }
}
