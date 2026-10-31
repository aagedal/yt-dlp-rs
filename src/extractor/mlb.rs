//! Upstream `MLBIE` and `MLBVideoIE` (`yt_dlp/extractor/mlb.py`): a video's
//! content API (`MLBIE`) or GraphQL `mediaPlayback` feed (`MLBVideoIE`) with
//! its playbacks: HLS masters as `m3u8_native` formats and progressive files
//! with their size and bitrate read from the names. MLB.TV, articles and
//! closed-caption tracks are not ported; closed captions keep their error.
use super::{Extractor, dict_format, dict_info, direct, download_json, update_url_query};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Mlb;
pub struct MlbVideo;

const NAME: &str = "MLB";
const KEY: &str = "MLB";
const VIDEO_NAME: &str = "MLBVideo";
const VIDEO_KEY: &str = "MLBVideo";

const GRAPHQL: &str = r#"{
  mediaPlayback(ids: "%s") {
    description
    feeds(types: CMS) {
      closedCaptions
      duration
      image {
        cuts {
          width
          height
          src
        }
      }
      playbacks {
        name
        url
      }
    }
    id
    timestamp
    title
  }
}"#;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:[\da-z_-]+\.)*mlb\.com/(?:(?:(?:[^/]+/)*video/[^/]+/c-|(?:shared/video/embed/(?:embed|m-internal-embed)\.html|(?:[^/]+/)+(?:play|index)\.jsp|)\?.*?\bcontent_id=)(?P<id>\d+))",
    )
    .unwrap()
});
static VIDEO_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?mlb\.com/(?:[^/]+/)*video/(?P<id>[^/?&#]+)")
        .unwrap()
});
static BITRATE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_(\d+)K_(\d+)X(\d+)").unwrap());
static BITRATE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_(\d+)x(\d+)_(\d+)_(\d+)K\.mp4").unwrap());

/// `_match_id` for `MLBIE`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `MLBVideoIE.suitable`: not for URLs `MLBIE` takes.
pub fn video_suitable(url: &str) -> bool {
    !suitable(url) && VIDEO_VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MLB {what} is not supported natively"))
}

/// The `[id, name]` pairs of `playbacks`, skipping empty URLs.
fn playbacks(feed: &Value) -> Vec<(String, String)> {
    feed.get("playbacks")
        .and_then(Value::as_array)
        .map(|playbacks| {
            playbacks
                .iter()
                .filter_map(|playback| {
                    let url = playback.get("url").and_then(Value::as_str)?;
                    if url.is_empty() {
                        return None;
                    }
                    let name = playback
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    Some((url.to_owned(), name.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The `image.cuts` thumbnails with a source, in upstream's key order.
fn thumbnails(feed: &Value) -> Value {
    let cuts = feed
        .pointer("/image/cuts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Value::Array(
        cuts.iter()
            .filter(|cut| {
                cut.get("src")
                    .and_then(Value::as_str)
                    .is_some_and(|src| !src.is_empty())
            })
            .map(|cut| {
                let number = |key: &str| crate::extractor::streamable::int_or_none(cut.get(key));
                json!({
                    "height": number("height"),
                    "url": cut.get("src").cloned().unwrap_or(Value::Null),
                    "width": number("width"),
                })
            })
            .collect(),
    )
}

/// `_extract_mlb_subtitles`: caption tracks are not ported.
fn ensure_no_subtitles(has_captions: bool) -> Result<()> {
    if has_captions {
        Err(unsupported("closed captions"))
    } else {
        Ok(())
    }
}

/// The shared `MLBBaseIE._real_extract` result.
#[allow(clippy::too_many_arguments)]
fn build(
    url: &Url,
    video_id: &str,
    title: Value,
    description: Value,
    duration: Value,
    thumbnails: Value,
    timestamp: Value,
    feed: &Value,
    extractor: (&'static str, &'static str),
    client: &Client,
) -> Result<VideoInfo> {
    let mut formats = Vec::new();
    for (playback_url, name) in playbacks(feed) {
        let parsed = Url::parse(&playback_url)?;
        if direct::determine_ext(&playback_url).as_deref() == Some("m3u8") {
            if let Some(info) = super::extract_m3u8_master_optional(
                client,
                &parsed,
                extractor.0,
                video_id,
                Some(&name),
                false,
                false,
            ) {
                formats.extend(info.formats);
            }
            continue;
        }
        let mut fields = vec![
            ("format_id", Value::from(name.clone())),
            ("url", Value::from(playback_url.clone())),
        ];
        if let Some(captures) = BITRATE_NAME.captures(&name) {
            let int = |index: usize| Value::from(captures[index].parse::<i64>().unwrap_or(0));
            fields.push(("height", int(3)));
            fields.push(("tbr", int(1)));
            fields.push(("width", int(2)));
        }
        if let Some(captures) = BITRATE_URL.captures(&playback_url) {
            let int = |index: usize| Value::from(captures[index].parse::<i64>().unwrap_or(0));
            set_or_push(&mut fields, "fps", int(3));
            set_or_push(&mut fields, "height", int(2));
            set_or_push(&mut fields, "tbr", int(4));
            set_or_push(&mut fields, "width", int(1));
        }
        let mut format = dict_format(fields, extractor.0, video_id)?;
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    let mut info = dict_info(
        url,
        extractor,
        vec![
            ("id", Value::from(video_id)),
            ("title", title),
            ("formats", Value::Null),
            ("description", description),
            ("duration", duration),
            ("thumbnails", thumbnails),
            ("timestamp", timestamp),
            ("subtitles", Value::Null),
        ],
        formats,
    );
    // Upstream's thumbnail dicts keep `height`, `url`, `width` before the
    // `id` and `resolution` the sanitizer adds.
    if let Some(Value::String(shape)) = info.extra.get(crate::info_order::INFO_SHAPE).cloned() {
        info.extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("{shape};thumbnail=height,url,width").into(),
        );
    }
    Ok(info)
}

/// Python's `dict.update`: an existing key keeps its position.
fn set_or_push(fields: &mut Vec<(&'static str, Value)>, key: &'static str, value: Value) {
    match fields.iter_mut().find(|(name, _)| *name == key) {
        Some(field) => field.1 = value,
        None => fields.push((key, value)),
    }
}

fn duration_value(text: Option<&str>) -> Value {
    text.and_then(crate::parse_duration)
        .and_then(|duration| serde_json::to_value(duration).ok())
        .unwrap_or(Value::Null)
}

fn timestamp_value(text: Option<&str>) -> Value {
    text.and_then(crate::dates::parse_iso8601)
        .map_or(Value::Null, Value::from)
}

impl Extractor for Mlb {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "http://content.mlb.com/mlb/item/id/v1/{display_id}/details/web-v1.json"
        ))?;
        let video = download_json(client, &api, NAME, &display_id)?;
        let video_id = match video.get("id") {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => return Err(super::clyp::key_error(&display_id, "id")),
        };
        let title = video
            .get("title")
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&display_id, "title"))?;
        ensure_no_subtitles(
            video
                .get("keywordsAll")
                .and_then(Value::as_array)
                .is_some_and(|keywords| {
                    keywords.iter().any(|keyword| {
                        keyword
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| kind.starts_with("closed_captions_location_"))
                            && keyword
                                .get("value")
                                .and_then(Value::as_str)
                                .is_some_and(|v| !v.is_empty())
                    })
                }),
        )?;
        let description = video.get("description").cloned().unwrap_or(Value::Null);
        let info = build(
            url,
            &video_id,
            title,
            description,
            duration_value(video.get("duration").and_then(Value::as_str)),
            thumbnails(&video),
            timestamp_value(video.get("date").and_then(Value::as_str)),
            &video,
            (NAME, KEY),
            client,
        )?;
        Ok(Some(info))
    }
}

impl Extractor for MlbVideo {
    fn key(&self) -> &'static str {
        VIDEO_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = VIDEO_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(VIDEO_NAME);
        crate::screen::screen(&format!(
            "[{VIDEO_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let query = GRAPHQL.replace("%s", &display_id);
        let api = update_url_query(
            &Url::parse("https://fastball-gateway.mlb.com/graphql")?,
            &[("query", query.as_str())],
        );
        let response = download_json(client, &api, VIDEO_NAME, &display_id)?;
        let video = response
            .pointer("/data/mediaPlayback/0")
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&display_id, "data"))?;
        let video_id = video
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| super::clyp::key_error(&display_id, "id"))?;
        let title = video
            .get("title")
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&display_id, "title"))?;
        let feed = video.pointer("/feeds/0").cloned().unwrap_or(Value::Null);
        // Upstream's `_extract_mlb_subtitles` returns nothing here, so the
        // result's `subtitles` is `None` whatever the feed's captions.
        let mut info = build(
            url,
            &video_id,
            title,
            video.get("description").cloned().unwrap_or(Value::Null),
            duration_value(feed.get("duration").and_then(Value::as_str)),
            thumbnails(&feed),
            timestamp_value(video.get("timestamp").and_then(Value::as_str)),
            &feed,
            (VIDEO_NAME, VIDEO_KEY),
            client,
        )?;
        info.extra.insert("subtitles".into(), Value::Null);
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert!(suitable(
            "https://www.mlb.com/mariners/video/ackleys-spectacular-catch/c-34698933"
        ));
        assert!(!suitable(
            "https://www.mlb.com/mariners/video/ackley-s-spectacular-catch-c34698933"
        ));
        assert!(video_suitable(
            "https://www.mlb.com/mariners/video/ackley-s-spectacular-catch-c34698933"
        ));
        assert!(!video_suitable(
            "https://www.mlb.com/mariners/video/ackleys-spectacular-catch/c-34698933"
        ));
    }
}
