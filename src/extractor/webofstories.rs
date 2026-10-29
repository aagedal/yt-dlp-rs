//! Upstream `WebOfStoriesIE` and `WebOfStoriesPlaylistIE`
//! (`yt_dlp/extractor/webofstories.py`): a story page's `getEmbedCode(...)`
//! arguments as its MP4 and RTMP formats, and a speaker's `playAll` page as
//! a playlist of story pages.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct WebOfStories;
pub struct WebOfStoriesPlaylist;

const NAME: &str = "WebOfStories";
const PLAYLIST_NAME: &str = "WebOfStoriesPlaylist";
const VIDEO_DOMAIN: &str = "http://eu-mobile.webofstories.com/";
const GREAT_LIFE_STREAMER: &str = "rtmp://eu-cdn1.webofstories.com/cfx/st/";
const USER_STREAMER: &str = "rtmp://eu-users.webofstories.com/cfx/st/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?webofstories\.com/play/(?:[^/]+/)?(?P<id>[0-9]+)").unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?webofstories\.com/playAll/(?P<id>[^/]+)").unwrap()
});
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<strong>Title:\s*</strong>(.+?)<").unwrap());
static EMBED_PARAMS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r##"(?s)\$\("#embedCode"\).html\(getEmbedCode\((.*?)\)"##).unwrap()
});
static STORY_IDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bid=["']td_(\d+)"#).unwrap());
static SPEAKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div id="speakerName">\s*<span>([^<]+)</span>"#).unwrap());
static FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<span id="primaryField">([^<]+)</span>"#).unwrap());
static PAGE_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<title>Play\s+all\s+stories\s*-\s*([^<]+)\s*-\s*Web\s+of\s+Stories</title>")
        .unwrap()
});

/// `WebOfStoriesIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `WebOfStoriesPlaylistIE._match_id`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

fn search_error(name: &str, id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{name}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `[s.strip(" \r\n\t'") for s in params.split(',')]`, unpacked into ten
/// names (a different count is Python's bare `ValueError`).
fn embed_params(params: &str) -> Result<Vec<String>> {
    let parts: Vec<String> = params
        .split(',')
        .map(|part| {
            part.trim_matches(|c| matches!(c, ' ' | '\r' | '\n' | '\t' | '\''))
                .to_owned()
        })
        .collect();
    match parts.len() {
        10 => Ok(parts),
        got @ 0..10 => Err(Error::Reported(format!(
            "not enough values to unpack (expected 10, got {got})"
        ))),
        _ => Err(Error::Reported(
            "too many values to unpack (expected 10)".into(),
        )),
    }
}

/// `int_or_none(text)`.
fn int_or_none(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.starts_with('_') || text.ends_with('_') || text.contains("__") {
        return None;
    }
    text.replace('_', "").parse().ok()
}

impl Extractor for WebOfStories {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(NAME, url);
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // Sometimes og:title meta is malformed.
        let title = match crate::html::og_search_property(&webpage, "title") {
            Some(title) if !title.is_empty() => title,
            _ => TITLE
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]))
                .ok_or_else(|| search_error(NAME, &video_id, "title"))?,
        };
        let description = crate::html::html_search_meta_clean(&webpage, "description");
        if description.is_none() {
            super::search_warning(NAME, "description");
        }
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            super::search_warning(NAME, "thumbnail URL");
        }
        let params = EMBED_PARAMS
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| search_error(NAME, &video_id, "embed params"))?;
        let params = embed_params(&params)?;
        let speaker_id = &params[1];
        let story_id = &params[2];
        let duration = int_or_none(&params[3]);
        let speaker_type = &params[4];
        let is_great_life_series = params[5] == "true";
        let story_filename = &params[8];

        // URL building, see: http://www.webofstories.com/scripts/player.js
        let ms_prefix = if speaker_type.to_lowercase() == "ms" {
            "mini_sites/"
        } else {
            ""
        };
        let (mp4_url, rtmp_ext, streamer, play_path) = if is_great_life_series {
            (
                format!("{VIDEO_DOMAIN}lives/{speaker_id}/{story_filename}.mp4"),
                "flv",
                GREAT_LIFE_STREAMER,
                format!("stories/{speaker_id}/{story_filename}"),
            )
        } else {
            (
                format!("{VIDEO_DOMAIN}{ms_prefix}{speaker_id}/{story_filename}.mp4"),
                "mp4",
                USER_STREAMER,
                format!("mp4:{ms_prefix}{speaker_id}/{story_filename}.mp4"),
            )
        };
        let formats = vec![
            dict_format(
                vec![("format_id", "mp4_sd".into()), ("url", mp4_url.into())],
                NAME,
                story_id,
            )?,
            dict_format(
                vec![
                    ("format_id", "rtmp_sd".into()),
                    ("page_url", url.as_str().into()),
                    ("url", streamer.into()),
                    ("ext", rtmp_ext.into()),
                    ("play_path", play_path.into()),
                ],
                NAME,
                story_id,
            )?,
        ];
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", story_id.as_str().into()),
                ("title", title.into()),
                ("formats", Value::Null),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
                ("description", description.map_or(Value::Null, Value::from)),
                ("duration", duration.map_or(Value::Null, Value::from)),
            ],
            formats,
        )))
    }
}

impl Extractor for WebOfStoriesPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        announce(PLAYLIST_NAME, url);
        let webpage = download_webpage(client, url, PLAYLIST_NAME, &playlist_id)?;
        let mut seen: Vec<&str> = Vec::new();
        for found in STORY_IDS.captures_iter(&webpage) {
            let id = found.get(1).unwrap().as_str();
            if !seen.contains(&id) {
                seen.push(id);
            }
        }
        let entries = seen
            .iter()
            .map(|id| {
                super::nrk::url_entry(
                    &format!("http://www.webofstories.com/play/{id}"),
                    "WebOfStories",
                    Some(id),
                    None,
                    None,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let mut title = SPEAKER.captures(&webpage).map(|found| found[1].to_owned());
        if let Some(speaker) = title.as_mut().filter(|title| !title.is_empty())
            && let Some(field) = FIELD
                .captures(&webpage)
                .map(|found| found[1].to_owned())
                .filter(|field| !field.is_empty())
        {
            speaker.push_str(&format!(" ({field})"));
        }
        let title = match title.filter(|title| !title.is_empty()) {
            Some(title) => title,
            None => PAGE_TITLE
                .captures(&webpage)
                .map(|found| found[1].to_owned())
                .ok_or_else(|| search_error(PLAYLIST_NAME, &playlist_id, "title"))?,
        };
        // `playlist_result(entries, playlist_id, title)`: a falsy title is
        // left out.
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            if title.is_empty() {
                "playlist-dict;id"
            } else {
                "playlist-dict;id,title"
            }
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title,
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_NAME.into(),
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
        assert!(suitable("http://www.webofstories.com/play/hans.bethe/71"));
        assert!(suitable("http://www.webofstories.com/play/54215?o=MS"));
        assert!(playlist_suitable(
            "http://www.webofstories.com/playAll/donald.knuth"
        ));
        assert!(!suitable(
            "http://www.webofstories.com/playAll/donald.knuth"
        ));
    }

    #[test]
    fn embed_params_unpack_like_python() {
        let params =
            embed_params("size,\n\t 1003,\n 4536, 238, 'GL', true, 'x', !true, '71', 71").unwrap();
        assert_eq!(params[1], "1003");
        assert_eq!(params[4], "GL");
        assert_eq!(params[8], "71");
        assert_eq!(
            embed_params("a,b").unwrap_err().to_string(),
            "not enough values to unpack (expected 10, got 2)"
        );
    }
}
