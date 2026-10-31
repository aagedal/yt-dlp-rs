//! Upstream `RTVCKalturaIE` (`yt_dlp/extractor/rtvcplay.py`): the Señal
//! Colombia Kaltura player page's `config` script (its HLS `sources`) and
//! the channel's CMS metadata and HLS stream. `RTVCPlayIE`'s live channel and
//! single-asset pages (hydration state HLS) are ported as `RtvcPlay`; its
//! program and podcast pages are not, and `RTVCPlayEmbedIE` is not ported.
use super::{
    Extractor, dict_format, dict_info, download_json_note, download_webpage,
    extract_m3u8_master_optional,
};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct RtvcKaltura;
pub struct RtvcPlay;

const PLAY_NAME: &str = "RTVCPlay";
static PLAY_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?rtvcplay\.co/(?P<category>[^/]+)/(?:[^?#]+/)?(?P<id>[\w-]+)",
    )
    .unwrap()
});
static HYDRATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__RTVCPLAY_STATE__\s*=").unwrap());

const NAME: &str = "RTVCKaltura";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://media\.rtvc\.gov\.co/kalturartvc/(?P<id>[\w-]+)")
        .unwrap()
});
/// `_extract_player_config`'s start pattern.
static PLAYER_CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<script\b[^>]*>[^<]*(?:var|let|const)\s+config\s*=").unwrap());
/// `re.sub(r'"\s*\+\s*"', '', webpage)`: joins concatenated string literals.
static STRING_JOIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""\s*\+\s*""#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// The non-`None` string `key` of an object (`traverse_obj(..., {str})`).
fn string(data: &Value, path: &str) -> Option<Value> {
    data.pointer(path)
        .and_then(Value::as_str)
        .map(|text| text.to_owned().into())
}

/// `traverse_obj(data, ('sources', ..., lambda _, v: url_or_none(v['url'])))`
/// as `(url, mimetype)` pairs: each source object under `sources`, whether
/// directly or in a list keyed by its format.
fn sources(config: &Value) -> Vec<(String, Option<String>)> {
    let mut found = Vec::new();
    let mut visit = |source: &Value| {
        if let Some(url) = source
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            let mimetype = source
                .get("mimetype")
                .and_then(Value::as_str)
                .map(str::to_owned);
            found.push((url, mimetype));
        }
    };
    match config.get("sources") {
        Some(Value::Object(groups)) => {
            for group in groups.values() {
                match group {
                    Value::Array(items) => items.iter().for_each(&mut visit),
                    other => visit(other),
                }
            }
        }
        Some(Value::Array(items)) => items.iter().for_each(&mut visit),
        _ => {}
    }
    found
}

/// `_extract_formats_and_subtitles_player_config`: HLS sources are expanded,
/// other sources are direct formats.
fn formats_from_config(
    client: &Client,
    config: &Value,
    id: &str,
) -> Result<(Vec<crate::VideoFormat>, Subtitles)> {
    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    for (source_url, mimetype) in sources(config) {
        let ext = match mimetype.as_deref().and_then(mimetype_ext) {
            Some(ext) => ext.to_owned(),
            None => {
                super::direct::determine_ext(&source_url).unwrap_or_else(|| "unknown_video".into())
            }
        };
        if ext == "m3u8" {
            let manifest = Url::parse(&source_url)?;
            if let Some(stream) =
                extract_m3u8_master_optional(client, &manifest, NAME, id, None, true, false)
            {
                formats.extend(stream.formats);
                merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else {
            formats.push(dict_format(
                vec![("url", source_url.into()), ("ext", ext.into())],
                NAME,
                id,
            )?);
        }
    }
    Ok((formats, subtitles))
}

/// Appends `source`'s tracks to `target`'s languages.
fn merge_subtitles(target: &mut Subtitles, source: Subtitles) {
    for (language, tracks) in source.entries {
        match target.entries.iter_mut().find(|(key, _)| *key == language) {
            Some((_, existing)) => existing.extend(tracks),
            None => target.entries.push((language, tracks)),
        }
    }
}

/// `mimetype2ext` for the MIME types a Kaltura source declares.
fn mimetype_ext(mimetype: &str) -> Option<&'static str> {
    match mimetype.to_ascii_lowercase().as_str() {
        "application/x-mpegurl" | "application/vnd.apple.mpegurl" => Some("m3u8"),
        "video/mp4" => Some("mp4"),
        _ => None,
    }
}

/// `_extract_player_config`: the page's `config` object, JavaScript syntax
/// allowed.
fn player_config(webpage: &str, id: &str) -> Result<Value> {
    let joined = STRING_JOIN.replace_all(webpage, "");
    match super::search_json(&PLAYER_CONFIG, &joined, true) {
        Some(Some(config)) => Ok(config),
        _ => Err(Error::Reported(format!(
            "[{NAME}] {id}: Unable to extract player_config; {}",
            crate::BUG_REPORTS_MESSAGE
        ))),
    }
}

impl Extractor for RtvcKaltura {
    fn key(&self) -> &'static str {
        NAME
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
        let webpage = download_webpage(client, url, NAME, &video_id)?;

        let config = player_config(&webpage, &video_id)?;
        let (mut formats, mut subtitles) = formats_from_config(client, &config, &video_id)?;

        let channel_id = config
            .pointer("/rtvcplay/channelId")
            .filter(|value| !value.is_null() && **value != Value::Bool(false))
            .map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            });
        let metadata: Option<Value> = match channel_id {
            None => None,
            Some(channel_id) => {
                let api = Url::parse(&format!(
                    "https://cms.rtvcplay.co/api/v1/taxonomy_term/streaming/{channel_id}"
                ))?;
                optional_json(client, &api, &video_id)?
            }
        };

        let hls = metadata
            .as_ref()
            .and_then(|meta| meta.pointer("/channel/hls"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        if let Some(hls) = hls {
            let manifest = Url::parse(&hls)?;
            if let Some(stream) =
                extract_m3u8_master_optional(client, &manifest, NAME, &video_id, None, true, false)
            {
                formats.extend(stream.formats);
                merge_subtitles(&mut subtitles, stream.subtitles);
            }
        }

        let meta = metadata.unwrap_or(Value::Null);
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("is_live", true.into()),
        ];
        for (key, value) in [
            ("title", string(&meta, "/title")),
            ("description", string(&meta, "/description")),
            ("thumbnail", url_value(&meta, "/channel/image/logo/path")),
        ] {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

/// `_download_json(..., fatal=False)` for the CMS API.
fn optional_json(client: &Client, url: &Url, id: &str) -> Result<Option<Value>> {
    match download_json_note(client, url, NAME, id, "Downloading JSON metadata") {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// `traverse_obj(data, path, {url_or_none})` as a value.
fn url_value(data: &Value, path: &str) -> Option<Value> {
    data.pointer(path)
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map(Value::from)
}

/// `RTVCPlayIE._VALID_URL` (`(?!embed)` on the category, checked by hand).
pub fn play_suitable(url: &str) -> bool {
    PLAY_URL
        .captures(url)
        .is_some_and(|found| !found["category"].starts_with("embed"))
}

impl Extractor for RtvcPlay {
    fn key(&self) -> &'static str {
        PLAY_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !play_suitable(url.as_str()) {
            return Ok(None);
        }
        let found = PLAY_URL.captures(url.as_str()).expect("checked above");
        let video_id = found["id"].to_owned();
        let is_live = &found["category"] == "en-vivo";
        crate::screen::set_extractor(PLAY_NAME);
        crate::screen::screen(&format!(
            "[{PLAY_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, PLAY_NAME, &video_id)?;
        let hydration = match super::search_json(&HYDRATION, &webpage, true) {
            Some(Some(hydration)) => hydration,
            _ => {
                return Err(Error::Reported(format!(
                    "[{PLAY_NAME}] {video_id}: Unable to extract hydration; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let current = hydration
            .pointer("/content/currentContent")
            .ok_or_else(|| super::clyp::key_error(&video_id, "content"))?;

        let asset_id = current
            .pointer("/video/assetid")
            .filter(|value| super::nuxt::truthy(value))
            .and_then(Value::as_str);
        let hls_url = match asset_id {
            Some(asset_id) => {
                let base = current
                    .get("base_url_hls")
                    .and_then(Value::as_str)
                    .ok_or_else(|| super::clyp::key_error(&video_id, "base_url_hls"))?;
                Some(base.replace("[node:field_asset_id]", asset_id))
            }
            None => current
                .pointer("/channel/hls")
                .and_then(Value::as_str)
                .map(str::to_owned),
        };
        let Some(hls_url) = hls_url else {
            return Err(Error::UnsupportedHtml5(
                "RTVCPlay program and podcast pages are not supported natively".into(),
            ));
        };

        let title = current.get("title").cloned().filter(Value::is_string);
        let description = current.get("description").cloned().filter(Value::is_string);
        let thumbnail = [
            "/channel/image/logo/path",
            "/resource/image/cover_desktop/path",
        ]
        .iter()
        .find_map(|path| {
            current
                .pointer(path)
                .filter(|value| !value.is_null())
                .cloned()
        });

        let manifest = Url::parse(&hls_url)?;
        let stream =
            super::extract_m3u8_fatal_master(client, &manifest, PLAY_NAME, &video_id, None, true)?;

        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("is_live", is_live.into()),
        ];
        for (key, value) in [
            ("title", title),
            ("description", description),
            ("thumbnail", thumbnail),
        ] {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        let mut info = dict_info(url, (PLAY_NAME, PLAY_NAME), fields, stream.formats);
        info.subtitles = stream.subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://media.rtvc.gov.co/kalturartvc/indexSC.html"
        ));
        assert!(!suitable("https://media.rtvc.gov.co/"));
    }

    #[test]
    fn player_config_sources_are_found_in_groups() {
        let config: Value = serde_json::json!({
            "sources": {"hls": [{"mimetype": "application/x-mpegurl", "url": "https://a.example/x.m3u8"}]},
        });
        assert_eq!(
            sources(&config),
            vec![(
                "https://a.example/x.m3u8".to_owned(),
                Some("application/x-mpegurl".to_owned())
            )]
        );
    }

    #[test]
    fn play_urls_follow_upstream_valid_url() {
        assert!(play_suitable(
            "https://www.rtvcplay.co/en-vivo/canal-institucional"
        ));
        assert!(play_suitable(
            "https://www.rtvcplay.co/peliculas-ficcion/senoritas"
        ));
        assert!(!play_suitable(
            "https://www.rtvcplay.co/embed/72b0e699-248b-4929-a4a8-3782702fa7f9"
        ));
    }
}
