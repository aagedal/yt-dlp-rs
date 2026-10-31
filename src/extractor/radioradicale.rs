//! Upstream `RadioRadicaleIE` (`yt_dlp/extractor/radioradicale.py`): the
//! page's Drupal settings `RRscheda` playlist, each item's HLS sources, and
//! the OpenGraph and JSON-LD metadata (one video, or a `multi_video`
//! playlist of items).
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct RadioRadicale;

const NAME: &str = "RadioRadicale";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?radioradicale\.it/scheda/(?P<id>[0-9]+)")
        .unwrap()
});
static SETTINGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"jQuery\.extend\(Drupal\.settings\s*,").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Radio Radicale {what} are not supported natively"))
}

/// A list's items or a dict's values (`...`); nothing for other values.
fn branch(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map.values().collect(),
        _ => Vec::new(),
    }
}

/// `{'id', 'title', 'formats', 'subtitles'}` of one playlist item.
fn entry(client: &Client, page: &Url, video: &Value, video_id: &str) -> Result<VideoInfo> {
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for source in video.get("sources").map(branch).unwrap_or_default() {
        let Some(m3u8_url) = source
            .get("src")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let manifest = Url::parse(&m3u8_url)?;
        let stream =
            super::extract_m3u8_fatal_master(client, &manifest, NAME, video_id, None, true)?;
        formats.extend(stream.formats);
        super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
    }
    // `('subtitles', ..., lambda _, v: url_or_none(v['src']))` filters the
    // members of each track (a dict's values are its strings), so tracks
    // given as objects never match.
    let nested = video.get("subtitles").map(branch).unwrap_or_default();
    if nested.iter().any(|track| {
        branch(track).iter().any(|value| {
            value
                .get("src")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .is_some()
        })
    }) {
        return Err(unsupported("nested subtitle track lists"));
    }
    for format in &mut formats {
        if format.ext.is_empty() || format.ext == "unknown_video" {
            format.ext = "mp4".into();
        }
    }
    let mut info = dict_info(
        page,
        (NAME, NAME),
        vec![
            ("id", video_id.into()),
            ("title", video.get("title").cloned().unwrap_or(Value::Null)),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ],
        formats,
    );
    info.subtitles = subtitles;
    Ok(info)
}

impl Extractor for RadioRadicale {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            Some(_) => Err(unsupported("playlists in a video-only context")),
            None => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let page_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &page_id)?;

        let settings = super::search_json(&SETTINGS, &webpage, false)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {page_id}: Unable to extract videos_info; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .ok_or_else(|| unsupported("settings that are not JSON"))?;
        let videos_info = settings.get("RRscheda").ok_or_else(|| {
            Error::Reported(format!(
                "{page_id}: An extractor error has occurred. (caused by KeyError('RRscheda')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;

        // `('playlist', lambda _, v: v['sources'])`: items with truthy sources.
        let videos: Vec<&Value> = videos_info
            .get("playlist")
            .map(branch)
            .unwrap_or_default()
            .into_iter()
            .filter(|video| video.get("sources").is_some_and(super::nuxt::truthy))
            .collect();
        let entries = videos
            .iter()
            .enumerate()
            .map(|(index, video)| entry(client, url, video, &format!("{page_id}-{index}")))
            .collect::<Result<Vec<_>>>()?;

        // `{'id', 'title', 'description', 'location', **_search_json_ld(...)}`.
        let json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {page_id}: Unable to extract JSON-LD; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        if ["url", "formats", "id", "subtitles", "_type"]
            .iter()
            .any(|key| json_ld.contains_key(*key))
        {
            return Err(unsupported("JSON-LD media URLs"));
        }
        let og = |property: &str| crate::html::og_search_property(&webpage, property);
        let og_title = og("title");
        if og_title.is_none() {
            crate::screen::warning(&format!(
                "[{NAME}] {page_id}: unable to extract OpenGraph title; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let description = og("description");
        if description.is_none() {
            crate::screen::warning(&format!(
                "[{NAME}] {page_id}: unable to extract OpenGraph description; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let mut common: Vec<(String, Value)> = vec![
            ("id".into(), page_id.clone().into()),
            ("title".into(), og_title.map_or(Value::Null, Value::from)),
            (
                "description".into(),
                description.map_or(Value::Null, Value::from),
            ),
            (
                "location".into(),
                videos_info.get("luogo").cloned().unwrap_or(Value::Null),
            ),
        ];
        for (key, value) in json_ld {
            match common.iter_mut().find(|(name, _)| *name == key) {
                Some((_, old)) => *old = value,
                None => common.push((key, value)),
            }
        }

        if entries.len() == 1 {
            let entry = entries
                .into_iter()
                .next()
                .ok_or_else(|| unsupported("empty playlists"))?;
            // `{**entries[0], **common_info}`.
            let mut fields: Vec<(String, Value)> = vec![
                ("id".into(), Value::Null),
                ("title".into(), Value::from(entry.title.clone())),
                ("formats".into(), Value::Null),
                ("subtitles".into(), Value::Null),
            ];
            if entry.extra.contains_key(crate::info::TITLE_MISSING) {
                fields[1].1 = Value::Null;
            }
            for (key, value) in common {
                match fields.iter_mut().find(|(name, _)| *name == key) {
                    Some((_, old)) => *old = value,
                    None => fields.push((key, value)),
                }
            }
            let fields: Vec<(&str, Value)> = fields
                .iter()
                .filter(|(key, value)| {
                    matches!(key.as_str(), "title" | "formats" | "subtitles") || !value.is_null()
                })
                .map(|(key, value)| (key.as_str(), value.clone()))
                .collect();
            let mut info = dict_info(url, (NAME, NAME), fields, entry.formats);
            info.subtitles = entry.subtitles;
            return Ok(Some(ExtractResult::Video(info)));
        }

        // `playlist_result(entries, multi_video=True, **common_info)`.
        let mut keys = Vec::new();
        let mut extra = Map::new();
        let mut title = String::new();
        for (key, value) in common {
            if value.is_null() {
                continue;
            }
            keys.push(key.clone());
            match key.as_str() {
                "id" => {}
                "title" => title = value.as_str().unwrap_or_default().to_owned(),
                _ => {
                    extra.insert(key, value);
                }
            }
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;{}{}",
                keys.join(","),
                crate::info_order::GENERATOR
            )
            .into(),
        );
        extra.insert(crate::info::MULTI_VIDEO.into(), Value::Bool(true));
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: page_id,
            title,
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.radioradicale.it/scheda/471591"));
        assert!(suitable(
            "https://www.radioradicale.it/scheda/742783/parlamento-riunito-in-seduta-comune"
        ));
        assert!(!suitable("https://www.radioradicale.it/scheda/abc"));
    }
}
