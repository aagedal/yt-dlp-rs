//! Upstream `PicartoIE` and `PicartoVodIE` (`yt_dlp/extractor/picarto.py`):
//! Picarto live channels from the `ptvapi` GraphQL channel and its load
//! balancer's stream sources, and recorded videos from the API's recording
//! file as an HLS playlist.
use super::{Extractor, dict_format, dict_info, download_json_headers, download_json_note};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Picarto;
pub struct PicartoVod;

const NAME: &str = "picarto";
const KEY: &str = "Picarto";
const VOD_NAME: &str = "picarto:vod";
const VOD_KEY: &str = "PicartoVod";
const API: &str = "https://ptvintern.picarto.tv/ptvapi";
const HEADERS: [(&str, &str); 2] = [("Accept", "*/*"), ("Content-Type", "application/json")];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www.)?picarto\.tv/(?P<id>[^/#?]+)/?(?:$|[?#])")
        .unwrap()
});
static VOD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?picarto\.tv/(?:videopopout|\w+(?:/profile)?/videos)/(?P<id>[^/?#&]+)",
    )
    .unwrap()
});

/// `PicartoIE.suitable`: not video URLs.
pub fn suitable(url: &str) -> bool {
    !vod_suitable(url) && VALID_URL.is_match(url)
}

/// `PicartoVodIE._match_id`.
pub fn vod_suitable(url: &str) -> bool {
    VOD_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Picarto {what} are not supported natively"))
}

/// `traverse(value, keys)` that must exist (`KeyError` otherwise).
fn index<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| key_error(id, key))
}

fn ptvapi(client: &Client, name: &'static str, id: &str, query: &str) -> Result<Value> {
    let api = super::update_url_query(&Url::parse(API)?, &[("query", query)]);
    download_json_headers(client, &api, name, id, &HEADERS)
}

impl Extractor for Picarto {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if vod_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let channel_id = found["id"].to_owned();
        start(NAME, url);
        let query = format!(
            "{{\n  channel(name: \"{channel_id}\") {{\n    adult\n    id\n    online\n    stream_name\n    title\n  }}\n  getLoadBalancerUrl(channel_name: \"{channel_id}\") {{\n    url\n  }}\n}}"
        );
        let response = ptvapi(client, NAME, &channel_id, &query)?;
        let data = index(&response, "data", &channel_id)?;
        let metadata = index(data, "channel", &channel_id)?;
        // `metadata.get('online') == 0` (also true for `False`).
        if matches!(metadata.get("online"), Some(Value::Bool(false)))
            || metadata
                .get("online")
                .and_then(Value::as_f64)
                .is_some_and(|online| online == 0.0)
        {
            return Err(Error::Reported(format!(
                "[{NAME}] {channel_id}: Stream is offline"
            )));
        }
        let balancer = index(
            index(data, "getLoadBalancerUrl", &channel_id)?,
            "url",
            &channel_id,
        )?
        .as_str()
        .ok_or_else(|| unsupported("load balancer URLs that are not strings"))?;
        let stream_name = index(metadata, "stream_name", &channel_id)?
            .as_str()
            .ok_or_else(|| unsupported("stream names that are not strings"))?;
        // `update_url(url, scheme='https')`.
        let mut balancer = Url::parse(balancer)
            .map_err(|_| unsupported("load balancer URLs that are not URLs"))?;
        if balancer.set_scheme("https").is_err() {
            return Err(unsupported("load balancer URLs without an HTTP scheme"));
        }
        let cdn_url = format!(
            "{}/stream/json_{stream_name}.js",
            balancer.as_str().trim_end_matches('/')
        );
        let cdn_data = download_json_note(
            client,
            &Url::parse(&cdn_url)?,
            NAME,
            &channel_id,
            "Downloading load balancing info",
        )?;
        let mut formats = Vec::new();
        if let Some(Value::Array(sources)) = cdn_data.get("source") {
            for source in sources {
                let Some(source_url) = source
                    .get("url")
                    .filter(|value| super::nuxt::truthy(value))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                match source.get("type").and_then(Value::as_str) {
                    Some("html5/application/vnd.apple.mpegurl") => {
                        if let Some(stream) = super::extract_m3u8_master_optional(
                            client,
                            &Url::parse(source_url)?,
                            NAME,
                            &channel_id,
                            Some("hls"),
                            false,
                            false,
                        ) {
                            formats.extend(stream.formats);
                        }
                    }
                    Some("html5/video/mp4") => formats.push(dict_format(
                        vec![("url", source_url.into())],
                        NAME,
                        &channel_id,
                    )?),
                    _ => {}
                }
            }
        }
        let age_limit = match metadata.get("adult") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::Bool(true)) => 18.into(),
            Some(_) => 0.into(),
        };
        let title = match metadata.get("title") {
            Some(Value::String(title)) => title.trim().into(),
            _ => Value::Null,
        };
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", channel_id.clone().into()),
                ("title", title),
                ("is_live", true.into()),
                ("channel", channel_id.clone().into()),
                (
                    "channel_id",
                    metadata.get("id").cloned().unwrap_or(Value::Null),
                ),
                (
                    "channel_url",
                    format!("https://picarto.tv/{channel_id}").into(),
                ),
                ("age_limit", age_limit),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

impl Extractor for PicartoVod {
    fn key(&self) -> &'static str {
        VOD_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VOD_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(VOD_NAME, url);
        let query = format!(
            "{{\n  video(id: \"{video_id}\") {{\n    id\n    title\n    adult\n    file_name\n    video_recording_image_url\n    channel {{\n      name\n    }}\n  }}\n}}"
        );
        let response = ptvapi(client, VOD_NAME, &video_id, &query)?;
        let data = index(index(&response, "data", &video_id)?, "video", &video_id)?;
        let file_name = index(data, "file_name", &video_id)?
            .as_str()
            .ok_or_else(|| unsupported("file names that are not strings"))?;
        let image = index(data, "video_recording_image_url", &video_id)?
            .as_str()
            .ok_or_else(|| unsupported("recording images that are not strings"))?;
        let netloc = Url::parse(image)
            .ok()
            .and_then(|image| {
                image.host_str().map(|host| match image.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_owned(),
                })
            })
            .unwrap_or_default();
        let stream = super::extract_m3u8_fatal_master(
            client,
            &Url::parse(&format!(
                "https://{netloc}/stream/hls/{file_name}/index.m3u8"
            ))?,
            VOD_NAME,
            &video_id,
            Some("hls"),
            false,
        )?;
        let mut fields = vec![("id", Value::from(video_id.clone()))];
        match data.get("id") {
            Some(Value::String(id)) => fields[0].1 = id.clone().into(),
            Some(Value::Number(id)) => fields[0].1 = id.to_string().into(),
            _ => {}
        }
        if let Some(Value::String(title)) = data.get("title") {
            fields.push(("title", title.trim().into()));
        }
        if let Some(thumbnail) = data
            .get("video_recording_image_url")
            .filter(|value| !value.is_null())
        {
            fields.push(("thumbnail", thumbnail.clone()));
        }
        if let Some(Value::String(channel)) = data.pointer("/channel/name") {
            fields.push(("channel", channel.clone().into()));
        }
        if let Some(adult) = data.get("adult").filter(|value| !value.is_null()) {
            fields.push((
                "age_limit",
                if super::nuxt::truthy(adult) { 18 } else { 0 }.into(),
            ));
        }
        fields.push(("formats", Value::Null));
        Ok(Some(dict_info(
            url,
            (VOD_NAME, VOD_KEY),
            fields,
            stream.formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://picarto.tv/Setz"));
        assert!(!suitable(
            "https://picarto.tv/videopopout/ArtofZod_2017.12.12.00.13.23.flv"
        ));
        assert!(vod_suitable(
            "https://picarto.tv/videopopout/ArtofZod_2017.12.12.00.13.23.flv"
        ));
        assert!(vod_suitable("https://picarto.tv/ArtofZod/videos/771008"));
        assert!(vod_suitable(
            "https://picarto.tv/DrechuArt/profile/videos/400347"
        ));
    }
}
