//! Upstream `AbcNewsVideoIE` (`yt_dlp/extractor/abcnews.py`) and its
//! `AMPIE` base (`amp.py`): the Akamai AMP itemfeed's MRSS media (the HLS
//! master, the HDS manifest and the MP4 renditions), thumbnails and caption
//! tracks. The story page (`AbcNewsIE`) is not ported.
use super::clyp::key_error;
use super::{
    Extractor, dict_format, dict_info, direct, extract_f4m_optional, extract_m3u8_master_optional,
    streamable::int_or_none,
};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct AbcNewsVideo;

const NAME: &str = "abcnews:video";
const KEY: &str = "AbcNewsVideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(concat!(
        r"(?x)^https?://(?:",
        r"abcnews\.go\.com/",
        r"(?:",
        r"(?:[^/]+/)*video/(?P<display_id>[0-9a-z-]+)-|",
        r"video/(?:embed|itemfeed)\?.*?\bid=",
        r")|",
        r"fivethirtyeight\.abcnews\.go\.com/video/embed/\d+/",
        r")(?P<id>\d+)",
    ))
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python's truthiness of a parsed JSON value (`x or default`).
fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// `value` as a list of items (`dict` → one item, `None` → none).
fn as_list(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        Value::Null => Vec::new(),
        other => vec![other],
    }
}

/// `@attributes` of a node (`node.get('@attributes', {})`).
fn attributes(node: &Value) -> Value {
    node.get("@attributes")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()))
}

/// `AMPIE._extract_feed_info`'s `get_media_node(name)`.
fn media_node(item: &Value, name: &str) -> Value {
    let media_name = format!("media-{name}");
    let media_group = item
        .get("media-group")
        .filter(|group| truthy(group))
        .unwrap_or(item);
    for candidate in [media_group.get(&media_name), item.get(&media_name)]
        .into_iter()
        .flatten()
    {
        if truthy(candidate) {
            return candidate.clone();
        }
    }
    item.get(name).cloned().unwrap_or(Value::Null)
}

/// Downloads the feed (`_download_json` with the AMP errnote).
fn download_feed(client: &Client, url: &Url, video_id: &str) -> Result<Value> {
    // `_download_json(url, None, ...)`: the note has no video ID.
    crate::screen::screen(&format!("[{NAME}] Downloading Akamai AMP feed"));
    let response = direct::request(client, url, None).map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: NAME,
            id: video_id.to_owned(),
            what: "Akamai AMP feed",
            source: Box::new(error),
        },
        other => other,
    })?;
    let text = response.text().map_err(Error::Http)?;
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_extract_feed_info(url)` and `AbcNewsVideoIE`'s update: the feed's first
/// item as an info dict in the upstream key order (`id`, `title`,
/// `description`, `thumbnails`, `timestamp`, `duration`, `subtitles`,
/// `formats`, `display_id`).
fn extract_feed_info(
    client: &Client,
    feed_url: &Url,
    page: &Url,
    video_id: &str,
    display_id: Option<String>,
) -> Result<VideoInfo> {
    let feed = download_feed(client, feed_url, video_id)?;
    let channel = feed.get("channel").cloned().unwrap_or(Value::Null);
    let item = channel.get("item").cloned().unwrap_or(Value::Null);
    if !truthy(&item) {
        let error = feed
            .get("error")
            .ok_or_else(|| key_error(video_id, "error"))?;
        let text = match error {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        return Err(Error::Reported(format!(
            "[{NAME}] {video_id}: {NAME} said: {text}; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }

    let item_guid = item
        .get("guid")
        .ok_or_else(|| key_error(video_id, "guid"))?;
    let item_id = item_guid.as_str().unwrap_or_default().to_owned();

    let mut thumbnails = Vec::new();
    for thumbnail_data in as_list(media_node(&item, "thumbnail")) {
        let thumbnail = attributes(&thumbnail_data);
        let Some(thumbnail_url) = thumbnail
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        // Processing numbers the thumbnails without an `id` by position.
        thumbnails.push(serde_json::json!({
            "url": super::proto_relative_http(&thumbnail_url),
            "width": int_or_none(thumbnail.get("width")),
            "height": int_or_none(thumbnail.get("height")),
            "id": thumbnails.len().to_string(),
        }));
    }

    let mut subtitles = Subtitles::default();
    for subtitle_data in as_list(media_node(&item, "subTitle")) {
        let subtitle = attributes(&subtitle_data);
        let Some(href) = subtitle
            .get("href")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let lang = subtitle
            .get("lang")
            .filter(|lang| truthy(lang))
            .and_then(Value::as_str)
            .unwrap_or("en")
            .to_owned();
        let ext = media_ext(subtitle.get("type"), &href);
        let track = crate::SubtitleFormat {
            name: None,
            url: Url::parse(&href)?,
            ext,
            protocol: None,
            fragments: Vec::new(),
            extra: Map::new(),
        };
        push_track(&mut subtitles, Some(lang), track);
    }

    let mut formats = Vec::new();
    for media_data in as_list(media_node(&item, "content")) {
        let media = attributes(&media_data);
        let Some(media_url) = media
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let ext = media_ext(media.get("type"), &media_url);
        if ext == "f4m" {
            let manifest = Url::parse(&format!(
                "{media_url}?hdcore=3.4.0&plugin=aasp-3.4.0.132.124"
            ))?;
            formats.extend(extract_f4m_optional(
                client,
                NAME,
                &manifest,
                &item_id,
                Some("hds"),
            )?);
        } else if ext == "m3u8" {
            let manifest = Url::parse(&media_url)?;
            if let Some(stream) = extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &item_id,
                Some("hls"),
                true,
                false,
            ) {
                formats.extend(stream.formats);
                for (lang, tracks) in stream.subtitles.entries {
                    for track in tracks {
                        push_track(&mut subtitles, lang.clone(), track);
                    }
                }
            }
        } else {
            let label = media_data
                .get("media-category")
                .and_then(|category| category.get("@attributes"))
                .and_then(|attributes| attributes.get("label"))
                .cloned()
                .unwrap_or(Value::Null);
            formats.push(dict_format(
                vec![
                    ("format_id", label),
                    ("url", media_url.as_str().into()),
                    ("tbr", int_or_none(media.get("bitrate"))),
                    ("filesize", int_or_none(media.get("fileSize"))),
                    ("ext", ext.as_str().into()),
                ],
                NAME,
                &item_id,
            )?);
        }
    }

    let pub_date = item.get("pubDate").and_then(Value::as_str);
    let timestamp = pub_date
        .and_then(|date| crate::dates::unified_timestamp(date, true))
        .or_else(|| {
            item.get("dc-date")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
        });
    // `media_content[0]`: an empty list is upstream's IndexError.
    let first = as_list(media_node(&item, "content"))
        .into_iter()
        .next()
        .ok_or_else(|| Error::Reported("list index out of range".into()))?;
    let duration = int_or_none(attributes(&first).get("duration"));

    let mut info = dict_info(
        page,
        (NAME, KEY),
        vec![
            ("id", video_id.into()),
            ("title", media_node(&item, "title")),
            ("description", media_node(&item, "description")),
            ("thumbnails", Value::Array(thumbnails)),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
            ("duration", duration),
            ("subtitles", Value::Null),
            ("formats", Value::Null),
            ("display_id", display_id.map_or(Value::Null, Value::from)),
        ],
        formats,
    );
    info.subtitles = subtitles;
    Ok(info)
}

/// `mimetype2ext(type) or determine_ext(url)` for a media rendition.
fn media_ext(mime: Option<&Value>, media_url: &str) -> String {
    mime.and_then(Value::as_str)
        .and_then(|mime| {
            let ext = super::direct::mimetype2ext_or(mime, Some("\u{0}"));
            (ext != "\u{0}").then_some(ext)
        })
        .or_else(|| super::direct::determine_ext(media_url))
        .unwrap_or_else(|| "unknown_video".into())
}

/// Appends a track to `lang`'s list (`subtitles.setdefault(lang, [])`).
fn push_track(subtitles: &mut Subtitles, lang: Option<String>, track: crate::SubtitleFormat) {
    match subtitles.entries.iter_mut().find(|(key, _)| *key == lang) {
        Some((_, tracks)) => tracks.push(track),
        None => subtitles.entries.push((lang, vec![track])),
    }
}

impl Extractor for AbcNewsVideo {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found.name("display_id").map(|id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let feed_url = Url::parse(&format!(
            "http://abcnews.go.com/video/itemfeed?id={video_id}"
        ))?;
        // `info_dict.update({'id': video_id, 'display_id': display_id})`.
        extract_feed_info(client, &feed_url, url, &video_id, display_id).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://abcnews.go.com/ThisWeek/video/week-exclusive-irans-foreign-minister-zarif-20411932"
        ));
        assert!(suitable("http://abcnews.go.com/video/embed?id=46979033"));
        assert!(suitable(
            "https://abcnews.go.com/GMA/News/video/history-christmas-story-67894761"
        ));
        assert!(suitable(
            "https://fivethirtyeight.abcnews.go.com/video/embed/1234/56789"
        ));
        assert!(!suitable(
            "https://abcnews.go.com/Entertainment/peter-billingsley/story?id=51286501"
        ));
    }
}
