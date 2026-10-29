//! Upstream `Kenh14VideoIE` and `Kenh14PlaylistIE` (`yt_dlp/extractor/kenh14.py`):
//! a video page's `VideoStream` source (the direct MP4, plus the optional
//! HLS and DASH manifests from its `.json` sidecar), the kinghub metadata API
//! and the OpenGraph/page fallbacks; and a category playlist's `video-item`
//! links as URL results.
use super::json_ld::url_or_none;
use super::{
    Extractor, dict_format, dict_info, download_json_note, download_webpage,
    extract_m3u8_master_optional, extract_mpd_optional_note, search_warning,
};
use crate::{ExtractResult, PlaylistInfo, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Kenh14Video;
pub struct Kenh14Playlist;

const VIDEO_NAME: &str = "Kenh14Video";
const PLAYLIST_NAME: &str = "Kenh14Playlist";

static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://video\.kenh14\.vn/(?:video/)?[\w-]+-(?P<id>[0-9]+)\.chn").unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://video\.kenh14\.vn/playlist/[\w-]+-(?P<id>[0-9]+)\.chn").unwrap()
});

/// `Kenh14VideoIE._match_id`.
pub fn video_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

/// `Kenh14PlaylistIE._match_id`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `KeyError` reported against the URL's ID.
fn key_error(id: &str, key: &str) -> crate::Error {
    super::clyp::key_error(id, key)
}

/// `_download_json(..., fatal=False)`: a failure is the warning and no data.
fn optional_json(client: &Client, url: &Url, id: &str, note: &str) -> Option<Value> {
    match download_json_note(client, url, VIDEO_NAME, id, note) {
        Ok(value) => Some(value),
        Err(error) => {
            crate::screen::warning(&crate::upstream_error_text(&error));
            None
        }
    }
}

/// `x or y` for optional strings: an empty string is falsy.
fn truthy(text: Option<String>) -> Option<String> {
    text.filter(|text| !text.is_empty())
}

/// `clean_html(None)` is `None`.
fn clean(text: Option<String>) -> Option<String> {
    text.map(|text| crate::html::clean_html(&text))
}

/// `_og_search_*` with its non-fatal warning when the property is missing.
fn og(webpage: &str, property: &str, name: &str) -> Option<String> {
    let found = crate::html::og_search_property(webpage, property);
    if found.is_none() {
        search_warning(VIDEO_NAME, name);
    }
    found
}

/// The text of the first element with `class` (`get_element_by_class`).
fn element_text_by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()
        .and_then(|elements| elements.into_iter().next().map(|(text, _)| text))
}

/// The `kenh14` video's `_real_extract`.
fn extract_video(client: &Client, url: &Url, video_id: &str) -> Result<VideoInfo> {
    let webpage = download_webpage(client, url, VIDEO_NAME, video_id)?;

    // `extract_attributes(get_element_html_by_attribute('type', 'VideoStream'))`.
    let attrs =
        crate::html_elements::elements_text_and_html_by_attribute("type", "VideoStream", &webpage)
            .ok()
            .and_then(|elements| elements.into_iter().next())
            .map(|(_, element)| crate::html_elements::extract_attributes(&element))
            .unwrap_or_default();
    let direct_url = match attrs.get("data-vid") {
        Some(Value::String(direct)) => direct.clone(),
        _ => return Err(key_error(video_id, "data-vid")),
    };
    let file_name = direct_url
        .strip_prefix("kenh14cdn.com/")
        .unwrap_or(&direct_url);

    let metadata = optional_json(
        client,
        &Url::parse(&format!(
            "https://api.kinghub.vn/video/api/v1/detailVideoByGet?FileName={file_name}"
        ))?,
        video_id,
        "Downloading JSON metadata",
    );

    let mut formats = vec![dict_format(
        vec![
            ("url", format!("https://{direct_url}").into()),
            ("format_id", "http".into()),
            ("quality", 1.into()),
        ],
        VIDEO_NAME,
        video_id,
    )?];
    let mut subtitles = Subtitles::default();
    let video_data = optional_json(
        client,
        &Url::parse(&format!("https://{direct_url}.json"))?,
        video_id,
        "Downloading video data",
    );
    if let Some(hls_url) = video_data
        .as_ref()
        .and_then(|data| data.get("hls"))
        .and_then(Value::as_str)
        .and_then(url_or_none)
    {
        let manifest = Url::parse(&hls_url)?;
        if let Some(stream) = extract_m3u8_master_optional(
            client,
            &manifest,
            VIDEO_NAME,
            video_id,
            Some("hls"),
            true,
            false,
        ) {
            formats.extend(stream.formats);
            super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
        }
    }
    if let Some(dash_url) = video_data
        .as_ref()
        .and_then(|data| data.get("mpd"))
        .and_then(Value::as_str)
        .and_then(url_or_none)
    {
        let manifest = Url::parse(&dash_url)?;
        if let Some(stream) = extract_mpd_optional_note(
            client,
            &manifest,
            VIDEO_NAME,
            video_id,
            Some("dash"),
            "Downloading MPD manifest",
        ) {
            formats.extend(stream.formats);
            super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
        }
    }

    // `**traverse_obj(metadata, {...})`: only the keys that have values.
    let mut fields: Vec<(&str, Value)> = Vec::new();
    if let Some(metadata) = &metadata {
        let text = |key: &str| metadata.get(key).and_then(Value::as_str);
        if let Some(duration) = text("duration").and_then(crate::parse_duration) {
            fields.push((
                "duration",
                serde_json::to_value(duration).unwrap_or(Value::Null),
            ));
        }
        if let Some(uploader) = truthy(text("author").map(|text| text.trim().to_owned())) {
            fields.push(("uploader", uploader.into()));
        }
        if let Some(timestamp) = text("uploadtime").and_then(crate::dates::parse_iso8601_space) {
            fields.push(("timestamp", timestamp.into()));
        }
        let views = super::streamable::int_or_none(metadata.get("views"));
        if !views.is_null() {
            fields.push(("view_count", views));
        }
    }

    let meta_title = metadata
        .as_ref()
        .and_then(|data| data.get("title"))
        .and_then(Value::as_str)
        .map(|text| text.trim().to_owned());
    let title = truthy(meta_title)
        .or_else(|| truthy(clean(og(&webpage, "title", "OpenGraph title"))))
        .or_else(|| truthy(clean(element_text_by_class("vdbw-title", &webpage))));
    let description = truthy(clean(og(&webpage, "description", "OpenGraph description")))
        .or_else(|| truthy(clean(element_text_by_class("vdbw-sapo", &webpage))));
    let thumbnail = og(&webpage, "image", "thumbnail URL").or_else(|| {
        attrs
            .get("data-thumb")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    let tags = match crate::html::html_search_meta(&webpage, "keywords") {
        Some(keywords) => {
            let items: Vec<Value> = keywords
                .split(';')
                .filter(|item| !item.is_empty())
                .map(Value::from)
                .collect();
            Value::Array(items)
        }
        None => {
            search_warning(VIDEO_NAME, "keywords");
            Value::Null
        }
    };

    fields.extend([
        ("id", video_id.into()),
        ("title", title.map_or(Value::Null, Value::from)),
        ("formats", Value::Null),
        ("subtitles", Value::Null),
        ("description", description.map_or(Value::Null, Value::from)),
        ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
        ("tags", tags),
    ]);
    let mut info = dict_info(url, (VIDEO_NAME, VIDEO_NAME), fields, formats);
    info.subtitles = subtitles;
    Ok(info)
}

/// The playlist's `_real_extract`: `playlist_from_matches` over `video-item`
/// links, titled and described from the category block or JSON-LD.
fn extract_playlist(client: &Client, url: &Url, playlist_id: &str) -> Result<ExtractResult> {
    let webpage = download_webpage(client, url, PLAYLIST_NAME, playlist_id)?;

    let category_detail = element_text_by_class("category-detail", &webpage).unwrap_or_default();
    let embed_info: Map<String, Value> = crate::extractor::json_ld::yield_json_ld(&webpage)
        .into_iter()
        .find_map(|value| {
            let object = value.as_object()?.clone();
            let truthy_key = |key: &str| object.get(key).is_some_and(super::nuxt::truthy);
            (truthy_key("name") && truthy_key("alternateName")).then_some(object)
        })
        .unwrap_or_default();
    let unescaped = |key: &str| {
        embed_info
            .get(key)
            .and_then(Value::as_str)
            .map(crate::html::unescape_html)
    };

    let title = truthy(clean(element_text_by_class("name", &category_detail)))
        .or_else(|| truthy(unescaped("name")));
    let description = truthy(clean(element_text_by_class(
        "description",
        &category_detail,
    )))
    .or_else(|| truthy(unescaped("alternateName")));
    let thumbnail = og(&webpage, "image", "thumbnail URL")
        .and_then(|text| url_or_none(&text))
        .and_then(|text| Url::parse(&text).ok())
        .map(|mut thumbnail| {
            thumbnail.set_query(None);
            thumbnail.to_string()
        });

    let matches = crate::html_elements::elements_text_and_html_by_class("video-item", &webpage)
        .unwrap_or_default();
    let mut entries = Vec::new();
    for (_, element) in matches {
        let attrs = crate::html_elements::extract_attributes(&element);
        let data_id = match attrs.get("data-id") {
            Some(Value::String(data_id)) => data_id.clone(),
            _ => return Err(key_error(playlist_id, "data-id")),
        };
        let target = format!("https://video.kenh14.vn/video/video-{data_id}.chn");
        entries.push(super::nrk::url_entry(
            &target, VIDEO_NAME, None, None, None,
        )?);
    }

    let mut extra = Map::new();
    let mut keys = vec!["thumbnail", "id"];
    extra.insert(
        "thumbnail".into(),
        thumbnail.map_or(Value::Null, Value::from),
    );
    if title.is_some() {
        keys.push("title");
    }
    if let Some(description) = description {
        keys.push("description");
        extra.insert("description".into(), description.into());
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    Ok(ExtractResult::Playlist(PlaylistInfo {
        id: playlist_id.to_owned(),
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: PLAYLIST_NAME.into(),
        extractor_key: PLAYLIST_NAME.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    }))
}

impl Extractor for Kenh14Video {
    fn key(&self) -> &'static str {
        VIDEO_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        announce(VIDEO_NAME, url);
        let video_id = found["id"].to_owned();
        extract_video(client, url, &video_id).map(Some)
    }
}

impl Extractor for Kenh14Playlist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        announce(PLAYLIST_NAME, url);
        let playlist_id = found["id"].to_owned();
        extract_playlist(client, url, &playlist_id).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(video_suitable(
            "https://video.kenh14.vn/video/mo-hop-iphone-14-pro-max-nguon-unbox-therapy-316173.chn"
        ));
        assert!(video_suitable("https://video.kenh14.vn/video-316174.chn"));
        assert!(video_suitable("https://video.kenh14.vn/0-344740.chn"));
        assert!(!video_suitable("https://video.kenh14.vn/playlist/0-72.chn"));
        assert!(playlist_suitable(
            "https://video.kenh14.vn/playlist/tran-tinh-naked-love-mua-2-71.chn"
        ));
        assert!(playlist_suitable(
            "https://video.kenh14.vn/playlist/0-72.chn"
        ));
    }
}
