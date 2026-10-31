//! Upstream `ZenYandexIE` and `ZenYandexChannelIE` (`yt_dlp/extractor/yandexvideo.py`,
//! `IE_NAME` `dzen.ru` and `dzen.ru:channel`): a Zen video's `_params`
//! `ssrData` and its stream manifests, and a channel's paged feed of video
//! URLs. Upstream's `YandexVideoIE` and `YandexVideoPreviewIE` are not ported.
use super::{Extractor, dict_format, dict_info, download_json_note, download_webpage};
use crate::{ExtractResult, PlaylistInfo, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct ZenYandex;
pub struct ZenYandexChannel;

const NAME: &str = "dzen.ru";
const CHANNEL_NAME: &str = "dzen.ru:channel";
/// `qualities(('4', '0', '1', '2', '3', '5', '6', '7'))`.
const QUALITIES: [&str; 8] = ["4", "0", "1", "2", "3", "5", "6", "7"];

static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(concat!(
        r"^https?://(?:zen\.yandex|dzen)\.ru(?:/video)?/(?:media|watch)/",
        r"(?:(?:id/[^/]+/|[^/]+/)(?:[a-z0-9-]+)-)?(?P<id>[a-z0-9-]+)",
    ))
    .unwrap()
});
static CHANNEL_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:zen\.yandex|dzen)\.ru/(?:id/)?(?P<id>[a-z0-9-_]+)")
        .unwrap()
});
static REDIRECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:var|let|const)\s+it\s*=").unwrap());
static SSR_PARAMS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:var|let|const)\s+_params\s*=\s*\(\s*(\{["']ssrData.+\})"#).unwrap()
});

/// `_match_valid_url` for the video pattern.
pub fn suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

/// `_match_valid_url` for the channel pattern, whose `(?!media|video)`
/// lookahead is checked on the path's first segment.
pub fn channel_suitable(url: &str) -> bool {
    channel_id(url).is_some()
}

fn channel_id(url: &str) -> Option<String> {
    let (_, path) = url.split_once(".ru/")?;
    if path.starts_with("media") || path.starts_with("video") {
        return None;
    }
    CHANNEL_URL
        .captures(url)
        .map(|found| found["id"].to_owned())
}

fn key_error(id: &str, key: &str) -> crate::Error {
    super::clyp::key_error(id, key)
}

/// `_fetch_ssr_data`: a page's `it.retpath` redirect (the passport page), then
/// the page's `_params` `ssrData`.
fn fetch_ssr_data(
    client: &Client,
    (name, url): (&'static str, &Url),
    video_id: &str,
    match_id: &dyn Fn(&str) -> Option<String>,
) -> Result<(String, Value)> {
    let mut webpage = download_webpage(client, url, name, video_id)?;
    let mut video_id = video_id.to_owned();
    let redirect = super::search_json_with(&REDIRECT, &webpage, &str::to_owned)
        .flatten()
        .and_then(|value| {
            value
                .get("retpath")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    if let Some(redirect) = redirect {
        let target = Url::parse(&redirect)?;
        if let Some(id) = match_id(&redirect) {
            video_id = id;
        }
        webpage = super::download_webpage_note(client, &target, name, &video_id, "Redirecting")?;
    }
    let ssr = SSR_PARAMS
        .captures(&webpage)
        .and_then(|found| serde_json::from_str::<Value>(&found[1]).ok())
        .ok_or_else(|| {
            crate::Error::Reported(format!(
                "[{name}] {video_id}: Unable to extract metadata; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    let data = ssr.get("ssrData").cloned().unwrap_or(Value::Null);
    Ok((video_id, data))
}

/// `traverse_obj(video_data, ('video', ('id', ('streams', ...), ...)))` with
/// `url_or_none`, in first-seen order (upstream uses a set).
fn stream_urls(video: &Value) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut push = |value: Option<&Value>| {
        if let Some(url) = value
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            && !found.contains(&url)
        {
            found.push(url);
        }
    };
    push(video.get("id"));
    for stream in array(video.get("streams")) {
        push(Some(stream));
    }
    for stream in array(video.get("mp4Streams")) {
        push(stream.get("url"));
    }
    for stream in array(video.get("oneVideoStreams")) {
        push(stream.get("url"));
    }
    found
}

fn array(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `parse_qs(url)[key][0]`.
fn query_first(url: &str, key: &str) -> Option<String> {
    Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

/// `determine_ext(url)` with the stream's `ct` and `type` query values.
fn stream_format(s_url: &str, format_type: Option<String>) -> Result<crate::VideoFormat> {
    let quality = format_type
        .as_deref()
        .and_then(|name| QUALITIES.iter().position(|known| *known == name))
        .map_or(-1, |index| index as i64);
    dict_format(
        vec![
            ("url", s_url.into()),
            ("format_id", format_type.map_or(Value::Null, Value::from)),
            ("ext", "mp4".into()),
            ("quality", quality.into()),
        ],
        NAME,
        "",
    )
}

impl Extractor for ZenYandex {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        let match_id = |text: &str| VIDEO_URL.captures(text).map(|found| found["id"].to_owned());
        let (video_id, ssr) = fetch_ssr_data(client, (NAME, url), &found["id"], &match_id)?;
        let video_data = ssr
            .get("videoMetaResponse")
            .ok_or_else(|| key_error(&video_id, "videoMetaResponse"))?;

        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        let mut direct = Vec::new();
        for s_url in stream_urls(video_data.get("video").unwrap_or(&Value::Null)) {
            let s_url_text = s_url.clone();
            let ext = super::direct::determine_ext(&s_url_text).unwrap_or_default();
            let content_type = query_first(&s_url_text, "ct");
            let manifest = Url::parse(&s_url_text)?;
            if ext == "mpd" || content_type.as_deref() == Some("6") {
                if let Some(info) = super::extract_mpd_optional_note(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("dash"),
                    "Downloading MPD manifest",
                ) {
                    formats.extend(info.formats);
                    super::svt::merge_subtitles(&mut subtitles, info.subtitles);
                }
            } else if ext == "m3u8" || content_type.as_deref() == Some("8") {
                if let Some(info) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("hls"),
                    true,
                    false,
                ) {
                    formats.extend(info.formats);
                    super::svt::merge_subtitles(&mut subtitles, info.subtitles);
                }
            } else if content_type.as_deref() == Some("0") {
                direct.push(stream_format(
                    &s_url_text,
                    query_first(&s_url_text, "type"),
                )?);
            } else {
                crate::screen::warning(&format!(
                    "[{NAME}] Unsupported stream URL: {s_url_text}; {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
            }
        }
        // Direct streams keep their position among the manifest formats.
        let mut all = direct;
        all.append(&mut formats);
        let formats = all;

        let mut fields: Vec<(&str, Value)> = vec![("id", Value::from(video_id.as_str()))];
        fields.push(("formats", Value::Null));
        fields.push(("subtitles", Value::Null));
        let optional = |key: &str| video_data.get(key).cloned();
        if let Some(title) = optional("title").filter(Value::is_string) {
            fields.push(("title", title));
        }
        if let Some(description) = optional("description").filter(Value::is_string) {
            fields.push(("description", description));
        }
        if let Some(thumbnail) = video_data
            .get("image")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            fields.push(("thumbnail", thumbnail.into()));
        }
        let inner = video_data.get("video");
        let video_field = |key: &str| inner.and_then(|video| video.get(key));
        fields.push((
            "duration",
            super::streamable::int_or_none(video_field("duration")),
        ));
        fields.push((
            "view_count",
            super::streamable::int_or_none(video_field("views")),
        ));
        fields.push((
            "timestamp",
            super::streamable::int_or_none(video_data.get("publicationDate")),
        ));
        let tags: Vec<Value> = array(video_data.get("tags"))
            .iter()
            .filter(|tag| tag.is_string())
            .cloned()
            .collect();
        if !tags.is_empty() {
            fields.push(("tags", tags.into()));
        }
        if let Some(uploader) = video_data
            .get("source")
            .and_then(|source| source.get("title"))
            .filter(|title| title.is_string())
        {
            fields.push(("uploader", uploader.clone()));
        }

        let mut info = dict_info(url, (NAME, "ZenYandex"), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

/// `url_result(link, ZenYandexIE, id, title=...)` as a playlist entry, with
/// upstream's key order (`title`, `ie_key`, `id`, `_type`, `url`).
fn video_entry(link: &str, id: Option<&str>, title: Option<String>) -> Result<VideoInfo> {
    let url = Url::parse(link)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    if let Some(title) = title {
        entry.insert("title".into(), title.into());
    }
    entry.insert("ie_key".into(), "ZenYandex".into());
    if let Some(id) = id {
        entry.insert("id".into(), id.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), link.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        serde_json::json!({
            "url": link,
            "force_videoid": null,
            "fields": {},
            "ie_key": "ZenYandex",
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `_entries`: each block's video links, then the feed's `more` pages.
fn entries(client: &Client, first_feed: &Value, channel_id: &str) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    let mut feed = first_feed.clone();
    let mut next_page_id: Option<String> = None;
    for page in 1.. {
        for block in array(feed.get("items")) {
            if !matches!(
                block.get("tab").and_then(Value::as_str),
                Some("shorts" | "longs")
            ) {
                continue;
            }
            for item in array(block.get("items")) {
                let Some(link) = item
                    .get("link")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                else {
                    continue;
                };
                let id = item.get("id").and_then(Value::as_str);
                let title = item.get("title").and_then(Value::as_str).map(str::to_owned);
                entries.push(video_entry(&link, id, title)?);
            }
        }
        let more = feed
            .get("more")
            .and_then(|more| more.get("link"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let current_page_id = next_page_id.clone();
        next_page_id = more.as_deref().and_then(|link| {
            Url::parse(link).ok().and_then(|url| {
                url.query_pairs()
                    .filter(|(key, _)| key == "next_page_id")
                    .last()
                    .map(|(_, value)| value.into_owned())
            })
        });
        let (Some(more), Some(_), true) = (
            more.as_deref(),
            next_page_id.as_deref(),
            next_page_id != current_page_id,
        ) else {
            break;
        };
        let target = Url::parse(more)?;
        feed = download_json_note(
            client,
            &target,
            CHANNEL_NAME,
            channel_id,
            &format!("Downloading Page {}", page + 1),
        )?;
    }
    Ok(entries)
}

impl Extractor for ZenYandexChannel {
    fn key(&self) -> &'static str {
        CHANNEL_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(channel) = channel_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(CHANNEL_NAME);
        let match_id = |text: &str| channel_id(text);
        let (channel_id, ssr) = fetch_ssr_data(client, (CHANNEL_NAME, url), &channel, &match_id)?;
        let export = ssr
            .get("exportResponse")
            .ok_or_else(|| key_error(&channel_id, "exportResponse"))?;
        let feed = export
            .get("feedData")
            .ok_or_else(|| key_error(&channel_id, "feedData"))?;
        let entries = entries(client, feed, &channel_id)?;
        let source = export
            .get("channel")
            .and_then(|channel| channel.get("source"));
        let title = source
            .and_then(|source| source.get("title"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let description = source
            .and_then(|source| source.get("description"))
            .filter(|description| description.is_string())
            .cloned();
        // `playlist_result` with the channel's title and description; upstream's
        // entries are lazy, so `playlist_count` stays unset.
        let mut extra = Map::new();
        if title.is_none() {
            extra.insert("title".into(), Value::Null);
        }
        extra.insert("description".into(), description.unwrap_or(Value::Null));
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;title,description,id{}",
                crate::info_order::GENERATOR
            )
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: channel_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: CHANNEL_NAME.into(),
            extractor_key: "ZenYandexChannel".into(),
            nested: Vec::new(),
            pending: None,
            entries,
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
            "https://zen.yandex.ru/media/id/606fd806cc13cb3c58c05cf5/vot-eto-focus-dedy-morozy-na-gidrociklah-60c7c443da18892ebfe85ed7"
        ));
        assert!(suitable(
            "https://dzen.ru/video/watch/6002240ff8b1af50bb2da5e3"
        ));
        assert!(!suitable(
            "https://zen.yandex.ru/id/606fd806cc13cb3c58c05cf5"
        ));
    }

    #[test]
    fn channel_urls_follow_upstream_valid_url() {
        assert!(channel_suitable("https://zen.yandex.ru/tok_media"));
        assert!(channel_suitable(
            "https://dzen.ru/id/606fd806cc13cb3c58c05cf5"
        ));
        assert!(!channel_suitable(
            "https://zen.yandex.ru/media/id/606fd806cc13cb3c58c05cf5/x"
        ));
        assert!(!channel_suitable(
            "https://dzen.ru/video/watch/6002240ff8b1af50bb2da5e3"
        ));
    }
}
