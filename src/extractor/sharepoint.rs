//! Upstream `SharePointIE` (`yt_dlp/extractor/sharepoint.py`): SharePoint
//! video shares and `stream.aspx` links from the page's `g_fileInfo`: the
//! `videomanifest` DASH and HLS renditions next to its transform URL, and
//! the source download.
use super::{Extractor, dict_format, dict_info};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct SharePoint;

const NAME: &str = "SharePoint";

static VALID_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        crate::extractor::url_regex(
            r"^https?://[\w-]+\.sharepoint\.com/:v:/[a-z]/(?:[^/?#]+/)*(?P<id>[^/?#]{46})/?(?:$|[?#])",
        )
        .unwrap(),
        crate::extractor::url_regex(r"^https?://[\w-]+\.sharepoint\.com/(?:[^/?#]+/)*stream\.aspx\?(?:[^#]+&)?id=(?P<id>[^&#]+)")
            .unwrap(),
    ]
});
static FILE_INFO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"g_fileInfo\s*=").unwrap());

/// The `_VALID_URL` list's ID (`(?!:v:)` before the second pattern).
fn match_id(url: &str) -> Option<String> {
    if let Some(found) = VALID_URLS[0].captures(url) {
        return Some(found["id"].to_owned());
    }
    let found = VALID_URLS[1].captures(url)?;
    let after_base = url.splitn(4, '/').nth(3).unwrap_or_default();
    if after_base.starts_with(":v:") {
        return None;
    }
    Some(found["id"].to_owned())
}

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    match_id(url).is_some()
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("SharePoint {what} are not supported natively"))
}

/// The transform URL's sibling `videomanifest` with its query (`parse_qs`
/// values kept, then `cTag`, `action` and `part`).
fn manifest_base(transform: &str, ctag: &str) -> Result<Url> {
    let mut url =
        Url::parse(transform).map_err(|_| unsupported("transform URLs that are not URLs"))?;
    // `urljoin(f'{path}/', '../videomanifest')`.
    let path = url.path().to_owned();
    let parent = match path.trim_end_matches('/').rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/videomanifest"),
        None => "/videomanifest".to_owned(),
    };
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    for (key, value) in [("cTag", ctag), ("action", "Access"), ("part", "index")] {
        pairs.retain(|(existing, _)| existing != key);
        pairs.push((key.to_owned(), value.to_owned()));
    }
    url.set_path(&parent);
    url.set_query(None);
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in &pairs {
            query.append_pair(key, value);
        }
    }
    Ok(url)
}

impl Extractor for SharePoint {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(raw_id) = match_id(url.as_str()) else {
            return Ok(None);
        };
        let display_id = percent_encoding::percent_decode_str(&raw_id)
            .decode_utf8_lossy()
            .into_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let (webpage, final_url) = super::download_webpage_handle(client, url, NAME, &display_id)?;
        if final_url.host_str() == Some("login.microsoftonline.com") {
            return Err(Error::Reported(format!(
                "[{NAME}] {raw_id}: Session cookies are required for this URL and can be passed with the --cookies option. The --cookies-from-browser option will not work"
            )));
        }
        let video_data = match super::search_json(&FILE_INFO, &webpage, false) {
            Some(Some(data)) => data,
            Some(None) => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract player config - Failed to parse JSON"
                )));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {raw_id}: Unable to extract player config; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let video_id = match video_data.get("VroomItemId") {
            Some(Value::String(id)) => id.clone(),
            Some(_) => return Err(unsupported("item IDs that are not strings")),
            None => return Err(key_error(&raw_id, "VroomItemId")),
        };
        let transform = video_data
            .get(".transformUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&raw_id, ".transformUrl"))?;
        let ctag = video_data
            .get(".ctag")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&raw_id, ".ctag"))?;
        let base = manifest_base(transform, ctag)?;
        let mut formats = Vec::new();
        let dash = super::update_url_query(&base, &[("format", "dash")]);
        if let Some(mut stream) = super::extract_mpd_optional_note(
            client,
            &dash,
            NAME,
            &video_id,
            Some("dash"),
            "Downloading MPD manifest",
        ) {
            super::ignore_subtitles(&mut stream, NAME, "DASH");
            formats.extend(stream.formats);
        }
        for hls_type in ["hls", "hls-vnext"] {
            let manifest = super::update_url_query(&base, &[("format", hls_type)]);
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &video_id,
                Some(hls_type),
                false,
                false,
            ) {
                for mut format in stream.formats {
                    format.quality = Some(-2.0);
                    crate::info_order::mark_int_quality(&mut format);
                    formats.push(format);
                }
            }
        }
        if let Some(video_url) = video_data
            .get("downloadUrl")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            let name = video_data
                .get("extension")
                .filter(|value| super::nuxt::truthy(value))
                .or_else(|| video_data.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let ext = super::direct::determine_ext(name).unwrap_or_else(|| "unknown_video".into());
            let filesize = match video_data.get("size") {
                Some(Value::Number(number)) => number.as_i64().map(Value::from),
                Some(Value::String(text)) => text.trim().parse::<i64>().ok().map(Value::from),
                _ => None,
            };
            formats.push(dict_format(
                vec![
                    ("url", video_url.into()),
                    ("ext", ext.into()),
                    ("quality", 1.into()),
                    ("format_id", "source".into()),
                    ("filesize", filesize.unwrap_or(Value::Null)),
                    (
                        "vcodec",
                        if video_data.get("isAudio") == Some(&Value::Bool(true)) {
                            "none".into()
                        } else {
                            Value::Null
                        },
                    ),
                ],
                NAME,
                &video_id,
            )?);
        }
        let title = video_data
            .get("title")
            .filter(|value| super::nuxt::truthy(value))
            .or_else(|| video_data.get("displayName"))
            .cloned()
            .unwrap_or(Value::Null);
        let duration = video_data
            .get("MediaServiceFastMetadata")
            .and_then(Value::as_str)
            .and_then(|text| crate::json::from_str::<Value>(text).ok())
            .and_then(|metadata| metadata.pointer("/media/duration").and_then(Value::as_f64))
            .map_or(Value::Null, |duration| json!(duration / 10_000_000.0));
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("formats", Value::Null),
                ("title", title),
                ("display_id", display_id.into()),
                (
                    "uploader_id",
                    video_data.get("authorId").cloned().unwrap_or(Value::Null),
                ),
                ("duration", duration),
                (
                    "thumbnail",
                    video_data
                        .get("thumbnailUrl")
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                        .map_or(Value::Null, Value::from),
                ),
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
        assert!(suitable(
            "https://lut-my.sharepoint.com/:v:/g/personal/juha_eerola_student_lab_fi/EUrAmrktb4ZMhUcY9J2PqMEBD_9x_l0DyYWVgAvp-TTOMw?e=ZpQOOw"
        ));
        assert!(suitable(
            "https://acme.sharepoint.com/sites/x/_layouts/15/stream.aspx?id=%2Fsites%2Fx%2Fv.mp4&ga=1"
        ));
        assert!(!suitable(
            "https://acme.sharepoint.com/:v:/stream.aspx?id=x"
        ));
    }

    #[test]
    fn manifests_sit_next_to_the_transform_url() {
        let base = manifest_base(
            "https://acme.sharepoint.com/_api/v2.0/drives/b/items/i/driveItem/transform?provider=spo&v=",
            "\"c:{abc},2\"",
        )
        .unwrap();
        assert_eq!(
            base.as_str(),
            "https://acme.sharepoint.com/_api/v2.0/drives/b/items/i/driveItem/videomanifest?provider=spo&cTag=%22c%3A%7Babc%7D%2C2%22&action=Access&part=index"
        );
    }
}
