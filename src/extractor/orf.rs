//! Upstream `ORFONIE` (`yt_dlp/extractor/orf.py`): ORF ON episodes and
//! segments through the TVthek v4.3 `episode/encrypted` API (the obfuscated
//! base64 ID), with its HLS/DASH `sources`, `_embedded.subtitle` URLs,
//! metadata, the DRM error and `--no-playlist` segment selection.
//!
//! Not supported natively: episodes without a combined version (and
//! `orfon:prefer_segments_playlist`), which upstream returns as a
//! `multi_video` playlist of their segments (an explicit error); the
//! `orf:radio`, `orf:podcast`, `orf:iptv` and `orf:fm4:story` extractors.
use super::{Extractor, dict_info, download_json, extract_m3u8_master_optional};
use crate::{Error, Result, SubtitleFormat, Subtitles, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct OrfOn;

const NAME: &str = "orf:on";
const KEY: &str = "ORFON";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://on\.orf\.at/video/(?P<id>\d+)(?:/(?P<segment>\d+))?").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("ORF ON {what} are not supported natively"))
}

/// `traverse_obj(value, (*path))` through objects only.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().try_fold(value, |value, key| value.get(*key))
}

/// `...` over a list's items or an object's values.
fn branch(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(entries) => entries.values().collect(),
        _ => Vec::new(),
    }
}

fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

fn iso8601(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map(Value::from)
}

/// `_parse_metadata(api_json)`: the present fields in template order.
fn parse_metadata(api: &Value) -> Vec<(&'static str, Value)> {
    let first_text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| api.get(*key).filter(|value| value.is_string()))
            .cloned()
    };
    // `('id', {int}, {str_or_none})`.
    let id = api
        .get("id")
        .and_then(|id| id.as_i64().map(|id| id.to_string()))
        .map(Value::from);
    // `float_or_none(scale=1000)`.
    let duration = match api.get("exact_duration") {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .map(|duration| Value::from(duration / 1000.0));
    let age_limit = Some(super::videopress::parse_age_limit(
        api.get("age_classification"),
    ))
    .filter(|age| !age.is_null());
    let timestamp = ["date", "episode_date"]
        .iter()
        .find_map(|key| iso8601(api.get(*key)));
    [
        ("id", id),
        ("age_limit", age_limit),
        ("duration", duration),
        ("title", first_text(&["title", "headline"])),
        ("description", first_text(&["description", "teaser_text"])),
        (
            "media_type",
            api.get("video_type")
                .filter(|kind| kind.is_string())
                .cloned(),
        ),
        (
            "thumbnail",
            url_or_none(path(
                api,
                &[
                    "_embedded",
                    "image",
                    "public_urls",
                    "highlight_teaser",
                    "url",
                ],
            ))
            .map(Value::from),
        ),
        ("timestamp", timestamp),
        ("release_timestamp", iso8601(api.get("release_date"))),
        ("modified_timestamp", iso8601(api.get("updated_at"))),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| (key, value)))
    .collect()
}

/// `_extract_video_info(video_id, api_json)`.
fn extract_video_info(
    client: &Client,
    page: &Url,
    video_id: &str,
    api: &Value,
) -> Result<VideoInfo> {
    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    if let Some(Value::Object(sources)) = api.get("sources") {
        for (kind, entries) in sources {
            if kind != "hls" && kind != "dash" {
                continue;
            }
            for entry in branch(entries) {
                let Some(manifest_url) = url_or_none(entry.get("src")) else {
                    continue;
                };
                let manifest = Url::parse(&manifest_url)?;
                let stream = if kind == "hls" {
                    extract_m3u8_master_optional(
                        client,
                        &manifest,
                        NAME,
                        video_id,
                        Some("hls"),
                        true,
                        false,
                    )
                } else {
                    super::extract_mpd_optional_note(
                        client,
                        &manifest,
                        NAME,
                        video_id,
                        Some("dash"),
                        "Downloading MPD manifest",
                    )
                };
                if let Some(stream) = stream {
                    formats.extend(stream.formats);
                    super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            }
        }
    }
    if let Some(subtitle) = path(api, &["_embedded", "subtitle"]) {
        for key in [
            "xml_url", "sami_url", "stl_url", "ttml_url", "srt_url", "vtt_url",
        ] {
            let Some(subtitle_url) = url_or_none(subtitle.get(key)) else {
                continue;
            };
            let parsed = Url::parse(&subtitle_url)
                .map_err(|_| unsupported("subtitle URLs that are not absolute"))?;
            // Processing's `determine_ext(url).lower()`.
            let ext = super::direct::determine_ext(&subtitle_url)
                .unwrap_or_else(|| "unknown_video".into())
                .to_lowercase();
            let mut track = Subtitles::default();
            track.entries.push((
                Some("de".into()),
                vec![SubtitleFormat {
                    name: None,
                    url: parsed,
                    ext,
                    protocol: None,
                    fragments: Vec::new(),
                    extra: Map::new(),
                }],
            ));
            super::svt::merge_subtitles(&mut subtitles, track);
        }
    }
    let mut fields = vec![
        ("id", Value::from(video_id)),
        ("formats", Value::Null),
        ("subtitles", Value::Null),
        // `make_archive_id('ORFTVthek', video_id)`.
        ("_old_archive_ids", json!([format!("orftvthek {video_id}")])),
    ];
    for (key, value) in parse_metadata(api) {
        match fields.iter_mut().find(|(known, _)| *known == key) {
            Some(field) => field.1 = value,
            None => fields.push((key, value)),
        }
    }
    let mut info = dict_info(page, (NAME, KEY), fields, formats);
    info.subtitles = subtitles;
    Ok(info)
}

impl Extractor for OrfOn {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let segment_id = found.name("segment").map(|id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let encrypted = base64::engine::general_purpose::STANDARD
            .encode(format!("3dSlfek03nsLKdj4Jsd{video_id}"));
        let api_url = Url::parse(&format!(
            "https://api-tvthek.orf.at/api/v4.3/public/episode/encrypted/{encrypted}"
        ))?;
        let api = download_json(client, &api_url, NAME, &video_id)?;
        if api.get("is_drm_protected").is_some_and(super::nuxt::truthy) {
            // `report_drm(video_id)`.
            crate::no_formats::raise_expected(
                NAME,
                Some(&video_id),
                None,
                "This video is DRM protected",
            )?;
        }

        // `('_embedded', 'segments', lambda _, v: v['id'])`.
        let segments: Vec<&Value> = path(&api, &["_embedded", "segments"])
            .map(branch)
            .unwrap_or_default()
            .into_iter()
            .filter(|segment| segment.get("id").is_some_and(super::nuxt::truthy))
            .collect();
        // `str(v['id']) == segment_id`.
        let selected = segment_id.as_deref().and_then(|wanted| {
            segments.iter().copied().find(|segment| {
                let id = &segment["id"];
                id.as_str().map_or_else(|| id.to_string(), str::to_owned) == wanted
            })
        });
        if let (Some(selected), Some(segment_id)) = (selected, segment_id.as_deref()) {
            // `_yes_playlist(video_id, segment_id, playlist_label='episode',
            // video_label='segment')`.
            if super::no_playlist() {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading just the segment {segment_id} because of --no-playlist"
                ));
                return extract_video_info(client, url, segment_id, selected).map(Some);
            }
            crate::screen::screen(&format!(
                "[{NAME}] Downloading episode {video_id} - add --no-playlist to download just the segment {segment_id}"
            ));
        }

        // Some segmented videos have an unsegmented version in the root.
        let has_sources = api.get("sources").is_some_and(|sources| {
            branch(sources)
                .into_iter()
                .flat_map(branch)
                .any(|entry| url_or_none(entry.get("src")).is_some())
        });
        let prefer_segments =
            crate::manifest_args::configuration_arg("orfon", "prefer_segments_playlist", false)
                .is_some_and(|values| !values.is_empty());
        if prefer_segments || !has_sources {
            return Err(Error::UnsupportedHtml5(
                "ORF ON segment playlists (multi_video results) are not supported natively".into(),
            ));
        }
        extract_video_info(client, url, &video_id, &api).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        let found = VALID_URL
            .captures(
                "https://on.orf.at/video/14226549/15639808/jugendbande-einbrueche-aus-langeweile",
            )
            .unwrap();
        assert_eq!((&found["id"], &found["segment"]), ("14226549", "15639808"));
        assert!(suitable(
            "https://on.orf.at/video/14210000/school-of-champions-48"
        ));
        assert!(suitable("https://on.orf.at/video/3220355"));
        assert!(!suitable("https://on.orf.at/sendung/123"));
    }

    #[test]
    fn api_ids_are_obfuscated_like_upstream() {
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode("3dSlfek03nsLKdj4Jsd14210000"),
            "M2RTbGZlazAzbnNMS2RqNEpzZDE0MjEwMDAw"
        );
    }
}
