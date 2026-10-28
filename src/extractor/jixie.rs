//! Upstream `JixieBaseIE` (`yt_dlp/extractor/jixie.py`): a Jixie video's
//! public stream API data as HLS (DRM-flagged when the video is) and MP4
//! formats with the video's metadata, falling back to the page's meta tags.
use super::{dict_format, dict_info, download_json, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use url::Url;

/// Errors carry the temporary ID (the URL's video ID).
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Jixie {what} are not supported natively"))
}

/// `_html_search_meta(names, webpage)`: the first name present, else a
/// warning naming the first.
fn search_meta(name: &str, webpage: &str, names: &[&str]) -> Value {
    match names
        .iter()
        .find_map(|meta| crate::html::html_search_meta(webpage, meta))
    {
        Some(value) => value.into(),
        None => {
            super::search_warning(name, names[0]);
            Value::Null
        }
    }
}

/// `try_call(lambda: (value or None).split(','))`.
fn split(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) if !text.is_empty() => {
            Value::Array(text.split(',').map(Value::from).collect())
        }
        _ => Value::Null,
    }
}

/// `_extract_data_from_jixie_id(display_id, video_id, webpage)`.
pub(crate) fn extract_from_jixie_id(
    client: &Client,
    page: &Url,
    (name, key): (&'static str, &'static str),
    display_id: &str,
    video_id: &str,
    webpage: &str,
) -> Result<VideoInfo> {
    let api = super::update_url_query(
        &Url::parse("https://apidam.jixie.io/api/public/stream")?,
        &[("metadata", "full"), ("video_id", video_id)],
    );
    let response = download_json(client, &api, name, display_id)?;
    let data = response
        .get("data")
        .ok_or_else(|| key_error(video_id, "data"))?;
    let streams = match data.get("streams") {
        Some(Value::Array(streams)) => streams,
        Some(_) => return Err(unsupported("stream lists that are not lists")),
        None => return Err(key_error(video_id, "streams")),
    };
    let drm = data.get("drm").is_some_and(super::nuxt::truthy);
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for stream in streams {
        let stream_url = stream.get("url").and_then(Value::as_str);
        if stream.get("type").and_then(Value::as_str) == Some("HLS") {
            let manifest = stream_url
                .and_then(|url| Url::parse(url).ok())
                .ok_or_else(|| unsupported("HLS streams without a URL"))?;
            let stream =
                extract_m3u8_fatal_master(client, &manifest, name, display_id, None, true)?;
            for mut format in stream.formats {
                if drm {
                    format.extra.insert("has_drm".into(), Value::Bool(true));
                    crate::info_order::mark_format_update(&mut format, "has_drm");
                }
                formats.push(format);
            }
            super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
        } else {
            let stream_url = stream_url.ok_or_else(|| unsupported("streams without a URL"))?;
            let get = |key: &str| stream.get(key).cloned().unwrap_or(Value::Null);
            formats.push(dict_format(
                vec![
                    ("url", stream_url.into()),
                    ("width", get("width")),
                    ("height", get("height")),
                    ("ext", "mp4".into()),
                ],
                name,
                display_id,
            )?);
        }
    }
    let metadata = data.get("metadata");
    let meta = |key: &str| metadata.and_then(|metadata| metadata.get(key));
    let title = data
        .get("title")
        .filter(|title| super::nuxt::truthy(title))
        .cloned()
        .unwrap_or_else(|| search_meta(name, webpage, &["og:title", "twitter:title"]));
    let description = meta("description")
        .and_then(Value::as_str)
        .map(crate::html::clean_html)
        .filter(|text| !text.is_empty())
        .map_or_else(
            || {
                search_meta(
                    name,
                    webpage,
                    &["description", "og:description", "twitter:description"],
                )
            },
            Value::from,
        );
    let duration = match meta("duration") {
        Some(Value::Number(number)) => number.as_f64().map(|value| json!(value)),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok().map(|value| json!(value)),
        _ => None,
    }
    .unwrap_or(Value::Null);
    let mut info = dict_info(
        page,
        (name, key),
        vec![
            ("id", video_id.into()),
            ("display_id", display_id.into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("title", title),
            ("description", description),
            (
                "thumbnails",
                meta("thumbnails").cloned().unwrap_or(Value::Null),
            ),
            ("duration", duration),
            ("tags", split(meta("keywords"))),
            ("categories", split(meta("categories"))),
            (
                "uploader_id",
                data.get("owner_id").cloned().unwrap_or(Value::Null),
            ),
        ],
        formats,
    );
    info.subtitles = subtitles;
    // The API's thumbnail dicts keep their key order.
    let mut thumbnail_keys: Vec<&str> = Vec::new();
    if let Some(Value::Array(thumbnails)) = meta("thumbnails") {
        for key in thumbnails
            .iter()
            .filter_map(Value::as_object)
            .flat_map(|map| map.keys())
        {
            if !thumbnail_keys.contains(&key.as_str()) {
                thumbnail_keys.push(key);
            }
        }
    }
    if !thumbnail_keys.is_empty()
        && let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE)
    {
        shape.push_str(&format!(";thumbnail={}", thumbnail_keys.join(",")));
    }
    Ok(info)
}
