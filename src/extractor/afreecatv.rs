//! Upstream `AfreecaTVCatchStoryIE` (`yt_dlp/extractor/afreecatv.py`,
//! sooplive.com): a catch story's clips from
//! `api.m.sooplive.com/catchstory/a/view` as direct URL entries. `AfreecaTVIE`
//! (VODs), `AfreecaTVLiveIE` and `AfreecaTVUserIE` are not ported.
use super::{Extractor, direct, download_json_note_headers};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct AfreecaTvCatchStory;

const NAME: &str = "soop:catchstory";
const KEY: &str = "AfreecaTVCatchStory";
const API: &str = "https://api.m.sooplive.com";

static CATCH_URL: LazyLock<Regex> = LazyLock::new(|| {
    super::url_regex(r"^https?://vod\.sooplive\.com/player/(?P<id>\d+)/catchstory").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    CATCH_URL.is_match(url)
}

/// `_fixup_thumb`: the thumbnail as a `jpg` (its URL ends in a PHP script).
fn fixup_thumb(thumb: Option<&Value>) -> Option<Value> {
    let url = thumb?.as_str()?;
    (!url.is_empty()).then(|| json!([{ "url": url, "ext": "jpg", "id": "0" }]))
}

/// `int_or_none(value, scale=1000)`.
fn scaled_int(value: Option<&Value>) -> Option<Value> {
    let number = match value? {
        Value::Number(number) => number.as_i64()?,
        Value::String(text) => text.trim().parse::<i64>().ok()?,
        _ => return None,
    };
    Some((number / 1000).into())
}

/// A string-valued field (`{str}`), else `None`.
fn text(value: Option<&Value>) -> Option<Value> {
    match value {
        Some(Value::String(text)) => Some(text.clone().into()),
        _ => None,
    }
}

/// A direct URL entry with `fields` (in order): the URL as its one format,
/// as upstream's `url` results of a playlist.
fn direct_entry(page: &Url, url: &str, fields: Vec<(&str, Value)>) -> Result<VideoInfo> {
    let media = Url::parse(url)?;
    let ext = direct::determine_ext(url).unwrap_or_default();
    let mut info = direct::with_format(direct::build_at(media.clone(), &media, ext.clone()), "");
    for format in &mut info.formats {
        format.ext = ext.clone();
        format.format_id = String::new();
    }
    info.format_id = None;
    info.webpage_url = page.clone();
    info.direct = None;
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
    let mut keys = Vec::new();
    for (name, value) in fields {
        keys.push(name.to_owned());
        match name {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            _ => crate::metadata_parser::set_field(&mut info, name, value),
        }
    }
    let shape = format!("untyped-dict;{};thumbnail=url,ext", keys.join(","));
    Ok(crate::info_order::mark_info(info, &shape))
}

impl Extractor for AfreecaTvCatchStory {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CATCH_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let api = Url::parse(&format!(
            "{API}/catchstory/a/view?aStoryListIdx=&nStoryIdx={video_id}"
        ))?;
        let data = download_json_note_headers(
            client,
            &api,
            "soop",
            &video_id,
            "Downloading API JSON",
            &[("Referer", url.as_str())],
        )?;

        let mut entries = Vec::new();
        for story in data
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if story.get("story_type").and_then(Value::as_str) != Some("catch") {
                continue;
            }
            for clip in story
                .get("catch_list")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                // `lambda _, v: v['files'][0]['file']`: a truthy file.
                let first = clip.get("files").and_then(|files| files.get(0));
                let Some(file) = first
                    .and_then(|first| first.get("file"))
                    .filter(|file| super::nuxt::truthy(file))
                else {
                    continue;
                };
                let file = file.as_str().unwrap_or_default();
                // Upstream's `url_or_none` drops anything else.
                if !(file.starts_with("https://") || file.starts_with("http://")) {
                    continue;
                }
                // Upstream's dict order: traverse_obj keeps the present keys.
                let mut fields: Vec<(&str, Value)> = Vec::new();
                if let Some(id) = text(first.and_then(|first| first.get("file_info_key"))) {
                    fields.push(("id", id));
                }
                fields.push(("url", file.into()));
                if let Some(duration) = scaled_int(first.and_then(|first| first.get("duration"))) {
                    fields.push(("duration", duration));
                }
                if let Some(title) = text(clip.get("title")) {
                    fields.push(("title", title));
                }
                if let Some(uploader) = text(clip.get("writer_nick")) {
                    fields.push(("uploader", uploader));
                }
                if let Some(uploader_id) = text(clip.get("writer_id")) {
                    fields.push(("uploader_id", uploader_id));
                }
                if let Some(thumbnails) = fixup_thumb(clip.get("thumb")) {
                    fields.push(("thumbnails", thumbnails));
                }
                if let Some(timestamp) = clip.get("write_timestamp").and_then(Value::as_i64) {
                    fields.push(("timestamp", timestamp.into()));
                }
                entries.push(direct_entry(url, file, fields)?);
            }
        }

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: video_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: KEY.into(),
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
        assert!(suitable(
            "https://vod.sooplive.com/player/103247/catchstory"
        ));
        assert!(!suitable("https://vod.sooplive.com/player/192805325"));
    }

    #[test]
    fn durations_scale_from_milliseconds() {
        assert_eq!(scaled_int(Some(&json!(16_999))), Some(json!(16)));
        assert_eq!(scaled_int(Some(&json!("13000"))), Some(json!(13)));
    }
}
