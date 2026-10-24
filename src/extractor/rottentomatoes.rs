//! Upstream `RottenTomatoesIE` (`yt_dlp/extractor/rottentomatoes.py`): a
//! movie or trailers page's `heroVideos`/`videos` script JSON, each `hls`
//! video's non-fatal playlist as an entry (or the one whose `publicId`
//! the URL names).
use super::{Extractor, dict_info, download_webpage, extract_m3u8_optional};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct RottenTomatoes;

const NAME: &str = "RottenTomatoes";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?rottentomatoes\.com/m/(?P<playlist>[^/]+)(?:/(?P<tr>trailers)(?:/(?P<id>\w+))?)?",
    )
    .unwrap()
});
static DATA_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script[^>]+\bid=["'](?:heroV|v)ideos["'][^>]*>\s*"#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_json(..., contains_pattern=r'\[{(?s:.+)}\]')`: the greedy
/// `[{...}]` after the script tag, parsed with `ignore_extra`.
fn search_data(webpage: &str) -> Option<Option<Value>> {
    let found = DATA_START.find(webpage)?;
    let rest = &webpage[found.end()..];
    if !rest.starts_with("[{") {
        return None;
    }
    let end = rest.rfind("}]")?;
    if end == 0 {
        return None;
    }
    Some(
        serde_json::Deserializer::from_str(&rest[..end + 2])
            .into_iter::<Value>()
            .next()
            .and_then(std::result::Result::ok),
    )
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

/// `_extract_videos`: one entry per `publicId`/`file`/`type == 'hls'` video.
fn extract_videos(
    client: &Client,
    page: &Url,
    videos: &[&Value],
    display_id: &str,
) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    for video in videos {
        if !(truthy(video.get("publicId"))
            && truthy(video.get("file"))
            && video.get("type").and_then(Value::as_str) == Some("hls"))
        {
            continue;
        }
        let Some(file) = video.get("file").and_then(Value::as_str) else {
            return Err(Error::UnsupportedHtml5(
                "RottenTomatoes video files that are not strings".into(),
            ));
        };
        let formats = match Url::parse(file) {
            Ok(manifest) => {
                extract_m3u8_optional(client, &manifest, NAME, display_id, Some("hls"), false)
                    .map(|stream| stream.formats)
                    .unwrap_or_default()
            }
            Err(_) => {
                return Err(Error::UnsupportedHtml5(
                    "RottenTomatoes video files that are not URLs".into(),
                ));
            }
        };
        // `traverse_obj(video, {...})` keeps only non-`None` values.
        let mut fields = vec![("formats", Value::Null)];
        if let Some(id) = video.get("publicId").filter(|id| !id.is_null()) {
            let Some(id) = id.as_str() else {
                return Err(Error::UnsupportedHtml5(
                    "RottenTomatoes public IDs that are not strings".into(),
                ));
            };
            fields.push(("id", id.into()));
        }
        for key in ["title", "description"] {
            match video.get(key) {
                None | Some(Value::Null) => {}
                Some(Value::String(text)) => fields.push((key, text.clone().into())),
                Some(_) => {
                    return Err(Error::UnsupportedHtml5(format!(
                        "RottenTomatoes video {key}s that are not strings"
                    )));
                }
            }
        }
        // `float_or_none(durationInSeconds)`.
        let duration = match video.get("durationInSeconds") {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
            _ => None,
        };
        if let Some(duration) = duration {
            fields.push(("duration", duration.into()));
        }
        // `url_or_none(image)`.
        if let Some(image) = video
            .get("image")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            fields.push(("thumbnail", image.into()));
        }
        entries.push(dict_info(page, (NAME, NAME), fields, formats));
    }
    Ok(entries)
}

impl Extractor for RottenTomatoes {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = match found.name("tr") {
            Some(trailers) => format!("{}-{}", &found["playlist"], trailers.as_str()),
            None => found["playlist"].to_owned(),
        };
        let video_id = found.name("id").map(|id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &playlist_id)?;
        let data = match search_data(&webpage) {
            Some(Some(data)) => data,
            Some(None) => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {playlist_id}: Unable to extract data - Failed to parse JSON"
                )));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {playlist_id}: Unable to extract data; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let videos: Vec<&Value> = match &data {
            Value::Array(items) => items.iter().collect(),
            _ => Vec::new(),
        };
        if let Some(video_id) = video_id {
            let matching: Vec<&Value> = videos
                .into_iter()
                .filter(|video| video.get("publicId").and_then(Value::as_str) == Some(&video_id))
                .collect();
            if matching.is_empty() {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract video from webpage; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            // `next(self._extract_videos(video_data, video_id))`.
            let Some(entry) = extract_videos(client, url, &matching, &video_id)?
                .into_iter()
                .next()
            else {
                return Err(Error::UnsupportedHtml5(
                    "RottenTomatoes trailers without an HLS file".into(),
                ));
            };
            return Ok(Some(ExtractResult::Video(entry)));
        }
        let entries = extract_videos(client, url, &videos, &playlist_id)?;
        // `clean_html(get_element_by_class('scoreboard__title', webpage))`.
        let title =
            crate::html_elements::elements_text_and_html_by_class("scoreboard__title", &webpage)
                .ok()
                .and_then(|found| found.into_iter().next())
                .map(|(text, _)| crate::html::clean_html(&text));
        let keys = if title.is_some() { "id,title" } else { "id" };
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
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
        assert!(suitable(
            "https://www.rottentomatoes.com/m/toy_story_3/trailers/VycaVoBKhGuk"
        ));
        assert!(suitable("http://www.rottentomatoes.com/m/toy_story_3"));
        assert!(suitable(
            "http://www.rottentomatoes.com/m/toy_story_3/trailers"
        ));
        assert!(!suitable("https://www.rottentomatoes.com/tv/the_bear"));
    }

    #[test]
    fn data_is_the_greedy_list() {
        let page = r#"<script id="videos" type="application/json">[{"a": 1}]</script><p>}]"#;
        assert_eq!(search_data(page), Some(Some(serde_json::json!([{"a": 1}]))));
    }
}
