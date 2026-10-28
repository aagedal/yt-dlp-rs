//! Upstream `SkebIE` (`yt_dlp/extractor/skeb.py`): Skeb works as playlists
//! of their mp3/mp4/wav previews from the works API (retried with the
//! `request_key` cookie a 429 page sets), with the work's metadata.
use super::{Extractor, dict_info};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Skeb;

const NAME: &str = "Skeb";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://skeb\.jp/@(?P<uploader_id>[^/?#]+)/works/(?P<id>\d+)").unwrap()
});
static REQUEST_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"document\.cookie\s*=\s*["']request_key=([^;"']+)"#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

const HEADERS: &[(&str, &str)] = &[
    ("Accept", "application/json"),
    ("Authorization", "Bearer null"),
];

/// `_call_api`: the parsed works JSON, or a 429 response's page.
fn call_api(
    client: &Client,
    api: &Url,
    work_id: &str,
) -> Result<std::result::Result<Value, String>> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {work_id}: Downloading JSON metadata"));
    let mut response = super::direct::request_expecting(client, api, HEADERS, &[429]).map_err(
        |error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: NAME,
                id: work_id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            },
            error => error,
        },
    )?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    if status == 429 {
        return Ok(Err(String::from_utf8_lossy(&body).into_owned()));
    }
    let text = super::decode_webpage(&body, response.headers());
    crate::json::from_str(&text).map(Ok).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {work_id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// The work's `info` dict, in its key order.
fn work_info(works: &Value, uploader_id: &str) -> Vec<(&'static str, Value)> {
    let mut info = vec![("uploader_id", Value::from(uploader_id))];
    if works.get("nsfw") == Some(&Value::Bool(true)) {
        info.push(("age_limit", 18.into()));
    }
    let description = ["source_body", "body"].iter().find_map(|key| {
        let text = works.get(*key)?.as_str()?;
        let text = crate::html::clean_html(text);
        (!text.is_empty()).then_some(text)
    });
    if let Some(description) = description {
        info.push(("description", description.into()));
    }
    if let Some(genre) = works
        .get("genre")
        .and_then(Value::as_str)
        .filter(|genre| !genre.is_empty())
    {
        info.push(("genres", json!([genre])));
    }
    let tags: Vec<Value> = match works.get("tag_list") {
        Some(Value::Array(tags)) => tags
            .iter()
            .filter(|tag| tag.as_str().is_some_and(|tag| !tag.is_empty()))
            .cloned()
            .collect(),
        Some(Value::Object(tags)) => tags
            .values()
            .filter(|tag| tag.as_str().is_some_and(|tag| !tag.is_empty()))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    if !tags.is_empty() {
        info.push(("tags", Value::Array(tags)));
    }
    if let Some(uploader) = works
        .pointer("/creator/name")
        .filter(|name| name.is_string())
    {
        info.push(("uploader", uploader.clone()));
    }
    info
}

fn preview_entry(
    page: &Url,
    preview: &Value,
    ext: &str,
    title: String,
    info: &[(&'static str, Value)],
) -> Result<VideoInfo> {
    let media = url_or_none(preview.get("url")).unwrap_or_default();
    let vtt = url_or_none(preview.get("vtt_url"));
    let mut fields: Vec<(&str, Value)> = vec![
        ("ext", ext.into()),
        ("title", title.into()),
        ("subtitles", Value::Null),
        (
            "vcodec",
            if matches!(ext, "mp3" | "wav") {
                "none".into()
            } else {
                Value::Null
            },
        ),
    ];
    fields.extend(info.iter().cloned());
    let optional = [
        ("id", str_or_none(preview.get("id")).map(Value::from)),
        (
            "thumbnail",
            url_or_none(preview.get("poster_url")).map(Value::from),
        ),
        ("url", Some(Value::from(media.clone()))),
    ];
    fields.extend(
        optional
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value))),
    );
    let information = preview.get("information").filter(|value| value.is_object());
    let mut format = json!({"format_id": "", "url": media, "ext": ext});
    if let Some(information) = information {
        for (key, source) in [
            ("duration", "duration"),
            ("fps", "frame_rate"),
            ("height", "height"),
            ("width", "width"),
        ] {
            if let Some(value) = int_or_none(information.get(source)) {
                fields.push((key, value.into()));
                if key != "duration" {
                    format[key] = value.into();
                }
            }
        }
    }
    if ext != "mp4" {
        format["vcodec"] = "none".into();
    }
    let mut format: crate::VideoFormat = serde_json::from_value(format)
        .map_err(|error| Error::Reported(format!("[{NAME}] {error}")))?;
    crate::info_order::mark_format(&mut format, "url-only");
    let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
    let shape = format!("untyped-dict;{};subtitle=ext,url", keys.join(","));
    let mut entry = dict_info(page, (NAME, NAME), fields, vec![format]);
    match vtt {
        // `'subtitles': None` keeps its key.
        None => {
            entry.extra.insert("subtitles".into(), Value::Null);
        }
        Some(vtt) => {
            entry.subtitles.insert(
                Some("ja".into()),
                vec![
                    serde_json::from_value(json!({"ext": "vtt", "url": vtt}))
                        .map_err(|error| Error::Reported(format!("[{NAME}] {error}")))?,
                ],
            );
        }
    }
    Ok(crate::info_order::mark_info(entry, &shape))
}

impl Extractor for Skeb {
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
        let uploader_id = found["uploader_id"].to_owned();
        let work_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://skeb.jp/api/users/{uploader_id}/works/{work_id}"
        ))?;
        let works = match call_api(client, &api, &work_id)? {
            Ok(works) => works,
            Err(webpage) => {
                let value = REQUEST_KEY
                    .captures(&webpage)
                    .map(|found| found[1].to_owned())
                    .ok_or_else(|| {
                        Error::Reported(format!(
                            "[{NAME}] {work_id}: Unable to extract request key; {}",
                            crate::BUG_REPORTS_MESSAGE
                        ))
                    })?;
                crate::cookies::set_cookie("skeb.jp", "request_key", &value);
                super::download_json_headers(client, &api, NAME, &work_id, HEADERS)?
            }
        };
        let info = work_info(&works, &uploader_id);
        let mut entries = Vec::new();
        if let Some(Value::Array(previews)) = works.get("previews") {
            let previews = previews
                .iter()
                .filter(|preview| url_or_none(preview.get("url")).is_some());
            for (index, preview) in previews.enumerate() {
                let ext = preview
                    .pointer("/information/extension")
                    .and_then(Value::as_str);
                let Some(ext) = ext.filter(|ext| matches!(*ext, "mp3" | "mp4" | "wav")) else {
                    crate::screen::warning(&format!(
                        "[{NAME}] Skipping unsupported extension \"{}\"",
                        ext.unwrap_or("None")
                    ));
                    continue;
                };
                let title = format!("{work_id}-{}", index + 1);
                entries.push(preview_entry(url, preview, ext, title, &info)?);
            }
        }
        let mut extra = Map::new();
        // `playlist_result(entries, work_id, **info)`: the info keys first.
        let mut keys = Vec::new();
        for (key, value) in info {
            keys.push(key);
            extra.insert(key.into(), value);
        }
        keys.push("id");
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: work_id,
            title: String::new(),
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
        assert!(suitable("https://skeb.jp/@riiru_wm/works/10"));
        assert!(!suitable("https://skeb.jp/@riiru_wm"));
    }
}
