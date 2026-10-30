//! Upstream `StreaksIE` (`yt_dlp/extractor/streaks.py`): a player or playback
//! URL's playback API JSON, its HLS sources as non-fatal formats and its
//! captions. Upstream's `ssai` live sessions, `live_from_start` and DRM
//! handling are not ported: DRM-only media and live `ssai` sources error
//! out rather than guessing.
use super::{Extractor, direct, extract_m3u8_master_optional};
use crate::{Error, Result, SubtitleFormat, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Streaks;

const NAME: &str = "Streaks";

static PLAYER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://players\.streaks\.jp/(?P<project_id>[\w-]+)/(?P<api_key>[\da-f]+)/index\.html\?(?:[^#]+&)?m=(?P<id>(?:ref:)?[\w-]+)",
    )
    .unwrap()
});
static PLAYBACK_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://playback\.api\.streaks\.jp/v1/projects/(?P<project_id>[\w-]+)/medias/(?P<id>(?:ref:)?[\w-]+)",
    )
    .unwrap()
});

/// `_match_valid_url`: the first of the two patterns that matches.
fn captures(url: &str) -> Option<(String, Option<String>, String)> {
    for pattern in [&*PLAYER_URL, &*PLAYBACK_URL] {
        if let Some(found) = pattern.captures(url) {
            let project = found["project_id"].to_owned();
            let api_key = found.name("api_key").map(|key| key.as_str().to_owned());
            return Some((project, api_key, found["id"].to_owned()));
        }
    }
    None
}

/// `value[key]` as a string for `clean_html`-style fields; `None` when absent,
/// null or empty.
fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => {
            let cleaned = crate::html::clean_html(text);
            (!cleaned.is_empty()).then_some(cleaned)
        }
        _ => None,
    }
}

/// `traverse_obj(error, ('id', {int_or_none}))`.
fn int_id(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// The playback API response for `media_id`, or the error upstream raises for
/// its `403`/`404` error bodies.
fn playback_json(
    client: &Client,
    project: &str,
    media_id: &str,
    headers: &[(&str, &str)],
) -> Result<Value> {
    let api = Url::parse(&format!(
        "https://playback.api.streaks.jp/v1/projects/{project}/medias/{media_id}"
    ))?;
    let mut all = vec![
        ("Accept", "application/json"),
        ("Origin", "https://players.streaks.jp"),
    ];
    all.extend_from_slice(headers);
    crate::screen::screen(&format!(
        "[{NAME}] {media_id}: Downloading STREAKS playback API JSON"
    ));
    let mut response = direct::request_expecting(client, &api, &all, &[403, 404])?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut response, &mut body).map_err(Error::ExtractionRead)?;
    if status == 200 {
        let body = String::from_utf8_lossy(&body);
        return crate::json::from_str(&body).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {media_id}: Failed to parse JSON  (caused by {error})"
            ))
        });
    }
    let reason = response.status().canonical_reason().unwrap_or_default();
    let error: Option<Value> = serde_json::from_slice(&body).ok();
    if error.is_none() {
        // `_parse_json(..., fatal=False)`'s warning.
        let cause = crate::json::python_decode_error_text(&String::from_utf8_lossy(&body))
            .unwrap_or_default();
        crate::screen::warning(&format!(
            "[{NAME}] {media_id}: Failed to parse JSON: {cause}"
        ));
    }
    let message = text(error.as_ref().and_then(|error| error.get("message")));
    let code = text(error.as_ref().and_then(|error| error.get("code")));
    let error_id = int_id(error.as_ref().and_then(|error| error.get("id")));
    if code.as_deref() == Some("REQUEST_FAILED") {
        match error_id {
            Some(124) => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {media_id}: This video is geo restricted to Japan"
                )));
            }
            Some(126) => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {media_id}: Access is denied (possibly due to invalid/missing API key)"
                )));
            }
            _ => {}
        }
    }
    if code.as_deref() == Some("MEDIA_NOT_FOUND") {
        // `expected=True`: no bug-report suffix.
        let text = join(&[code.as_deref(), message.as_deref()]);
        return Err(Error::Reported(format!("[{NAME}] {media_id}: {text}")));
    }
    if code.is_some() || message.is_some() {
        let id_text = error_id.map(|id| id.to_string());
        let text = join(&[code.as_deref(), id_text.as_deref(), message.as_deref()]);
        return Err(Error::Reported(format!(
            "[{NAME}] {media_id}: {text}; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    Err(Error::Reported(format!(
        "[{NAME}] {media_id}: Unable to download JSON metadata: HTTP Error {status}: {reason} (caused by <HTTPError {status}: {reason}>)"
    )))
}

/// `join_nonempty(..., delim=': ')`.
fn join(parts: &[Option<&str>]) -> String {
    parts
        .iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(": ")
}

/// `thumbnail` or `poster`'s `src` when it is a URL.
fn thumbnail(response: &Value) -> Option<String> {
    ["thumbnail", "poster"].iter().find_map(|key| {
        response
            .get(*key)
            .and_then(|value| value.get("src"))
            .and_then(Value::as_str)
            .filter(|src| Url::parse(src).is_ok())
            .map(str::to_owned)
    })
}

impl Extractor for Streaks {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some((project, url_key, media_id)) = captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api_key = url_key.or_else(|| {
            crate::manifest_args::configuration_arg("streaks", "api_key", false)
                .and_then(|values| values.into_iter().next())
        });
        let headers: Vec<(&str, String)> = api_key
            .iter()
            .map(|key| ("X-Streaks-Api-Key", key.clone()))
            .collect();
        let borrowed: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        let response = playback_json(client, &project, &media_id, &borrowed)?;
        self.info(client, url, &project, &media_id, &response)
            .map(Some)
    }
}

impl Streaks {
    fn info(
        &self,
        client: &Client,
        url: &Url,
        project: &str,
        media_id: &str,
        response: &Value,
    ) -> Result<VideoInfo> {
        let streaks_id = response
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Streaks responses without a string id are not supported natively".into(),
                )
            })?;
        let live_status = match response.get("type").and_then(Value::as_str) {
            Some("clip") => Some("was_live"),
            Some("file") => Some("not_live"),
            Some("linear" | "live") => Some("is_live"),
            _ => None,
        };
        if live_status == Some("is_live") {
            return Err(Error::UnsupportedHtml5(
                "Streaks live streams (ssai sessions) are not supported natively".into(),
            ));
        }
        let sources = response
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        let mut drm = false;
        for source in &sources {
            let Some(src) = source.get("src").and_then(Value::as_str) else {
                continue;
            };
            if Url::parse(src).is_err() {
                continue;
            }
            if source
                .get("key_systems")
                .is_some_and(|keys| !keys.is_null())
            {
                drm = true;
                continue;
            }
            // `mimetype2ext(type) != 'm3u8'`: only HLS sources are read.
            let mime = source
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !mime.to_ascii_lowercase().contains("mpegurl") {
                let ext = mime.rsplit('/').next().unwrap_or_default();
                crate::screen::warning(&format!("[{NAME}] Unsupported stream type: {ext}"));
                continue;
            }
            let manifest = Url::parse(src)?;
            if let Some(found) = extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                media_id,
                Some("hls"),
                true,
                false,
            ) {
                formats.extend(found.formats);
                for (lang, tracks) in found.subtitles.entries {
                    subtitles.entries.push((lang, tracks));
                }
            }
        }
        if formats.is_empty() && drm {
            return Err(Error::Reported(format!(
                "[{NAME}] {media_id}: This video is DRM protected"
            )));
        }
        for track in response
            .get("tracks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let kind = track.get("kind").and_then(Value::as_str);
            let src = track.get("src").and_then(Value::as_str);
            let (Some("captions" | "subtitles"), Some(src)) = (kind, src) else {
                continue;
            };
            let Ok(src_url) = Url::parse(src) else {
                continue;
            };
            let lang = track
                .get("srclang")
                .and_then(Value::as_str)
                .map_or_else(|| "ja".to_owned(), str::to_lowercase);
            let entry = SubtitleFormat {
                name: None,
                ext: super::direct::determine_ext(src).unwrap_or_default(),
                url: src_url,
                protocol: None,
                fragments: Vec::new(),
                extra: Map::new(),
            };
            subtitles.entries.push((Some(lang), vec![entry]));
        }
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", streaks_id.clone().into()),
            ("display_id", media_id.into()),
            ("formats", Value::Null),
            ("live_status", live_status.map_or(Value::Null, Value::from)),
            ("subtitles", Value::Null),
            ("uploader_id", project.into()),
        ];
        let title = text(response.get("name"));
        if let Some(title) = title {
            fields.push(("title", title.into()));
        }
        if let Some(description) = text(response.get("description")) {
            fields.push(("description", description.into()));
        }
        if let Some(duration) = response.get("duration").and_then(Value::as_f64) {
            fields.push(("duration", duration.into()));
        }
        if let Some(stamp) = response
            .get("updated_at")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.push(("modified_timestamp", stamp.into()));
        }
        let tags: Vec<Value> = response
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| text(Some(tag)).map(Value::from))
            .collect();
        if !tags.is_empty() {
            fields.push(("tags", tags.into()));
        }
        if let Some(thumbnail) = thumbnail(response) {
            fields.push(("thumbnail", thumbnail.into()));
        }
        if let Some(stamp) = response
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.push(("timestamp", stamp.into()));
        }
        let mut info = super::dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(info)
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    captures(url).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        let player = "https://players.streaks.jp/tipness/08155cd19dc14c12bebefb69b92eafcc/index.html?m=ref:clip";
        let playback = "https://playback.api.streaks.jp/v1/projects/tbs/medias/ref:simul-02";
        assert_eq!(
            captures(player).map(|(project, _, id)| (project, id)),
            Some(("tipness".into(), "ref:clip".into()))
        );
        assert_eq!(
            captures(playback).map(|(_, _, id)| id),
            Some("ref:simul-02".into())
        );
        assert!(captures("https://example.com/video").is_none());
    }
}
