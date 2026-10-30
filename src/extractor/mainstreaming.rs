//! Upstream `MainStreamingIE` (`yt_dlp/extractor/mainstreaming.py`): the
//! MainStreaming `webtools` player's content API (`/api/v2/content/ID`)
//! for live streams (heartbeat check, HLS and DASH manifests with the
//! DVR query), VOD (the progressive `original.mp4` plus its manifests) and
//! playlists (url entries). The embedded-page fallback is ported only for
//! its error path; DRM-flagged content and unknown content types error.
use super::{
    Extractor, dict_format, download_webpage, extract_m3u8_master_optional, extract_mpd_optional,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub struct MainStreaming;

const NAME: &str = "MainStreaming";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https?://(?:webtools-?)?(?P<host>[A-Za-z0-9-]*\.msvdn\.net)/(?:embed|amp_embed|content)/(?P<id>\w+)",
    )
    .unwrap()
});
static CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)config\s*=\s*(\{.+?\})\s*;").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MainStreaming {what} are not supported natively"))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_get_webtools_host(host)`.
fn webtools_host(host: &str) -> String {
    if host.starts_with("webtools") {
        host.to_owned()
    } else if host.starts_with('.') {
        format!("webtools{host}")
    } else {
        format!("webtools-{host}")
    }
}

/// `_get_webtools_base_url(host)`.
fn webtools_base(host: &str) -> String {
    format!("https://{}", webtools_host(host))
}

/// `_download_json(url, id, note=..., fatal=False)`: `None` with a warning
/// on HTTP errors.
fn download_json_optional(
    client: &Client,
    url: &Url,
    id: &str,
    note: &str,
) -> Result<Option<Value>> {
    crate::screen::screen(&format!("[{NAME}] {id}: {note}"));
    let mut response = match super::direct::request(client, url, None) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, response.headers());
    crate::json::from_str(&text)
        .map(Some)
        .map_err(|_| unsupported("API responses that are not JSON"))
}

/// `_call_api(host, path, item_id, note=..., fatal=False)`.
fn call_api(
    client: &Client,
    host: &str,
    path: &str,
    id: &str,
    note: &str,
) -> Result<Option<Value>> {
    let url = Url::parse(&format!("{}/api/v2/{path}", webtools_base(host)))?;
    download_json_optional(client, &url, id, note)
}

/// `_playlist_entries(host, playlist_content)`.
fn playlist_entry(host: &str, entry: &Value) -> Result<VideoInfo> {
    let content_id = entry
        .get("contentID")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let target = format!("https://{host}/embed/{content_id}");
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = content_id.to_owned();
    let mut raw = Map::new();
    raw.insert("_type".into(), "url".into());
    raw.insert("ie_key".into(), NAME.into());
    raw.insert("id".into(), content_id.into());
    let duration = super::streamable::int_or_none(entry.pointer("/duration/totalSeconds"));
    raw.insert("duration".into(), duration.clone());
    let title = entry.get("title").cloned().unwrap_or(Value::Null);
    raw.insert("title".into(), title.clone());
    raw.insert("url".into(), target.clone().into());
    let mut overlay = Map::new();
    overlay.insert("duration".into(), duration);
    overlay.insert("title".into(), title);
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": overlay,
            "ie_key": NAME,
            "extractor": NAME,
            "entry": Value::Object(raw),
        }),
    );
    Ok(info)
}

/// The embedded player config's `contentInfo` (`_real_extract`'s fallback).
fn fallback_content_info(client: &Client, url: &Url, id: &str) -> Result<Value> {
    let webpage = download_webpage(client, url, NAME, id)?;
    let config = CONFIG
        .captures(&webpage)
        .and_then(|found| crate::json::from_str(&crate::js_to_json::js_to_json(&found[1])).ok())
        .unwrap_or_else(|| Value::Object(Map::new()));
    config
        .get("contentInfo")
        .cloned()
        .ok_or_else(|| key_error(id, "contentInfo"))
}

impl Extractor for MainStreaming {
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
        let (mut host, mut video_id) = (found["host"].to_owned(), found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let mut content_info = call_api(
            client,
            &host,
            &format!("content/{video_id}"),
            &video_id,
            "Downloading content info API JSON",
        )?
        .and_then(|response| response.get("playerContentInfo").cloned())
        .filter(Value::is_object);
        if content_info.is_none() {
            content_info = Some(fallback_content_info(client, url, &video_id)?);
        }
        let content = content_info.unwrap_or(Value::Null);

        if let Some(value) = content.get("host").and_then(Value::as_str)
            && !value.is_empty()
        {
            host = value.to_owned();
        }
        if let Some(value) = content.get("contentID").and_then(Value::as_str)
            && !value.is_empty()
        {
            video_id = value.to_owned();
        }
        let title = content.get("title").cloned().unwrap_or(Value::Null);
        let description = ["longDescription", "shortDescription"]
            .iter()
            .filter_map(|key| content.get(*key))
            .find(|value| value.is_string())
            .cloned();
        if content.get("drmEnabled").is_some_and(super::nuxt::truthy) {
            crate::no_formats::raise_expected(
                NAME,
                Some(&video_id),
                None,
                "This video is DRM protected",
            )?;
        }
        if let Some(alternative) = content.get("alternativeContentID")
            && super::nuxt::truthy(alternative)
        {
            crate::screen::warning(&format!(
                "[{NAME}] Ignoring alternative content ID: {}",
                alternative
                    .as_str()
                    .map_or_else(|| alternative.to_string(), str::to_owned)
            ));
        }

        let content_type = super::streamable::int_or_none(content.get("contentType"));
        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        match content_type.as_i64() {
            // Live content.
            Some(20) => {
                let dvr = content
                    .pointer("/playerSettings/dvrEnabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                // `content_info['liveSourceID']` (a `None` value prints as `None`).
                let source = match content.get("liveSourceID") {
                    Some(Value::String(source)) => source.clone(),
                    Some(Value::Null) => "None".to_owned(),
                    Some(value) => value.to_string(),
                    None => return Err(key_error(&video_id, "liveSourceID")),
                };
                let base = format!(
                    "https://{host}/live/{source}/{video_id}/%s{}",
                    if dvr { "?DVR" } else { "" }
                );
                let heartbeat = call_api(
                    client,
                    &host,
                    &format!("heartbeat/{video_id}"),
                    &video_id,
                    "Checking stream status",
                )?
                .unwrap_or_else(|| Value::Object(Map::new()));
                if heartbeat.get("heartBeatUp").and_then(Value::as_bool) == Some(false) {
                    let message = match heartbeat.get("responseMessage") {
                        Some(Value::String(message)) => message.clone(),
                        Some(Value::Null) | None => "None".to_owned(),
                        Some(value) => value.to_string(),
                    };
                    crate::no_formats::raise_expected(
                        NAME,
                        Some(&video_id),
                        None,
                        &format!("MainStreaming said: {message}"),
                    )?;
                }
                let m3u8 = Url::parse(&base.replace("%s", "playlist.m3u8"))?;
                if let Some(mut parsed) =
                    extract_m3u8_master_optional(client, &m3u8, NAME, &video_id, None, true, false)
                {
                    // `_extract_m3u8_formats_and_subtitles` without `ext`.
                    super::hls::without_ext(&mut parsed);
                    formats.extend(parsed.formats);
                    super::svt::merge_subtitles(&mut subtitles, parsed.subtitles);
                }
                let mpd = Url::parse(&base.replace("%s", "manifest.mpd"))?;
                if let Some(parsed) = extract_mpd_optional(client, &mpd, NAME, &video_id, true) {
                    formats.extend(parsed.formats);
                    super::svt::merge_subtitles(&mut subtitles, parsed.subtitles);
                }
                let live_status = Value::from("is_live");
                finish(
                    url,
                    &host,
                    &video_id,
                    title,
                    description,
                    formats,
                    subtitles,
                    &content,
                    live_status,
                )
            }
            // Playlist.
            Some(31) => {
                let entries = content
                    .get("playlistContents")
                    .and_then(Value::as_array)
                    .ok_or_else(|| unsupported("playlists without entries"))?
                    .iter()
                    .map(|entry| playlist_entry(&host, entry))
                    .collect::<Result<Vec<_>>>()?;
                let title_text = title.as_str().map(str::to_owned).unwrap_or_default();
                let mut extra = Map::new();
                extra.insert(
                    crate::info_order::INFO_SHAPE.into(),
                    "playlist-dict;id,title".into(),
                );
                if let Some(text) = description.as_ref().and_then(Value::as_str) {
                    extra.insert("description".into(), text.into());
                }
                Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                    id: video_id,
                    title: title_text,
                    webpage_url: url.clone(),
                    extractor: NAME.into(),
                    extractor_key: NAME.into(),
                    nested: Vec::new(),
                    pending: None,
                    entries,
                    extra,
                })))
            }
            // Normal video content.
            Some(10) => {
                let base = format!("https://{host}/vod/{video_id}/%s");
                formats.push(dict_format(
                    vec![
                        ("url", Value::from(base.replace("%s", "original.mp4"))),
                        ("format_note", "original".into()),
                        ("quality", 1.into()),
                    ],
                    NAME,
                    &video_id,
                )?);
                let m3u8 = Url::parse(&base.replace("%s", "playlist.m3u8"))?;
                if let Some(mut parsed) =
                    extract_m3u8_master_optional(client, &m3u8, NAME, &video_id, None, true, false)
                {
                    // `_extract_m3u8_formats_and_subtitles` without `ext`.
                    super::hls::without_ext(&mut parsed);
                    formats.extend(parsed.formats);
                    super::svt::merge_subtitles(&mut subtitles, parsed.subtitles);
                }
                let mpd = Url::parse(&base.replace("%s", "manifest.mpd"))?;
                if let Some(parsed) = extract_mpd_optional(client, &mpd, NAME, &video_id, true) {
                    formats.extend(parsed.formats);
                    super::svt::merge_subtitles(&mut subtitles, parsed.subtitles);
                }
                finish(
                    url,
                    &host,
                    &video_id,
                    title,
                    description,
                    formats,
                    subtitles,
                    &content,
                    Value::from("not_live"),
                )
            }
            _ => Err(unsupported(
                "content types other than live, VOD and playlists",
            )),
        }
    }
}

/// The video result shared by live and VOD content (`_real_extract`'s
/// final dict).
#[allow(clippy::too_many_arguments)]
fn finish(
    url: &Url,
    host: &str,
    video_id: &str,
    title: Value,
    description: Option<Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: Subtitles,
    content: &Value,
    live_status: Value,
) -> Result<Option<ExtractResult>> {
    let fields = vec![
        ("id", Value::from(video_id)),
        ("title", title),
        ("description", description.unwrap_or(Value::Null)),
        ("formats", Value::Null),
        ("live_status", live_status),
        (
            "duration",
            content
                .get("duration")
                .and_then(Value::as_str)
                .and_then(crate::parse_duration)
                .and_then(|duration| serde_json::to_value(duration).ok())
                .unwrap_or(Value::Null),
        ),
        ("tags", content.get("tags").cloned().unwrap_or(Value::Null)),
        ("subtitles", Value::Null),
        (
            "thumbnail",
            Value::from(format!("{}/image/{video_id}/poster", webtools_base(host))),
        ),
    ];
    let mut info = super::dict_info(url, (NAME, NAME), fields, formats);
    info.subtitles = subtitles;
    Ok(Some(ExtractResult::Video(info)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://webtools-e18da6642b684f8aa9ae449862783a56.msvdn.net/embed/53EN6GxbWaJC"
        ));
        assert!(suitable(
            "https://859c1818ed614cc5b0047439470927b0.msvdn.net/amp_embed/tDoFkZD3T1Lw"
        ));
        assert!(suitable("https://webtools.msvdn.net/embed/MfuWmzL2lGkA"));
        assert!(!suitable("https://www.msvdn.example/embed/abc"));
    }

    #[test]
    fn webtools_hosts_follow_upstream_prefixing() {
        assert_eq!(
            webtools_host("example.msvdn.net"),
            "webtools-example.msvdn.net"
        );
        assert_eq!(webtools_host("webtools.msvdn.net"), "webtools.msvdn.net");
        assert_eq!(webtools_host(".msvdn.net"), "webtools.msvdn.net");
    }
}
