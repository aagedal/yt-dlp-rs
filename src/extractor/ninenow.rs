//! Upstream `NineNowIE` (`yt_dlp/extractor/ninenow.py`, IE_NAME `9now.com.au`):
//! 9Now clips and episodes, whose Next.js flight data names a Brightcove
//! video; a `url_transparent` result of its player with the page's metadata.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NineNow;

const NAME: &str = "9now.com.au";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?9now\.com\.au/(?:[^/?#]+/){2}(?P<id>(?P<type>clip|episode)-[^/?#]+)",
    )
    .unwrap()
});

/// `BRIGHTCOVE_URL_TEMPLATE`.
const BRIGHTCOVE_URL_TEMPLATE: &str =
    "http://players.brightcove.net/4460760524001/default_default/index.html?videoId=";

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none`: integers, and strings of digits.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// `_search_nextjs_v13_data` (fatal) then `get_first(..., ('payload', {dict}))`.
fn payload(webpage: &str, display_id: &str) -> Result<Option<Value>> {
    let data = super::search_nextjs_v13_data(webpage, NAME, display_id)?;
    Ok(data.values().find_map(|item| {
        item.get("payload")
            .filter(|value| value.is_object())
            .cloned()
    }))
}

impl Extractor for NineNow {
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
        let display_id = found["id"].to_owned();
        let video_type = found["type"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let common = payload(&webpage, &display_id)?.unwrap_or(Value::Null);
        let page = common.get(&video_type);
        let video = page.and_then(|page| page.get("video"));

        if video
            .and_then(|video| video.get("drm"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            crate::no_formats::raise_expected(
                NAME,
                Some(&display_id),
                None,
                "This video is DRM protected",
            )?;
            return Err(Error::UnsupportedHtml5(
                "9Now DRM videos with --ignore-no-formats-error".into(),
            ));
        }
        // `brightcoveId` (a string) or `referenceId` as `ref:...`, the first truthy.
        let brightcove_id = non_empty_str(video.and_then(|video| video.get("brightcoveId")))
            .map(str::to_owned)
            .or_else(|| {
                non_empty_str(video.and_then(|video| video.get("referenceId")))
                    .map(|reference| format!("ref:{reference}"))
            })
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract brightcove ID; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;

        let mut fields = Map::new();
        // `'id'`: the video's integer ID as text, else the Brightcove ID.
        let id = video
            .and_then(|video| video.get("id"))
            .and_then(Value::as_i64)
            .map_or_else(|| brightcove_id.clone(), |id| id.to_string());
        fields.insert("id".into(), id.into());
        if let Some(title) = page
            .and_then(|page| page.get("name"))
            .and_then(Value::as_str)
        {
            fields.insert("title".into(), title.into());
        }
        if let Some(description) = page
            .and_then(|page| page.get("description"))
            .and_then(Value::as_str)
        {
            fields.insert("description".into(), description.into());
        }
        if let Some(duration) = video
            .and_then(|video| video.get("duration"))
            .and_then(Value::as_f64)
        {
            fields.insert("duration".into(), json!(duration / 1000.0));
        }
        if let Some(tags) = page
            .and_then(|page| page.get("tags"))
            .and_then(Value::as_array)
        {
            let names: Vec<Value> = tags
                .iter()
                .filter_map(|tag| non_empty_str(tag.get("name")))
                .map(Value::from)
                .collect();
            fields.insert("tags".into(), Value::Array(names));
        }
        let series = common
            .pointer("/tvSeries/name")
            .and_then(Value::as_str)
            .map(Value::from);
        if let Some(series) = series {
            fields.insert("series".into(), series);
        }
        if let Some(season) = int_or_none(common.pointer("/season/seasonNumber")) {
            fields.insert("season_number".into(), season.into());
        }
        if let Some(episode) = int_or_none(common.pointer("/episode/episodeNumber")) {
            fields.insert("episode_number".into(), episode.into());
        }
        if let Some(air_date) = common
            .pointer("/episode/airDate")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.insert("timestamp".into(), air_date.into());
        }
        if let Some(available) = page
            .and_then(|page| page.get("availability"))
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.insert("release_timestamp".into(), available.into());
        }
        if let Some(sizes) = page
            .and_then(|page| page.pointer("/image/sizes"))
            .and_then(Value::as_object)
        {
            let thumbnails: Vec<Value> = sizes
                .iter()
                .filter_map(|(key, value)| {
                    let url = super::json_ld::url_or_none(value.as_str()?)?;
                    Some(json!({
                        "id": key,
                        "url": url,
                        "width": super::html5::parse_resolution(&url).0,
                    }))
                })
                .collect();
            fields.insert("thumbnails".into(), Value::Array(thumbnails));
        }

        let target = format!("{BRIGHTCOVE_URL_TEMPLATE}{brightcove_id}");
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({
                "url": target,
                "fields": Value::Object(fields),
                "thumbnail_keys": "id,url,width",
            }),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.9now.com.au/today/season-2025/clip-cm8hw9h5z00080hquqa5hszq7"
        ));
        assert!(suitable(
            "https://www.9now.com.au/afl-footy-show/2016/episode-19"
        ));
        assert!(!suitable("https://www.9now.com.au/afl-footy-show/2016/"));
    }
}
