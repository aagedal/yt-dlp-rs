//! Upstream `FilmOnIE` and `FilmOnChannelIE` (`yt_dlp/extractor/filmon.py`):
//! FilmOn VOD movies (their streams and poster thumbnails; series as
//! `filmon:ID` episode playlists) and channels (live streams, or Wowza
//! formats for VOD channels), with the APIs' error messages.
use super::{Extractor, dict_format, dict_info};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct FilmOn;
pub struct FilmOnChannel;

const NAME: &str = "filmon";
const KEY: &str = "FilmOn";
const CHANNEL_NAME: &str = "filmon:channel";
const CHANNEL_KEY: &str = "FilmOnChannel";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:https?://(?:www\.)?filmon\.com/vod/view/|filmon:)(?P<id>\d+)").unwrap()
});
static CHANNEL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?filmon\.com/(?:tv|channel)/(?P<id>[a-z0-9-]+)").unwrap()
});

/// `FilmOnIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `FilmOnChannelIE._match_valid_url`.
pub fn channel_suitable(url: &str) -> bool {
    CHANNEL_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("FilmOn {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `str(value)` of a JSON value in an f-string.
fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(number) => match number.as_i64() {
            Some(number) => number.to_string(),
            None => crate::screen::py_float(number.as_f64().unwrap_or_default()),
        },
        _ => value.to_string(),
    }
}

/// `_download_json(url, id)[key]`; an HTTP error's JSON body gives
/// `{IE_NAME} said: {body[message_key]}` (expected).
fn api(
    client: &Client,
    name: &'static str,
    url: &Url,
    id: &str,
    key: &str,
    message_key: &str,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{name}] {id}: Downloading JSON metadata"));
    let statuses: Vec<u16> = (400..600).collect();
    let mut response = super::direct::request_expecting(client, url, &[], &statuses)?;
    let status = response.status();
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    if status.is_client_error() || status.is_server_error() {
        let error: Value = crate::json::from_str(&text).map_err(|error| {
            Error::Reported(format!(
                "[{name}] {id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let message = error
            .get(message_key)
            .ok_or_else(|| key_error(id, message_key))?;
        return Err(Error::Reported(format!(
            "[{name}] {id}: {name} said: {}",
            py_str(message)
        )));
    }
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let data: Value = crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{name}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })?;
    data.get(key).cloned().ok_or_else(|| key_error(id, key))
}

/// `url_result(url)`.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": null,
            "entry": {"_type": "url", "url": target},
        }),
    );
    Ok(info)
}

/// The thumbnails' `{'id', 'url', 'width', 'height'}` key order (`None`
/// sizes kept).
fn thumbnail_order(mut info: VideoInfo) -> VideoInfo {
    if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
        shape.push_str(";thumbnail!=id,url,width,height");
    }
    info
}

/// `qualities(('low', 'high'))(quality)`.
fn quality(value: Option<&Value>) -> i64 {
    match value.and_then(Value::as_str) {
        Some("low") => 0,
        Some("high") => 1,
        _ => -1,
    }
}

/// `int_or_none`.
fn int(value: Option<&Value>) -> Value {
    super::streamable::int_or_none(value)
}

/// `dict.get(key, default)` of an object (`None` when it is not one).
fn object<'a>(value: Option<&'a Value>, what: &str) -> Result<Option<&'a Map<String, Value>>> {
    match value {
        None => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(_) => Err(unsupported(what)),
    }
}

impl Extractor for FilmOn {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);
        let response = api(
            client,
            NAME,
            &Url::parse(&format!(
                "https://www.filmon.com/api/vod/movie?id={video_id}"
            ))?,
            &video_id,
            "response",
            "reason",
        )?;
        let Some(response) = response.as_object() else {
            return Err(unsupported("responses that are not objects"));
        };
        let title = response
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "title"))?;
        let description = match response.get("description") {
            Some(Value::String(text)) => Value::from(crate::batch::py_strip(text)),
            _ => Value::Null,
        };

        if response.get("type_id").and_then(Value::as_i64) == Some(1) {
            let episodes = match response.get("episodes") {
                None => Vec::new(),
                Some(Value::Array(episodes)) => episodes.clone(),
                Some(_) => return Err(unsupported("episode lists that are not lists")),
            };
            let mut entries = Vec::new();
            for episode_id in episodes {
                let Value::String(episode_id) = episode_id else {
                    return Err(Error::Reported(format!(
                        "can only concatenate str (not \"{}\") to str",
                        match episode_id {
                            Value::Null => "NoneType",
                            Value::Bool(_) => "bool",
                            Value::Number(number) if number.is_f64() => "float",
                            Value::Number(_) => "int",
                            Value::Array(_) => "list",
                            Value::Object(_) => "dict",
                            Value::String(_) => unreachable!(),
                        }
                    )));
                };
                entries.push(url_entry(&format!("filmon:{episode_id}"))?);
            }
            // `playlist_result(entries, video_id, title, description)`.
            let title = title.as_str().unwrap_or_default().to_owned();
            let mut keys = vec!["id"];
            if !title.is_empty() {
                keys.push("title");
            }
            let mut extra = Map::new();
            if !description.is_null() {
                keys.push("description");
                extra.insert("description".into(), description);
            }
            extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                format!("playlist-dict;{}", keys.join(",")).into(),
            );
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id: video_id,
                title,
                webpage_url: url.clone(),
                extractor: NAME.into(),
                extractor_key: KEY.into(),
                nested: Vec::new(),
                pending: None,
                entries,
                extra,
            })));
        }

        let mut formats = Vec::new();
        if let Some(streams) = object(response.get("streams"), "stream maps that are not objects")?
        {
            for (format_id, stream) in streams {
                let Some(stream) = stream.as_object() else {
                    return Err(unsupported("streams that are not objects"));
                };
                let Some(stream_url) = stream.get("url").filter(|url| super::nuxt::truthy(url))
                else {
                    continue;
                };
                formats.push(dict_format(
                    vec![
                        ("format_id", format_id.clone().into()),
                        ("url", stream_url.clone()),
                        ("ext", "mp4".into()),
                        ("quality", quality(stream.get("quality")).into()),
                        ("protocol", "m3u8_native".into()),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }

        // `thumbs['poster'] = poster`: the poster itself is the last thumbnail
        // (or replaces a `poster` thumb in its place).
        let poster = object(response.get("poster"), "posters that are not objects")?
            .cloned()
            .unwrap_or_default();
        let mut thumbs = object(poster.get("thumbs"), "thumbnail maps that are not objects")?
            .cloned()
            .unwrap_or_default();
        thumbs.insert("poster".into(), Value::Object(poster.clone()));
        let mut thumbnails = Vec::new();
        for (thumb_id, thumb) in &thumbs {
            let Some(thumb) = thumb.as_object() else {
                return Err(unsupported("thumbnails that are not objects"));
            };
            let Some(thumb_url) = thumb.get("url").filter(|url| super::nuxt::truthy(url)) else {
                continue;
            };
            thumbnails.push(json!({
                "id": thumb_id,
                "url": thumb_url,
                "width": int(thumb.get("width")),
                "height": int(thumb.get("height")),
            }));
        }
        Ok(Some(ExtractResult::Video(thumbnail_order(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("formats", Value::Null),
                ("description", description),
                ("thumbnails", Value::Array(thumbnails)),
            ],
            formats,
        )))))
    }
}

/// `FilmOnChannelIE._THUMBNAIL_RES`.
const THUMBNAIL_RES: [(&str, i64, i64); 3] = [
    ("logo", 56, 28),
    ("big_logo", 106, 106),
    ("extra_big_logo", 300, 300),
];

impl Extractor for FilmOnChannel {
    fn key(&self) -> &'static str {
        CHANNEL_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = CHANNEL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        start(CHANNEL_NAME, url);
        let data = api(
            client,
            CHANNEL_NAME,
            &Url::parse(&format!("http://www.filmon.com/api-v2/channel/{temp_id}"))?,
            &temp_id,
            "data",
            "message",
        )?;
        let Some(data) = data.as_object() else {
            return Err(unsupported("channel data that is not an object"));
        };
        let channel_id = py_str(data.get("id").ok_or_else(|| key_error(&temp_id, "id"))?);
        let truthy = |key: &str| data.get(key).is_some_and(super::nuxt::truthy);
        let is_live = !truthy("is_vod") && !truthy("is_vox");
        let title = data
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&temp_id, "title"))?;

        let streams = match data.get("streams") {
            None => Vec::new(),
            Some(Value::Array(streams)) => streams.clone(),
            Some(_) => return Err(unsupported("stream lists that are not lists")),
        };
        let mut formats = Vec::new();
        for stream in &streams {
            let Some(stream) = stream.as_object() else {
                return Err(unsupported("streams that are not objects"));
            };
            let Some(stream_url) = stream.get("url").filter(|url| super::nuxt::truthy(url)) else {
                continue;
            };
            if !is_live {
                let Some(stream_url) = stream_url.as_str() else {
                    return Err(unsupported("stream URLs that are not strings"));
                };
                formats.extend(super::extract_wowza_formats(
                    client,
                    stream_url,
                    CHANNEL_NAME,
                    &channel_id,
                    &["dash", "rtmp"],
                )?);
                continue;
            }
            // An m3u8 stream, deliberately not read as one: it has no
            // bitrate variants.
            formats.push(dict_format(
                vec![
                    (
                        "format_id",
                        stream.get("quality").cloned().unwrap_or(Value::Null),
                    ),
                    ("url", stream_url.clone()),
                    ("ext", "mp4".into()),
                    ("quality", quality(stream.get("quality")).into()),
                ],
                CHANNEL_NAME,
                &channel_id,
            )?);
        }
        let thumbnails: Vec<Value> = THUMBNAIL_RES
            .iter()
            .map(|(name, width, height)| {
                json!({
                    "id": name,
                    "url": format!("http://static.filmon.com/assets/channels/{channel_id}/{name}.png"),
                    "width": width,
                    "height": height,
                })
            })
            .collect();
        let get = |key: &str| data.get(key).cloned().unwrap_or(Value::Null);
        Ok(Some(thumbnail_order(dict_info(
            url,
            (CHANNEL_NAME, CHANNEL_KEY),
            vec![
                ("id", channel_id.into()),
                ("display_id", get("alias")),
                ("title", title),
                ("description", get("description")),
                ("thumbnails", Value::Array(thumbnails)),
                ("formats", Value::Null),
                ("is_live", is_live.into()),
            ],
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.filmon.com/vod/view/24869-0-plan-9-from-outer-space"
        ));
        assert!(suitable("filmon:2825"));
        assert!(channel_suitable("http://www.filmon.com/tv/sports-haters"));
        assert!(channel_suitable(
            "https://www.filmon.com/channel/filmon-sports"
        ));
        assert!(!channel_suitable("https://www.filmon.com/vod/view/1"));
    }
}
