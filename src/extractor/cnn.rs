//! Upstream `CNNIE` (`yt_dlp/extractor/cnn.py`): the `video-player`
//! components of a CNN page, each resolved through the fave API (direct MP4
//! files, captions and metadata) and the medium API (an HLS master). One
//! player is the video itself; several make a playlist of the page's display
//! ID. `CNNIndonesiaIE` is not ported: its embed page no longer resolves.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::subtitles::{SubtitleFormat, Subtitles};
use crate::{ExtractResult, PlaylistInfo, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Cnn;

const NAME: &str = "CNN";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:edition|www|money|cnnespanol)\.)?cnn\.com/(?P<display_id>[^?#]+?)(?:[?#]|$|/index\.html)")
        .unwrap()
});
static WINDOW_ENV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"window\.env\s*=").unwrap());
static RESOLUTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-(?P<res>\d+x\d+)_(?P<tbr>\d+)k\.mp4").unwrap());

/// `_match_valid_url`'s suitability.
pub fn suitable(url: &str) -> bool {
    display_id_of(url).is_some()
}

/// The display ID, unless the path is an `audio/` URL (`(?!audio/)`).
fn display_id_of(url: &str) -> Option<String> {
    let found = VALID_URL.captures(url)?;
    let display_id = found["display_id"].to_owned();
    (!display_id.starts_with("audio/")).then_some(display_id)
}

fn key_text(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// `int_or_none(scale=1000)` of a millisecond timestamp.
fn scaled_seconds(value: Option<&Value>) -> Option<Value> {
    let millis = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    Some(Value::from((millis / 1000.0) as i64))
}

/// A dict update: an existing key keeps its place, a new key is appended.
fn upsert(fields: &mut Vec<(String, Value)>, key: &str, value: Value) {
    match fields.iter_mut().find(|(name, _)| name == key) {
        Some(slot) => slot.1 = value,
        None => fields.push((key.to_owned(), value)),
    }
}

fn add_subtitle(subtitles: &mut Subtitles, language: &str, track: SubtitleFormat) {
    match subtitles
        .entries
        .iter_mut()
        .find(|(key, _)| key.as_deref() == Some(language))
    {
        Some((_, tracks)) => tracks.push(track),
        None => subtitles.entries.push((Some(language.into()), vec![track])),
    }
}

/// One player's entry before the page's display ID is applied.
struct Entry {
    media_id: String,
    fields: Vec<(String, Value)>,
    formats: Vec<VideoFormat>,
    subtitles: Subtitles,
}

/// The fave API's direct files, captions and metadata, and the medium API's
/// HLS master for one `video-player` element.
fn entry(
    client: &Client,
    attributes: &Map<String, Value>,
    app_id: Option<&str>,
) -> Result<Option<Entry>> {
    let Some(media_id) = key_text(attributes.get("data-media-id")) else {
        return Ok(None);
    };
    let mut fields: Vec<(String, Value)> = Vec::new();
    if let Some(headline) = key_text(attributes.get("data-headline")) {
        upsert(
            &mut fields,
            "title",
            Value::from(crate::html::clean_html(headline)),
        );
    }
    if let Some(description) = key_text(attributes.get("data-description")) {
        upsert(
            &mut fields,
            "description",
            Value::from(crate::html::clean_html(description)),
        );
    }
    if let Some(duration) = key_text(attributes.get("data-duration"))
        .and_then(crate::parse_duration)
        .and_then(|number| serde_json::to_value(number).ok())
    {
        upsert(&mut fields, "duration", duration);
    }
    if let Some(publish) =
        key_text(attributes.get("data-publish-date")).and_then(crate::dates::parse_iso8601)
    {
        upsert(&mut fields, "timestamp", Value::from(publish));
    }
    if let Some(poster) = key_text(attributes.get("data-poster-image-override"))
        && let Ok(poster) = serde_json::from_str::<Value>(poster)
        && let Some(uri) = poster
            .pointer("/big/uri")
            .and_then(Value::as_str)
            .filter(|uri| Url::parse(uri).is_ok())
    {
        let mut thumbnail = Url::parse(uri)?;
        thumbnail.set_query(Some("c=original"));
        upsert(&mut fields, "thumbnail", thumbnail.as_str().into());
    }
    if let Some(slug) = key_text(attributes.get("data-video-slug")) {
        upsert(&mut fields, "display_id", slug.into());
    }

    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    if let Some(parent_uri) = key_text(attributes.get("data-video-resource-parent-uri")) {
        let fave = Url::parse_with_params(
            "https://fave.api.cnn.io/v1/video",
            [("id", media_id), ("stellarUri", parent_uri)],
        )?;
        let video_data = download_json(client, &fave, NAME, media_id).unwrap_or(Value::Null);
        for file in video_data
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(direct_url) = file
                .get("fileUri")
                .and_then(Value::as_str)
                .filter(|uri| Url::parse(uri).is_ok())
            else {
                continue;
            };
            let (resolution, bitrate) = match RESOLUTION.captures(direct_url) {
                Some(found) => (Some(found["res"].to_owned()), Some(found["tbr"].to_owned())),
                None => (None, None),
            };
            let tbr = bitrate
                .and_then(|text| text.parse::<i64>().ok())
                .map_or(Value::Null, Value::from);
            let mut fields: Vec<(&str, Value)> = vec![
                ("url", direct_url.into()),
                ("format_id", "direct".into()),
                ("quality", 1.into()),
                ("tbr", tbr),
            ];
            if let Some(resolution) = resolution
                && let (Some(width), Some(height)) = super::html5::parse_resolution(&resolution)
            {
                fields.push(("width", width.into()));
                fields.push(("height", height.into()));
            }
            let mut format = dict_format(fields, NAME, media_id)?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
        for track_data in video_data
            .pointer("/closedCaptions/types")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(track) = track_data.get("track") else {
                continue;
            };
            let Some(track_url) = key_text(track.get("url")).filter(|u| Url::parse(u).is_ok())
            else {
                continue;
            };
            let language = key_text(track_data.get("lang")).unwrap_or("en");
            let ext =
                super::direct::determine_ext(track_url).unwrap_or_else(|| "unknown_video".into());
            add_subtitle(
                &mut subtitles,
                language,
                SubtitleFormat {
                    name: track
                        .get("label")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    url: Url::parse(track_url)?,
                    ext,
                    protocol: None,
                    fragments: Vec::new(),
                    extra: Default::default(),
                },
            );
        }
        // `traverse_obj(video_data, {...})`: these override the page's values.
        let overrides = [
            (
                "timestamp",
                scaled_seconds(video_data.pointer("/dateCreated/uts")),
            ),
            (
                "description",
                key_text(video_data.get("description"))
                    .map(|text| Value::from(crate::html::clean_html(text))),
            ),
            (
                "title",
                key_text(video_data.get("headline")).map(Value::from),
            ),
            (
                "modified_timestamp",
                scaled_seconds(video_data.pointer("/lastModified/uts")),
            ),
            (
                "duration",
                video_data
                    .get("trt")
                    .and_then(Value::as_f64)
                    .map(|t| Value::from(t as i64)),
            ),
        ];
        for (key, value) in overrides {
            if let Some(value) = value {
                upsert(&mut fields, key, value);
            }
        }
    }

    if let Some(app_id) = app_id {
        let medium = Url::parse_with_params(
            &format!("https://medium.ngtv.io/v2/media/{media_id}/desktop"),
            [("appId", app_id)],
        )?;
        let media_data = download_json(client, &medium, NAME, media_id).unwrap_or(Value::Null);
        let m3u8 = media_data
            .pointer("/media/desktop/unprotected/unencrypted/url")
            .and_then(Value::as_str)
            .and_then(|text| Url::parse(text).ok());
        if let Some(m3u8) = m3u8
            && let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &m3u8,
                NAME,
                media_id,
                Some("hls"),
                true,
                false,
            )
        {
            formats.extend(stream.formats);
            super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
        }
    }

    Ok(Some(Entry {
        media_id: media_id.to_owned(),
        fields,
        formats,
        subtitles,
    }))
}

/// The entry's info dict in upstream's key order.
fn entry_info(page: &Url, entry: Entry) -> VideoInfo {
    let mut fields: Vec<(String, Value)> = entry.fields;
    upsert(&mut fields, "id", entry.media_id.as_str().into());
    let mut fields: Vec<(&str, Value)> = fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    fields.push(("formats", Value::Null));
    fields.push(("subtitles", Value::Null));
    let mut info = dict_info(page, (NAME, NAME), fields, entry.formats);
    info.subtitles = entry.subtitles;
    // Upstream's track dicts are `{url, name}` plus processing's `ext`.
    if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
        shape.push_str(";subtitle=url,name,ext");
    }
    info
}

impl Extractor for Cnn {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(display_id) = display_id_of(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let app_id = super::search_json(&WINDOW_ENV, &webpage, false)
            .flatten()
            .and_then(|env| {
                env.get("TOP_AUTH_SERVICE_APP_ID")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });

        let mut entries = Vec::new();
        let players = crate::html_elements::elements_text_and_html_by_attribute(
            "data-component-name",
            "video-player",
            &webpage,
        )
        .unwrap_or_default();
        for (_, html) in players {
            let attributes = crate::html_elements::extract_attributes(&html);
            if let Some(found) = entry(client, &attributes, app_id.as_deref())? {
                entries.push(found);
            }
        }

        // `{**entries[0], 'display_id': display_id}` for a single video.
        if let [single] = entries.as_mut_slice() {
            upsert(&mut single.fields, "display_id", display_id.as_str().into());
        }
        let mut infos: Vec<VideoInfo> = entries
            .into_iter()
            .map(|found| entry_info(url, found))
            .collect();
        if infos.len() == 1 {
            return Ok(infos.pop().map(ExtractResult::Video));
        }

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries: infos,
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
            "https://www.cnn.com/2024/05/31/sport/video/jadon-sancho-borussia-dortmund-champions-league-exclusive-spt-intl"
        ));
        assert!(suitable(
            "https://edition.cnn.com/2024/06/11/style/king-charles-portrait-vandalized/index.html"
        ));
        assert!(suitable(
            "https://cnnespanol.cnn.com/video/ataque-misil-israel-beirut-libano-octubre-trax"
        ));
        assert!(!suitable("https://www.cnn.com/audio/podcasts/x"));
        assert!(!suitable("https://www.example.com/video"));
    }
}
