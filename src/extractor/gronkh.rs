//! Upstream `GronkhIE` (`yt_dlp/extractor/gronkh.py`): the video info and
//! playlist APIs, the playlist's HLS formats and the info's VTT subtitles
//! and chapters. The feed and VOD listing extractors are not ported.
use super::clyp::{float_or_none, key_error};
use super::{Extractor, dict_info, download_json, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Gronkh;

const NAME: &str = "Gronkh";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?gronkh\.tv/(?:watch/)?streams?/(?P<id>\d+)")
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Gronkh {
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
        let api = |kind: &str| {
            Url::parse(&format!(
                "https://api.gronkh.tv/v1/video/{kind}?episode={id}"
            ))
        };
        let data = download_json(client, &api("info")?, NAME, &id)?;
        let playlist = download_json(client, &api("playlist")?, NAME, &id)?;
        let manifest = playlist
            .get("playlist_url")
            .ok_or_else(|| key_error(&id, "playlist_url"))?
            .as_str()
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Gronkh playlist URLs that are not strings are not supported natively".into(),
                )
            })?;
        let mut stream = extract_m3u8_fatal(client, &Url::parse(manifest)?, NAME, &id, None, true)?;
        super::hls::without_ext(&mut stream);
        let mut subtitles = stream.subtitles;
        if let Some(vtt) = data
            .get("vtt_url")
            .filter(|vtt| super::lnk::truthy(Some(vtt)))
        {
            let vtt = vtt.as_str().ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Gronkh VTT URLs that are not strings are not supported natively".into(),
                )
            })?;
            let track: crate::SubtitleFormat =
                serde_json::from_value(serde_json::json!({"url": Url::parse(vtt)?, "ext": "vtt"}))
                    .map_err(|error| Error::Reported(error.to_string()))?;
            match subtitles
                .entries
                .iter_mut()
                .find(|(language, _)| language.as_deref() == Some("en"))
            {
                Some((_, tracks)) => tracks.push(track),
                None => subtitles.entries.push((Some("en".into()), vec![track])),
            }
        }
        // `('chapters', lambda _, v: float_or_none(v['offset']) is not None,
        // {'title': 'title', 'start_time': ('offset', {float_or_none})})`.
        let chapters: Vec<Value> = data
            .get("chapters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|chapter| {
                let start = float_or_none(chapter.get("offset"));
                if start.is_null() {
                    return None;
                }
                let mut entry = Map::new();
                if let Some(title) = chapter.get("title").filter(|title| !title.is_null()) {
                    entry.insert("title".into(), title.clone());
                }
                entry.insert("start_time".into(), start);
                Some(Value::Object(entry))
            })
            .collect();
        let get = |key: &str| data.get(key).cloned().unwrap_or(Value::Null);
        let upload_date = data
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_strdate(date, true))
            .map_or(Value::Null, Value::from);
        let mut fields = vec![
            ("id", id.clone().into()),
            ("title", get("title")),
            ("view_count", get("views")),
            ("thumbnail", get("preview_url")),
            ("upload_date", upload_date),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("duration", float_or_none(data.get("source_length"))),
            (
                "chapters",
                if chapters.is_empty() {
                    Value::Null
                } else {
                    Value::Array(chapters)
                },
            ),
        ];
        // Processing drops a `None` duration.
        fields.retain(|(key, value)| *key != "duration" || !value.is_null());
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("dict;{};chapter=title,start_time", keys.join(","));
        let info = dict_info(url, (NAME, NAME), fields, stream.formats);
        let mut info = crate::info_order::mark_info(info, &shape);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://gronkh.tv/streams/657"));
        assert!(suitable("https://gronkh.tv/stream/536"));
        assert!(suitable("https://gronkh.tv/watch/stream/546"));
        assert!(!suitable("https://gronkh.tv/feed"));
    }
}
