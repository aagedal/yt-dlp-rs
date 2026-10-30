//! Upstream `NestIE` and `NestClipIE` (`yt_dlp/extractor/nest.py`): a public
//! camera's live HLS playlist from the dropcam camera API, and a clip's
//! `videos.get_by_filename` item as its untitled-by-default direct URL.
use super::{Extractor, direct, download_json, update_url_query};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Nest;
pub struct NestClip;

const NAME: &str = "Nest";
const CLIP_NAME: &str = "NestClip";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://video\.nest\.com/(?:embedded/)?live/(?P<id>\w+)")
        .unwrap()
});
static CLIP_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://video\.nest\.com/(?:embedded/)?clip/(?P<id>\w+)").unwrap()
});

/// `NestIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `NestClipIE._match_id`.
pub fn clip_suitable(url: &str) -> bool {
    CLIP_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// The first of `keys` holding a non-empty string (`{str}, filter, any`).
fn first_text(object: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .find(|text| !text.is_empty())
        .map(str::to_owned)
}

/// `(('timezone', {lambda x: x.split('/')[1].replace('_', ' ')}), 'where')`:
/// a timezone without a `/` is skipped (`traverse_obj` swallows the
/// `IndexError`).
fn location(item: &Value) -> Option<String> {
    item.get("timezone")
        .and_then(Value::as_str)
        .and_then(|zone| zone.split('/').nth(1))
        .map(|city| city.replace('_', " "))
        .filter(|city| !city.is_empty())
        .or_else(|| first_text(item, &["where"]))
}

impl Extractor for Nest {
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
        let video_id = found["id"].to_owned();
        announce(NAME, url);
        let api = update_url_query(
            &Url::parse("https://video.nest.com/api/dropcam/cameras.get_by_public_token")?,
            &[("token", video_id.as_str())],
        );
        let data = download_json(client, &api, NAME, &video_id)?;
        let item = match data.get("items") {
            Some(Value::Array(items)) => items
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "Nest camera items that are not lists are not supported natively".into(),
                ));
            }
            None => return Err(super::clyp::key_error(&video_id, "items")),
        };
        let uuid = item
            .get("uuid")
            .and_then(Value::as_str)
            .filter(|uuid| !uuid.is_empty());
        let stream_domain = item
            .get("live_stream_host")
            .and_then(Value::as_str)
            .filter(|host| !host.is_empty());
        let (Some(uuid), Some(stream_domain)) = (uuid, stream_domain) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to construct playlist URL; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let thumb_domain = item
            .get("nexus_api_nest_domain_host")
            .and_then(Value::as_str);
        let manifest = update_url_query(
            &Url::parse(&format!(
                "https://{stream_domain}/nexus_aac/{uuid}/playlist.m3u8"
            ))?,
            &[("public", video_id.as_str())],
        );
        let mut stream =
            super::extract_m3u8_fatal_live(client, &manifest, NAME, &video_id, None, false)?;
        for format in &mut stream.formats {
            format.ext = "mp4".into();
        }
        let mut fields: Vec<(&str, Value)> = vec![("id", video_id.clone().into())];
        if let Some(description) = item.get("description").and_then(Value::as_str) {
            fields.push(("description", description.into()));
        }
        if let Some(title) = first_text(item, &["title", "name", "where"]) {
            fields.push(("title", title.into()));
        }
        if let Some(alt_title) = first_text(item, &["name"]) {
            fields.push(("alt_title", alt_title.into()));
        }
        if let Some(location) = location(item) {
            fields.push(("location", location.into()));
        }
        let thumbnail = match thumb_domain {
            Some(domain) => Value::from(
                update_url_query(
                    &Url::parse(&format!("https://{domain}/get_image"))?,
                    &[("uuid", uuid), ("public", video_id.as_str())],
                )
                .to_string(),
            ),
            None => Value::Null,
        };
        fields.push(("thumbnail", thumbnail));
        // `_availability(is_private=is_public is False)`: only a private
        // camera is known, anything else is `None`.
        let private = item.get("is_public") == Some(&Value::Bool(false));
        fields.push((
            "availability",
            if private {
                "private".into()
            } else {
                Value::Null
            },
        ));
        fields.push(("formats", Value::Null));
        fields.push(("is_live", true.into()));
        let info = super::dict_info(url, (NAME, NAME), fields, stream.formats);
        Ok(Some(ExtractResult::Video(info)))
    }
}

impl Extractor for NestClip {
    fn key(&self) -> &'static str {
        CLIP_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = CLIP_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(CLIP_NAME, url);
        let api = update_url_query(
            &Url::parse("https://video.nest.com/api/dropcam/videos.get_by_filename")?,
            &[("filename", format!("{video_id}.mp4").as_str())],
        );
        let data = download_json(client, &api, CLIP_NAME, &video_id)?;
        // `traverse_obj(data, ('items', 0, {...}))`: missing parts are skipped.
        let item = data.get("items").and_then(|items| items.get(0));
        let text = |key: &str| item.and_then(|item| item.get(key)).and_then(Value::as_str);
        let title = text("title").map(str::to_owned);
        let thumbnail = text("thumbnail_url").and_then(super::json_ld::url_or_none);
        let media = text("download_url").and_then(super::json_ld::url_or_none);
        let timestamp = item
            .and_then(|item| item.get("start_time"))
            .map(|start| super::clyp::float_or_none(Some(start)))
            .filter(|value| !value.is_null());
        let mut keys = vec!["id"];
        let media = media.ok_or_else(|| {
            Error::UnsupportedHtml5(
                "Nest clips without a download URL are not supported natively".into(),
            )
        })?;
        let media = Url::parse(&media)?;
        let ext = direct::determine_ext(media.as_str())
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        let mut info =
            direct::with_format(direct::build_at(media.clone(), &media, ext.clone()), "");
        for format in &mut info.formats {
            format.ext = ext.clone();
        }
        info.direct = None;
        info.id = video_id.clone();
        if let Some(title) = title {
            info.title = title;
            keys.push("title");
        }
        if let Some(thumbnail) = thumbnail {
            info.metadata.thumbnail = Some(thumbnail);
            keys.push("thumbnail");
        }
        keys.push("url");
        if let Some(timestamp) = timestamp {
            crate::metadata_parser::set_field(&mut info, "timestamp", timestamp);
            keys.push("timestamp");
        }
        info.webpage_url = url.clone();
        info.extractor = CLIP_NAME.into();
        info.extractor_key = CLIP_NAME.into();
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("untyped-dict;{}", keys.join(",")),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://video.nest.com/embedded/live/4fvYdSo8AX?autoplay=0"
        ));
        assert!(suitable("https://video.nest.com/live/4fvYdSo8AX"));
        assert!(!suitable(
            "https://video.nest.com/clip/f34c9dd237a44eca9a0001af685e3dff"
        ));
        assert!(clip_suitable(
            "https://video.nest.com/embedded/clip/34e0432adc3c46a98529443d8ad5aa76"
        ));
    }
}
