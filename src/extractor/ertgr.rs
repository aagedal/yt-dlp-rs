//! Upstream `ERTFlixCodenameIE` (`yt_dlp/extractor/ertgr.py`): an ERTFLIX title
//! by its codename (`ertflix:<codename>`) through the ERTFLIX player API: the
//! HLS and DASH manifests and direct files of its `main` media role. The
//! VOD pages of `ERTFlixIE` (`ertflix.gr/vod/<id>`) resolve their tile to that
//! codename as a `url_transparent` result; series pages and the ert.gr webtv
//! embeds (`ERTWebtvEmbedIE`) are not ported yet.
use super::{
    Extractor, dict_format, dict_info, download_json, post_json_errnote_optional, update_url_query,
};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct ERTFlixCodename;

const NAME: &str = "ertflix:codename";
const KEY: &str = "ERTFlixCodename";
const API: &str = "https://api.app.ertflix.gr/v1/Player/AcquireContent";
/// `json.dumps` of the `$headers` query parameter (Python's separators).
const API_HEADERS: &str = r#"{"X-Api-Date-Format": "iso", "X-Api-Camel-Case": false}"#;

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| crate::extractor::url_regex(r"^ertflix:(?P<id>[\w-]+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `url_or_none` on a JSON value: the string itself when it is a URL.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    (Url::parse(text).is_ok() && !text.contains(char::is_whitespace)).then(|| text.to_owned())
}

/// `str_or_none` on a JSON value.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

impl Extractor for ERTFlixCodename {
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
        let api = Url::parse(API)?;
        let api = update_url_query(
            &api,
            &[
                ("platformCodename", "www"),
                ("$headers", API_HEADERS),
                ("codename", video_id.as_str()),
            ],
        );
        // `_call_api` (non-fatal): an unsuccessful answer is no media info.
        let response = download_json(client, &api, NAME, &video_id)
            .ok()
            .filter(|response| {
                response
                    .get("Result")
                    .and_then(|result| result.get("Success"))
                    == Some(&Value::Bool(true))
            });
        let Some(response) = response else {
            return Err(Error::UnsupportedHtml5(format!(
                "ERTFLIX codename {video_id} without a successful player API answer returns nothing upstream and is not supported natively"
            )));
        };

        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let media_files = response
            .get("MediaFiles")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for media in media_files
            .iter()
            .filter(|media| media.get("RoleCodename").and_then(Value::as_str) == Some("main"))
        {
            let items = media
                .get("Formats")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in &items {
                let Some(format_url) = url_or_none(item.get("Url")) else {
                    continue;
                };
                let ext = super::direct::determine_ext(&format_url).unwrap_or_default();
                if ext == "m3u8" {
                    let manifest = Url::parse(&format_url)?;
                    if let Some(stream) = super::extract_m3u8_master_optional(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some("hls"),
                        true,
                        false,
                    ) {
                        formats.extend(stream.formats);
                        for (lang, tracks) in stream.subtitles.entries {
                            match subtitles.entries.iter_mut().find(|(key, _)| *key == lang) {
                                Some((_, existing)) => existing.extend(tracks),
                                None => subtitles.entries.push((lang, tracks)),
                            }
                        }
                    }
                } else if ext == "mpd" {
                    let manifest = Url::parse(&format_url)?;
                    if let Some(stream) = super::extract_mpd_optional_note(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some("dash"),
                        "Downloading MPD manifest",
                    ) {
                        formats.extend(stream.formats);
                    }
                } else {
                    formats.push(dict_format(
                        vec![
                            ("url", format_url.clone().into()),
                            (
                                "format_id",
                                str_or_none(item.get("Id")).map_or(Value::Null, Value::from),
                            ),
                        ],
                        NAME,
                        &video_id,
                    )?);
                }
            }
        }

        if formats.is_empty() {
            return Err(Error::UnsupportedHtml5(format!(
                "ERTFLIX codename {video_id} without formats returns nothing upstream and is not supported natively"
            )));
        }
        // `'title': self._generic_title(url)`: the codename is the URL's basename.
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.clone().into()),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("title", video_id.clone().into()),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

pub struct ERTFlix;

const VOD_NAME: &str = "ertflix";
const VOD_KEY: &str = "ERTFlix";
const TILES_API: &str = "https://api.app.ertflix.gr/v2/Tile/GetTiles";
/// `json.dumps` of the `$headers` parameter of `_call_api` with `data`.
const TILES_API_HEADERS: &str = r#"{"X-Api-Date-Format": "iso", "X-Api-Camel-Case": false, "Content-Type": "application/json;charset=utf-8"}"#;

static VOD_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://www\.ertflix\.gr/(?:[^/]+/)?(?:series|vod)/(?P<id>[a-z]{3}\.\d+)",
    )
    .unwrap()
});

/// `_match_id` for `ERTFlixIE`.
pub fn vod_suitable(url: &str) -> bool {
    VOD_VALID_URL.is_match(url)
}

fn tile_error(video_id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{VOD_NAME}] {video_id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_call_api_get_tiles(video_id)`: the tile whose `Id` is the video ID
/// (`_call_api` answers `None` unless `Result.Success` is true).
fn matching_tile(client: &Client, video_id: &str) -> Result<Value> {
    let body =
        format!(r#"{{"platformCodename": "www", "RequestedTiles": [{{"Id": "{video_id}"}}]}}"#);
    let api = update_url_query(&Url::parse(TILES_API)?, &[("$headers", TILES_API_HEADERS)]);
    let response = post_json_errnote_optional(
        client,
        &api,
        (VOD_NAME, video_id),
        (
            "Downloading JSON metadata",
            "Unable to download JSON metadata",
        ),
        &[("Content-Type", "application/json;charset=utf-8")],
        body,
    )?
    .filter(|response| {
        response
            .get("Result")
            .and_then(|result| result.get("Success"))
            == Some(&Value::Bool(true))
    });
    let tiles = response
        .as_ref()
        .and_then(|response| response.get("Tiles"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    tiles
        .into_iter()
        .find(|tile| tile.get("Id").and_then(Value::as_str) == Some(video_id))
        .ok_or_else(|| tile_error(video_id, "No matching tile found"))
}

/// `dict_get(episode, keys)` over the string or number fields used here.
fn text_field(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

impl Extractor for ERTFlix {
    fn key(&self) -> &'static str {
        VOD_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VOD_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        if video_id.starts_with("ser.") {
            return Err(Error::UnsupportedHtml5(
                "ERTFLIX series pages are not supported natively".into(),
            ));
        }
        crate::screen::set_extractor(VOD_NAME);
        crate::screen::screen(&format!(
            "[{VOD_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let tile = matching_tile(client, &video_id)?;

        // `_extract_episode`: `None` unless the tile has a codename and title
        // and is playable.
        let codename = text_field(&tile, "Codename").filter(|text| !text.is_empty());
        let title = text_field(&tile, "Title").filter(|text| !text.is_empty());
        let playable = tile
            .get("HasPlayableStream")
            .is_none_or(super::nuxt::truthy);
        let (Some(codename), Some(title), true) = (codename, title, playable) else {
            return Err(Error::UnsupportedHtml5(
                "ERTFLIX tiles without a codename, title or playable stream are not supported natively"
                    .into(),
            ));
        };
        let thumbnail = match tile.get("Images").or_else(|| tile.get("Image")) {
            Some(Value::Array(images)) => images.clone(),
            Some(image) => vec![image.clone()],
            None => Vec::new(),
        }
        .iter()
        .find(|thumb| thumb.get("IsMain").is_some_and(super::nuxt::truthy))
        .and_then(|thumb| thumb.get("Url").and_then(Value::as_str))
        .and_then(super::json_ld::url_or_none);
        let description = text_field(&tile, "ShortDescription")
            .filter(|text| !text.is_empty())
            .or_else(|| text_field(&tile, "TinyDescription").filter(|text| !text.is_empty()))
            .map(|text| crate::html::clean_html(&text));
        let timestamp =
            text_field(&tile, "PublishDate").and_then(|date| crate::dates::parse_iso8601(&date));
        let duration = tile.get("DurationSeconds").and_then(Value::as_i64);
        let age_limit = {
            let rating = tile.get("AgeRating");
            let adult = tile.get("IsAdultContent").is_some_and(super::nuxt::truthy);
            let kids = tile.get("IsKidsContent").is_some_and(super::nuxt::truthy);
            super::videopress::parse_age_limit(rating.filter(|value| super::nuxt::truthy(value)))
                .as_i64()
                .or(if adult {
                    Some(18)
                } else if kids {
                    Some(0)
                } else {
                    None
                })
        };

        // `_extract_episode`'s `url_transparent` result, resolved through the
        // codename extractor, with its non-`None` fields overlaid.
        let target = Url::parse(&format!("ertflix:{codename}"))?;
        let Some(mut info) = ERTFlixCodename.extract(client, &target)? else {
            return Ok(None);
        };
        let mut keys = Vec::new();
        if let Some(thumbnail) = thumbnail {
            keys.push("thumbnail");
            crate::metadata_parser::set_field(&mut info, "thumbnail", Value::from(thumbnail));
        }
        if let Some(episode_id) = tile.get("Id").filter(|id| !id.is_null()) {
            keys.push("episode_id");
            crate::metadata_parser::set_field(&mut info, "episode_id", episode_id.clone());
        }
        keys.push("title");
        info.title = title;
        info.extra.shift_remove(crate::info::TITLE_MISSING);
        if let Some(alt_title) = text_field(&tile, "Subtitle") {
            keys.push("alt_title");
            crate::metadata_parser::set_field(&mut info, "alt_title", Value::from(alt_title));
        }
        if let Some(description) = description {
            keys.push("description");
            info.metadata.description = Some(description);
        }
        if let Some(timestamp) = timestamp {
            keys.push("timestamp");
            crate::metadata_parser::set_field(&mut info, "timestamp", Value::from(timestamp));
        }
        if let Some(duration) = duration {
            keys.push("duration");
            crate::metadata_parser::set_field(&mut info, "duration", Value::from(duration));
        }
        if let Some(age_limit) = age_limit {
            keys.push("age_limit");
            crate::metadata_parser::set_field(&mut info, "age_limit", Value::from(age_limit));
        }
        info.webpage_url = url.clone();
        let inner_shape = match info.extra.get(crate::info_order::INFO_SHAPE) {
            Some(Value::String(shape)) => shape.clone(),
            _ => String::new(),
        };
        let shape = format!("{inner_shape}|transparent:{}", keys.join(","));
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info, &shape,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vod_urls_follow_upstream_valid_url() {
        assert!(vod_suitable(
            "https://www.ertflix.gr/vod/vod.173258-aoratoi-ergates"
        ));
        assert!(vod_suitable(
            "https://www.ertflix.gr/en/series/ser.3448-monogramma?season=1"
        ));
        assert!(!vod_suitable("https://www.ertflix.gr/vod/"));
    }

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("ertflix:monogramma-praxitelis-tzanoylinos"));
        assert!(!suitable(
            "https://www.ertflix.gr/series/ser.3448-monogramma"
        ));
    }
}
