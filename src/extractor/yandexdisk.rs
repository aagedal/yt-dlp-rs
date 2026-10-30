//! Upstream `YandexDiskIE` (`yt_dlp/extractor/yandexdisk.py`): a public Yandex
//! Disk file (`yadi.sk`, `disk.yandex.*` and `disk.360.yandex.*` hosts). The
//! page's `store-prefetch` JSON names the file, its size and owner; the
//! public download API gives the source URL and, when the page's session
//! key is present, the HLS video streams are asked for through the page's
//! API.
use super::{
    Extractor, dict_format, dict_info, download_webpage, extract_m3u8_master_optional,
    post_json_errnote_optional,
};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct YandexDisk;

const NAME: &str = "YandexDisk";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?P<domain>yadi\.sk|disk\.(?:360\.)?yandex\.(?:az|by|co(?:m(?:\.(?:am|ge|tr))?|\.il)|ee|fr|k[gz]|l[tv]|md|t[jm]|u[az]|ru))/(?:[di]/|public.*?\bhash=)(?P<id>[^/?#&]+)",
    )
    .unwrap()
});
static STORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<script[^>]+id="store-prefetch"[^>]*>\s*(\{.+?\})\s*</script>"#).unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id` of a public URL (its short `yadi.sk` form names the file).
fn match_id(text: &str) -> Option<String> {
    VALID_URL
        .captures(text)
        .and_then(|found| found.name("id"))
        .map(|id| id.as_str().to_owned())
}

/// `resource.get(key) or {}` style lookups: a missing key or a falsy value.
fn object(value: Option<&Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    }
}

/// `_download_json(..., fatal=False) or {}`: a request error warns.
fn download_json_optional(client: &Client, url: &Url, id: &str) -> Result<Option<Value>> {
    match super::download_json(client, url, NAME, id) {
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

/// `KeyError(key)` of the page's store, reported like upstream.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for YandexDisk {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let domain = found["domain"].to_owned();
        let mut video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let Some(store) = STORE.captures(&webpage).and_then(|found| found.get(1)) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract store; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let store: Value = serde_json::from_str(store.as_str())
            .map_err(|_| Error::Reported(format!("[{NAME}] {video_id}: Failed to parse JSON")))?;

        let root_id = store
            .get("rootResourceId")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&video_id, "rootResourceId"))?;
        let resource = store
            .get("resources")
            .and_then(|resources| resources.get(root_id))
            .ok_or_else(|| key_error(&video_id, root_id))?;
        let title = resource
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&video_id, "name"))?
            .to_owned();
        let meta = object(resource.get("meta"));

        if let Some(id) = meta
            .get("short_url")
            .and_then(Value::as_str)
            .filter(|short| !short.is_empty())
            .and_then(match_id)
        {
            video_id = id;
        }

        let public_key = Url::parse_with_params(
            "https://cloud-api.yandex.net/v1/disk/public/resources/download",
            &[("public_key", url.as_str())],
        )?;
        let source_url = download_json_optional(client, &public_key, &video_id)?
            .as_ref()
            .and_then(|value| value.get("href"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut video_streams = object(resource.get("videoStreams"));
        let video_hash = resource
            .get("hash")
            .and_then(Value::as_str)
            .filter(|hash| !hash.is_empty())
            .unwrap_or(url.as_str())
            .to_owned();
        let environment = object(store.get("environment"));
        let session_key = environment
            .get("sk")
            .and_then(Value::as_str)
            .filter(|key| !key.is_empty());
        let yandexuid = environment
            .get("yandexuid")
            .and_then(Value::as_str)
            .filter(|uid| !uid.is_empty());
        let mut source_url = source_url;

        if let (Some(sk), Some(yandexuid)) = (session_key, yandexuid)
            && !(source_url.is_some() && !video_streams.is_empty())
        {
            crate::cookies::set_cookie(&domain, "yandexuid", yandexuid);
            let api = |action: &str| -> Result<Map<String, Value>> {
                let endpoint = url.join("/public/api/")?.join(action)?;
                let body = json!({ "hash": video_hash, "sk": sk }).to_string();
                let value = post_json_errnote_optional(
                    client,
                    &endpoint,
                    (NAME, &video_id),
                    (
                        "Downloading JSON metadata",
                        "Unable to download JSON metadata",
                    ),
                    &[("Content-Type", "text/plain")],
                    body,
                )?;
                Ok(object(value.as_ref().and_then(|value| value.get("data"))))
            };
            if source_url.is_none() {
                source_url = api("download-url")?
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            if video_streams.is_empty() {
                video_streams = api("get-video-streams")?;
            }
        }

        let mut formats: Vec<VideoFormat> = Vec::new();
        if let Some(source_url) = source_url {
            let fallback = meta
                .get("ext")
                .and_then(Value::as_str)
                .filter(|ext| !ext.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    meta.get("mime_type")
                        .and_then(Value::as_str)
                        .map(|mime| crate::extractor::direct::mimetype2ext_or(mime, None))
                        .filter(|ext| !ext.is_empty())
                })
                .unwrap_or_else(|| "mp4".to_owned());
            let ext = crate::extractor::direct::determine_ext(&title).unwrap_or(fallback);
            let filesize = meta.get("size").and_then(Value::as_i64);
            formats.push(dict_format(
                vec![
                    ("url", source_url.into()),
                    ("format_id", "source".into()),
                    ("ext", ext.into()),
                    ("quality", 1.into()),
                    ("filesize", filesize.map_or(Value::Null, Value::from)),
                ],
                NAME,
                &video_id,
            )?);
        }

        for video in video_streams
            .get("videos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(format_url) = video.get("url").and_then(Value::as_str) else {
                continue;
            };
            if video.get("dimension").and_then(Value::as_str) == Some("adaptive") {
                let manifest = Url::parse(format_url)?;
                if let Some(stream) = extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("hls"),
                    false,
                    false,
                ) {
                    for mut format in stream.formats {
                        format.ext = "mp4".into();
                        formats.push(format);
                    }
                }
            } else {
                let size = object(video.get("size"));
                let height = size.get("height").and_then(Value::as_i64);
                let format_id = match height {
                    Some(height) => format!("hls-{height}p"),
                    None => "hls".to_owned(),
                };
                formats.push(dict_format(
                    vec![
                        ("ext", "mp4".into()),
                        ("format_id", format_id.into()),
                        ("height", height.map_or(Value::Null, Value::from)),
                        ("protocol", "m3u8_native".into()),
                        ("url", format_url.into()),
                        (
                            "width",
                            size.get("width")
                                .and_then(Value::as_i64)
                                .map_or(Value::Null, Value::from),
                        ),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }

        let uid = resource.get("uid").cloned().unwrap_or(Value::Null);
        let display_name = uid
            .as_str()
            .and_then(|uid| store.get("users")?.get(uid)?.get("displayName"))
            .cloned()
            .unwrap_or(Value::Null);
        let duration = video_streams
            .get("duration")
            .and_then(Value::as_f64)
            .map(|milliseconds| milliseconds / 1000.0);
        let view_count = meta.get("views_counter").and_then(Value::as_i64);

        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("duration", duration.map_or(Value::Null, Value::from)),
                ("uploader", display_name),
                ("uploader_id", uid),
                ("view_count", view_count.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://yadi.sk/i/VdOeDou8eZs6Y"));
        assert!(suitable("https://yadi.sk/d/h3WAXvDS3Li3Ce"));
        assert!(suitable(
            "https://yadi.sk/public?hash=5DZ296JK9GWCLp02f6jrObjnctjRxMs8L6%2B%2FuhNqk38%3D"
        ));
        assert!(suitable("https://disk.360.yandex.ru/i/TM2xsIVsgjY4uw"));
        assert!(suitable("https://disk.yandex.com.tr/i/abc"));
        assert!(!suitable("https://disk.yandex.ru/"));
    }

    #[test]
    fn short_urls_name_the_file() {
        assert_eq!(
            match_id("https://yadi.sk/i/VdOeDou8eZs6Y").as_deref(),
            Some("VdOeDou8eZs6Y")
        );
    }
}
