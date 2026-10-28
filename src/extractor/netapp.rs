//! Upstream `NetAppVideoIE` and `NetAppCollectionIE`
//! (`yt_dlp/extractor/netapp.py`): NetApp media videos as transparent
//! Brightcove `url` results with the detail API's name, description and
//! creation time, and collections as playlists of such results.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NetAppVideo;
pub struct NetAppCollection;

const VIDEO_NAME: &str = "NetAppVideo";
const COLLECTION_NAME: &str = "NetAppCollection";
const BRIGHTCOVE: &str = "BrightcoveNew";

static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://media\.netapp\.com/video-detail/(?P<id>[0-9a-f-]+)").unwrap()
});
static COLLECTION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://media\.netapp\.com/collection/(?P<id>[0-9a-f-]+)").unwrap()
});

/// `NetAppVideoIE._match_id`.
pub fn video_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

/// `NetAppCollectionIE._match_id`.
pub fn collection_suitable(url: &str) -> bool {
    COLLECTION_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn brightcove_url(video_id: &str) -> String {
    format!(
        "https://players.brightcove.net/6255154784001/default_default/index.html?videoId={video_id}"
    )
}

fn each(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    }
}

/// `_parse_metadata(item)`.
fn parse_metadata(item: Option<&Value>) -> Map<String, Value> {
    let mut fields = Map::new();
    let Some(item) = item else {
        return fields;
    };
    for (key, source) in [("title", "name"), ("description", "description")] {
        if let Some(value) = item.get(source).filter(|value| value.is_string()) {
            fields.insert(key.into(), value.clone());
        }
    }
    if let Some(timestamp) = item
        .get("createdAt")
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
    {
        fields.insert("timestamp".into(), timestamp.into());
    }
    fields
}

fn id_text(value: &Value) -> Result<String> {
    match value {
        Value::String(id) => Ok(id.clone()),
        Value::Number(id) => Ok(id.to_string()),
        _ => Err(Error::UnsupportedHtml5(
            "NetApp Brightcove IDs that are not strings are not supported natively".into(),
        )),
    }
}

/// `url_result(BC_URL, BrightcoveNewIE, id, url_transparent=True, **meta)`:
/// the metadata keys precede `ie_key` and `id`.
fn transparent_entry(raw_id: &Value, metadata: Map<String, Value>) -> Result<VideoInfo> {
    let video_id = id_text(raw_id)?;
    let target = brightcove_url(&video_id);
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = video_id;
    let mut entry = metadata.clone();
    entry.insert("ie_key".into(), BRIGHTCOVE.into());
    entry.insert("id".into(), raw_id.clone());
    entry.insert("_type".into(), "url_transparent".into());
    entry.insert("url".into(), target.clone().into());
    let mut fields = metadata;
    fields.insert("id".into(), raw_id.clone());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": Value::Object(fields),
            "ie_key": BRIGHTCOVE,
            "extractor": BRIGHTCOVE,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for NetAppVideo {
    fn key(&self) -> &'static str {
        VIDEO_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_uuid = found["id"].to_owned();
        start(VIDEO_NAME, url);
        let api = Url::parse(&format!(
            "https://api.media.netapp.com/client/detail/{video_uuid}"
        ))?;
        let metadata = download_json(client, &api, VIDEO_NAME, &video_uuid)?;
        let sections = each(metadata.get("sections"));
        let of_type = |kind: &'static str| {
            sections
                .iter()
                .copied()
                .filter(move |section| section.get("type").and_then(Value::as_str) == Some(kind))
        };
        let Some(brightcove_id) = of_type("Player")
            .filter_map(|section| section.get("video"))
            .find_map(|video| video.as_str().map(str::to_owned))
        else {
            return Err(Error::Reported(format!(
                "[{VIDEO_NAME}] {video_uuid}: Unable to extract brightcove video id; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let video_item = of_type("VideoDetail").next();
        let mut fields = parse_metadata(video_item);
        fields.insert("id".into(), brightcove_id.clone().into());
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": brightcove_url(&brightcove_id), "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

impl Extractor for NetAppCollection {
    fn key(&self) -> &'static str {
        COLLECTION_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = COLLECTION_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let collection_uuid = found["id"].to_owned();
        start(COLLECTION_NAME, url);
        let api = Url::parse(&format!(
            "https://api.media.netapp.com/client/collection/{collection_uuid}"
        ))?;
        let metadata = download_json(client, &api, COLLECTION_NAME, &collection_uuid)?;
        let mut entries = Vec::new();
        for item in each(metadata.get("items")) {
            let Some(raw_id) = item
                .get("brightcoveVideoId")
                .filter(|id| super::nuxt::truthy(id))
            else {
                continue;
            };
            entries.push(transparent_entry(raw_id, parse_metadata(Some(item)))?);
        }
        let title = match metadata.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => Some(title.clone()).filter(|title| !title.is_empty()),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "NetApp collection names that are not strings are not supported natively"
                        .into(),
                ));
            }
        };
        Ok(Some(super::nrk::playlist_result(
            url,
            COLLECTION_NAME,
            collection_uuid,
            title,
            None,
            entries,
            true,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(video_suitable(
            "https://media.netapp.com/video-detail/da25fc01-82ad-5284-95bc-26920200a222/seamless-storage"
        ));
        assert!(collection_suitable(
            "https://media.netapp.com/collection/9820e190-f2a6-47ac-9c0a-98e5e64234a4"
        ));
        assert!(!video_suitable(
            "https://media.netapp.com/collection/9820e190"
        ));
    }
}
