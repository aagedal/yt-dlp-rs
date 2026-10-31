//! Upstream `ZypeIE` (`yt_dlp/extractor/zype.py`): Zype player embeds from
//! their `.json` player response: a body of outputs (HLS and progressive
//! files) or the player script's HLS URL (or Verizon Media/Uplynk source),
//! text tracks, thumbnails and video metadata; the API's 400/401/403
//! messages are upstream's errors. Player scripts on other pages are
//! followed.
use super::{Extractor, dict_format, dict_info};
use crate::{Error, Result, SubtitleFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Zype;

pub(crate) const NAME: &str = "Zype";
pub(crate) const KEY: &str = "Zype";

const COMMON: &str =
    r"//player\.zype\.com/embed/{ID}\.(?:js|json|html)\?.*?(?:access_token|(?:ap[ip]|player)_key)=";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        "^https?:{}[^&]+",
        COMMON.replace("{ID}", r"(?P<id>[\da-fA-F]+)")
    ))
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    let common = COMMON.replace("{ID}", r"[\da-fA-F]+");
    Regex::new(&format!(
        r#"<script[^>]+\bsrc=(?:"(?P<dq>(?:https?:)?{common}[^"\n]+?)"|'(?P<sq>(?:https?:)?{common}[^'\n]+?)')"#
    ))
    .unwrap()
});
static JSON_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(?:js|html)\?").unwrap());
static M3U8_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:"(?P<dq>[^"\n]+\.m3u8[^"\n]*)"|'(?P<sq>[^'\n]+\.m3u8[^'\n]*)')"#).unwrap()
});
static SOURCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)sources\s*:\s*\[\s*(\{.+?\})\s*\]").unwrap());
static TEXT_TRACKS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"textTracks\s*:\s*(\[[^\]]+\])").unwrap());

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
        let Some(raw) = captures.name("dq").or_else(|| captures.name("sq")) else {
            continue;
        };
        let Ok(embed) = url.join(&crate::html::unescape_html(raw.as_str())) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Zype {what} are not supported natively"))
}

fn regex_error(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        Some(Value::Bool(value)) => i64::from(*value).into(),
        _ => Value::Null,
    }
}

/// `dict_get(d, keys)`: the first truthy value.
fn dict_get<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find(|value| super::nuxt::truthy(value))
}

/// The player response, or the API's 400/401/403 `message`.
fn player_response(client: &Client, api: &Url, video_id: &str) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading JSON metadata"));
    let mut response = match super::direct::request_expecting(client, api, &[], &[400, 401, 403]) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            return Err(Error::ExtractorRequest {
                extractor: NAME,
                id: video_id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            });
        }
        Err(error) => return Err(error),
    };
    let status = response.status().as_u16();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    let data: Value = crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
        ))
    })?;
    if matches!(status, 400 | 401 | 403) {
        let message = match data.get("message") {
            Some(Value::String(message)) => message.clone(),
            Some(_) => return Err(unsupported("error messages that are not strings")),
            None => return Err(key_error(video_id, "message")),
        };
        return Err(Error::Reported(format!("[{NAME}] {video_id}: {message}")));
    }
    Ok(data)
}

fn quoted<'a>(found: &regex::Captures<'a>) -> Option<&'a str> {
    found
        .name("dq")
        .or_else(|| found.name("sq"))
        .map(|value| value.as_str())
}

impl Extractor for Zype {
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
        let api = Url::parse(&JSON_PATH.replace_all(url.as_str(), ".json?"))?;
        let response = player_response(client, &api, &video_id)?;
        let response = response
            .get("response")
            .ok_or_else(|| key_error(&video_id, "response"))?;
        let body = response
            .get("body")
            .ok_or_else(|| key_error(&video_id, "body"))?;
        let video = response
            .get("video")
            .ok_or_else(|| key_error(&video_id, "video"))?;
        let title = video
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "title"))?;
        let mut subtitles = crate::Subtitles::default();
        let mut formats = Vec::new();
        let text_tracks: Vec<Value> = match body {
            Value::Object(body) => {
                if let Some(Value::Array(outputs)) = body.get("outputs") {
                    for output in outputs {
                        let Some(output_url) =
                            output.get("url").filter(|value| super::nuxt::truthy(value))
                        else {
                            continue;
                        };
                        let name = output.get("name").cloned().unwrap_or(Value::Null);
                        if name.as_str() == Some("m3u8") {
                            // `formats, subtitles = ...` replaces earlier outputs.
                            let manifest = output_url
                                .as_str()
                                .and_then(|url| Url::parse(url).ok())
                                .ok_or_else(|| unsupported("output URLs that are not URLs"))?;
                            let stream = super::extract_m3u8_master_optional(
                                client,
                                &manifest,
                                NAME,
                                &video_id,
                                Some("hls"),
                                true,
                                false,
                            );
                            formats = stream
                                .as_ref()
                                .map(|stream| stream.formats.clone())
                                .unwrap_or_default();
                            subtitles = stream.map(|stream| stream.subtitles).unwrap_or_default();
                            continue;
                        }
                        let mut fields = vec![
                            ("format_id", name.clone()),
                            ("tbr", int_or_none(output.get("bitrate"))),
                            ("url", output_url.clone()),
                        ];
                        if matches!(name.as_str(), Some("m4a" | "mp3")) {
                            fields.push(("vcodec", "none".into()));
                        } else {
                            fields.push(("height", int_or_none(output.get("height"))));
                            fields.push(("width", int_or_none(output.get("width"))));
                        }
                        let mut format = dict_format(fields, NAME, &video_id)?;
                        crate::info_order::mark_int_rates(&mut format);
                        formats.push(format);
                    }
                }
                match body.get("subtitles") {
                    Some(Value::Array(tracks)) => tracks.clone(),
                    _ => Vec::new(),
                }
            }
            Value::String(body) => {
                let mut m3u8_url = M3U8_URL
                    .captures(body)
                    .and_then(|found| quoted(&found).map(str::to_owned));
                if m3u8_url.is_none() {
                    let source = SOURCE
                        .captures(body)
                        .map(|found| found[1].to_owned())
                        .ok_or_else(|| regex_error(&video_id, "source"))?;
                    let get_attr = |key: &str| {
                        Regex::new(&format!(
                            r#"\b{key}\s*:\s*(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')"#
                        ))
                        .expect("static pattern")
                        .captures(&source)
                        .and_then(|found| quoted(&found).map(str::to_owned))
                        .ok_or_else(|| regex_error(&video_id, key))
                    };
                    if get_attr("integration")? == "verizon-media" {
                        m3u8_url = Some(format!(
                            "https://content.uplynk.com/{}.m3u8",
                            get_attr("id")?
                        ));
                    }
                }
                let Some(m3u8_url) = m3u8_url else {
                    return Err(Error::Reported(format!(
                        "[{NAME}] {video_id}: Failed to obtain m3u8 URL; {}",
                        crate::BUG_REPORTS_MESSAGE
                    )));
                };
                let stream = super::extract_m3u8_fatal_master(
                    client,
                    &Url::parse(&m3u8_url)
                        .map_err(|_| unsupported("HLS URLs that are not absolute"))?,
                    NAME,
                    &video_id,
                    Some("hls"),
                    true,
                )?;
                formats = stream.formats;
                subtitles = stream.subtitles;
                match TEXT_TRACKS.captures(body) {
                    Some(found) => {
                        match crate::json::from_str::<Value>(&crate::js_to_json::js_to_json(
                            &found[1],
                        )) {
                            Ok(Value::Array(tracks)) => tracks,
                            Ok(_) => Vec::new(),
                            Err(_) => {
                                return Err(unsupported("text tracks that do not parse"));
                            }
                        }
                    }
                    None => Vec::new(),
                }
            }
            _ => {
                return Err(unsupported(
                    "player bodies that are neither objects nor strings",
                ));
            }
        };
        for track in &text_tracks {
            let Some(track_url) = dict_get(track, &["file", "src"]) else {
                continue;
            };
            let track_url = track_url
                .as_str()
                .and_then(|url| Url::parse(url).ok())
                .ok_or_else(|| unsupported("text track URLs that are not URLs"))?;
            let label = track
                .get("label")
                .filter(|label| super::nuxt::truthy(label))
                .and_then(Value::as_str)
                .unwrap_or("English")
                .to_owned();
            let ext = super::direct::determine_ext(track_url.as_str())
                .unwrap_or_else(|| "unknown_video".into());
            let entry = SubtitleFormat {
                name: None,
                url: track_url,
                ext,
                protocol: None,
                fragments: Vec::new(),
                extra: Map::new(),
            };
            let label = Some(label);
            match subtitles.entries.iter_mut().find(|(key, _)| *key == label) {
                Some((_, tracks)) => tracks.push(entry),
                None => subtitles.entries.push((label, vec![entry])),
            }
        }
        let mut thumbnails = Vec::new();
        if let Some(Value::Array(items)) = video.get("thumbnails") {
            for thumbnail in items {
                let Some(thumbnail_url) = thumbnail
                    .get("url")
                    .filter(|value| super::nuxt::truthy(value))
                else {
                    continue;
                };
                thumbnails.push(json!({
                    "url": thumbnail_url,
                    "width": int_or_none(thumbnail.get("width")),
                    "height": int_or_none(thumbnail.get("height")),
                }));
            }
        }
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                (
                    "display_id",
                    video.get("friendly_title").cloned().unwrap_or(Value::Null),
                ),
                ("title", title),
                ("thumbnails", Value::Array(thumbnails)),
                (
                    "description",
                    dict_get(
                        video,
                        &["description", "ott_description", "short_description"],
                    )
                    .cloned()
                    .unwrap_or(Value::Null),
                ),
                (
                    "timestamp",
                    video
                        .get("published_at")
                        .and_then(Value::as_str)
                        .and_then(crate::dates::parse_iso8601)
                        .map_or(Value::Null, Value::from),
                ),
                ("duration", int_or_none(video.get("duration"))),
                ("view_count", int_or_none(video.get("request_count"))),
                ("average_rating", int_or_none(video.get("rating"))),
                ("season_number", int_or_none(video.get("season"))),
                ("episode_number", int_or_none(video.get("episode"))),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        info.subtitles = subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=url,width,height;subtitle=url");
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
            "https://player.zype.com/embed/5b400b834b32992a310622b9.js?api_key=jZ9GUhRmxcPvX7M3SlfejB6Hle9jyHTdk2jVxG7wOHPLODgncEKVdPYBhuz9iWXQ&autoplay=false"
        ));
        assert!(suitable(
            "https://player.zype.com/embed/5b400b834b32992a310622b9.html?app_key=x"
        ));
        assert!(!suitable(
            "https://player.zype.com/embed/5b400b834b32992a310622b9.js"
        ));
    }
}
