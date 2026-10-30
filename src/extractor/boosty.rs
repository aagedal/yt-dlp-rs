//! Upstream `BoostyIE` (`yt_dlp/extractor/boosty.py`): a post's `ok_video`
//! players (HLS, DASH and progressive MP4 renditions) and its YouTube
//! videos, as one entry for a single video and a playlist otherwise. The
//! `auth` cookie's access token is sent when present.
use super::{
    Extractor, dict_format, dict_info, download_json_note_headers, download_webpage,
    extract_m3u8_master_optional, extract_mpd_optional_note,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Boosty;

const NAME: &str = "Boosty";
const MP4_TYPES: [&str; 8] = [
    "tiny", "lowest", "low", "medium", "high", "full_hd", "quad_hd", "ultra_hd",
];
const LOGIN_HINT: &str = "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?boosty\.to/(?P<user>[^/#?]+)/posts/(?P<post_id>[^/#?]+)")
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// One playlist entry: a YouTube `url` result, or a video of the post.
enum Entry {
    Url(String),
    Video(Box<VideoInfo>),
}

/// `url_result(url, YoutubeIE)` as a playlist entry (`url_transparent`
/// marker like the other hand-off entries).
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    entry.insert("ie_key".into(), "Youtube".into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": "Youtube",
            "extractor": "youtube",
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `qualities(_MP4_TYPES)(format_type)`.
fn quality(format_type: &str) -> i64 {
    MP4_TYPES
        .iter()
        .position(|item| *item == format_type)
        .map_or(-1, |index| index as i64)
}

/// `_extract_formats(player_urls, video_id)`.
fn extract_formats(
    client: &Client,
    player_urls: Option<&Value>,
    video_id: &str,
) -> Result<Vec<VideoFormat>> {
    let mut formats = Vec::new();
    let Some(players) = player_urls.and_then(Value::as_array) else {
        return Ok(formats);
    };
    for player in players {
        let Some(url) = player
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let format_type = player
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match format_type {
            "hls" | "hls_live" | "live_ondemand_hls" | "live_playback_hls" => {
                let manifest = Url::parse(&url)?;
                if let Some(stream) = extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    video_id,
                    Some("hls"),
                    false,
                    false,
                ) {
                    formats.extend(stream.formats);
                }
            }
            "dash" | "dash_live" | "live_playback_dash" => {
                let manifest = Url::parse(&url)?;
                if let Some(stream) = extract_mpd_optional_note(
                    client,
                    &manifest,
                    NAME,
                    video_id,
                    Some("dash"),
                    "Downloading MPD manifest",
                ) {
                    formats.extend(stream.formats);
                }
            }
            _ if MP4_TYPES.contains(&format_type) => {
                formats.push(dict_format(
                    vec![
                        ("url", url.into()),
                        ("ext", "mp4".into()),
                        ("format_id", format_type.into()),
                        ("quality", quality(format_type).into()),
                    ],
                    NAME,
                    video_id,
                )?);
            }
            _ => {
                crate::screen::warning(&format!(
                    "[{NAME}] Unknown format type: {}",
                    format_type_repr(player.get("type"))
                ));
            }
        }
    }
    Ok(formats)
}

/// Python's `repr()` of a JSON value (for the unknown-format warning).
fn format_type_repr(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => format!("'{text}'"),
        Some(Value::Null) | None => "None".into(),
        Some(other) => other.to_string(),
    }
}

/// `auth` cookie's `accessToken` as a bearer header (warns when unreadable).
fn authorization() -> Option<String> {
    let cookie = crate::cookies::get_cookie("https://boosty.to/", "auth")?;
    let decoded = percent_encoding::percent_decode_str(&cookie)
        .decode_utf8_lossy()
        .into_owned();
    match serde_json::from_str::<Value>(&decoded)
        .ok()
        .and_then(|data| {
            data.get("accessToken")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }) {
        Some(token) => Some(format!("Bearer {token}")),
        None => {
            crate::screen::warning(&format!(
                "[{NAME}] Failed to extract token from auth cookie; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
            None
        }
    }
}

/// `traverse_obj(post, ...)` of the common metadata, as ordered keys.
fn common_metadata(post: &Value) -> Vec<(&'static str, Value)> {
    let mut fields = Vec::new();
    let text = |path: &[&str]| {
        path.iter()
            .try_fold(post, |value, key| value.get(*key))
            .and_then(Value::as_str)
            .map(Value::from)
    };
    let int = |path: &[&str]| {
        path.iter()
            .try_fold(post, |value, key| value.get(*key))
            .and_then(Value::as_i64)
            .map(Value::from)
    };
    if let Some(name) = text(&["user", "name"]) {
        fields.push(("channel", name));
    }
    if let Some(id) = post.pointer("/user/id").and_then(|id| match id {
        Value::String(text) => Some(Value::from(text.as_str())),
        Value::Number(number) => Some(Value::from(number.to_string())),
        _ => None,
    }) {
        fields.push(("channel_id", id));
    }
    if let Some(created) = int(&["createdAt"]) {
        fields.push(("timestamp", created));
    }
    if let Some(published) = int(&["publishTime"]) {
        fields.push(("release_timestamp", published));
    }
    if let Some(updated) = int(&["updatedAt"]) {
        fields.push(("modified_timestamp", updated));
    }
    let tags: Vec<Value> = post
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tag| tag.get("title").and_then(Value::as_str).map(Value::from))
        .collect();
    if !tags.is_empty() {
        fields.push(("tags", Value::Array(tags)));
    }
    if let Some(likes) = int(&["count", "likes"]) {
        fields.push(("like_count", likes));
    }
    fields
}

impl Extractor for Boosty {
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
        let user = found["user"].to_owned();
        let post_id = found["post_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let mut headers = Vec::new();
        let authorization = authorization();
        if let Some(value) = &authorization {
            headers.push(("Authorization", value.as_str()));
        }
        let post = download_json_note_headers(
            client,
            &Url::parse(&format!(
                "https://api.boosty.to/v1/blog/{user}/post/{post_id}"
            ))?,
            NAME,
            &post_id,
            "Downloading post data",
            &headers,
        )?;

        let mut post_title = post
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if post_title.is_empty() {
            crate::screen::warning(&format!(
                "[{NAME}] Unable to extract post title. Falling back to parsing html page"
            ));
            let webpage = download_webpage(client, url, NAME, &post_id)?;
            post_title = crate::html::og_search_property(&webpage, "title")
                .or_else(|| crate::html::html_extract_title(&webpage))
                .unwrap_or_default();
        }
        let metadata = common_metadata(&post);

        let mut entries = Vec::new();
        for item in post
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.is_object())
        {
            let item_type = item.get("type").and_then(Value::as_str);
            let item_url = item
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none);
            if item_type == Some("video") && item_url.is_some() {
                entries.push(Entry::Url(item_url.unwrap_or_default()));
            } else if item_type == Some("ok_video") {
                let video_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .map_or_else(|| post_id.clone(), str::to_owned);
                let formats = extract_formats(client, item.get("playerUrls"), &video_id)?;
                let mut fields: Vec<(&str, Value)> = vec![
                    ("id", video_id.clone().into()),
                    ("alt_title", post_title.clone().into()),
                    ("formats", Value::Null),
                    ("title", post_title.clone().into()),
                ];
                fields.extend(
                    metadata
                        .iter()
                        .filter(|(key, _)| *key != "title")
                        .map(|(key, value)| (*key, value.clone())),
                );
                let title = item.get("title").and_then(Value::as_str);
                if let Some(title) = title {
                    for field in fields.iter_mut() {
                        if field.0 == "title" {
                            field.1 = title.into();
                        }
                    }
                }
                if let Some(duration) = item.get("duration").and_then(Value::as_i64) {
                    fields.push(("duration", duration.into()));
                }
                if let Some(views) = item.get("viewsCounter").and_then(Value::as_i64) {
                    fields.push(("view_count", views.into()));
                }
                let thumbnail = ["preview", "defaultPreview"]
                    .iter()
                    .filter_map(|key| item.get(*key).and_then(Value::as_str))
                    .find_map(super::json_ld::url_or_none);
                if let Some(thumbnail) = thumbnail {
                    fields.push(("thumbnail", thumbnail.into()));
                }
                entries.push(Entry::Video(Box::new(dict_info(
                    url,
                    (NAME, NAME),
                    fields,
                    formats,
                ))));
            }
        }

        if entries.is_empty() {
            if post.get("hasAccess").and_then(Value::as_bool) != Some(true) {
                return Err(Error::Reported(format!(
                    "[{NAME}] This post requires a subscription. {LOGIN_HINT}"
                )));
            }
            return Err(Error::Reported(format!("[{NAME}] No videos found")));
        }
        if entries.len() == 1 {
            return match entries.remove(0) {
                Entry::Url(target) => Ok(Some(super::follow_url(url, target))),
                Entry::Video(info) => Ok(Some(ExtractResult::Video(*info))),
            };
        }
        let mut videos = Vec::new();
        for entry in entries {
            videos.push(match entry {
                Entry::Url(target) => url_entry(&target)?,
                Entry::Video(info) => *info,
            });
        }
        // `playlist_result(entries, post_id, post_title, **common_metadata)`:
        // `_type`, `id`, `entries`, then `title` and a `None` description.
        let mut extra = Map::new();
        let mut keys = vec!["_type", "id", "entries", "title", "description"];
        extra.insert("description".into(), Value::Null);
        for (key, value) in metadata {
            keys.push(key);
            extra.insert(key.into(), value);
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("youtube-tab-plain;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: post_id,
            title: post_title,
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries: videos,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://boosty.to/kuplinov/posts/e55d050c-e3bb-4873-a7db-ac7a49b40c38"
        ));
        assert!(!suitable("https://boosty.to/kuplinov"));
    }

    #[test]
    fn mp4_types_are_ranked_in_order() {
        assert_eq!(quality("tiny"), 0);
        assert_eq!(quality("ultra_hd"), 7);
        assert_eq!(quality("hls"), -1);
    }
}
