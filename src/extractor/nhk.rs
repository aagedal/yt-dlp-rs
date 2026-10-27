//! Upstream `NhkVodIE` and `NhkVodProgramIE` (`yt_dlp/extractor/nhk.py`):
//! NHK World on-demand episodes and clips through the `showsapi` API
//! (`NhkBaseIE._extract_episode_info`: video HLS or audio `vod-stream`
//! HLS, the expired-stream error, metadata and thumbnails) and program pages
//! as playlists of their episodes or clips, extracted as the playlist is
//! processed like upstream's generator.
//!
//! Not ported: the NHK for School and Radiru extractors.
use super::{Extractor, dict_info, download_json, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NhkVod;
pub struct NhkVodProgram;

const VOD_NAME: &str = "NhkVod";
const PROGRAM_NAME: &str = "NhkVodProgram";
const BASE: &str = r"https?://www3\.nhk\.or\.jp/nhkworld/(?P<lang>[a-z]{2})/";

static VOD_URL: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(&format!(
            r"^{BASE}shows/(?:(?P<type>video)/)?(?P<id>\d{{4}}[\da-z]\d+)/?(?:$|[?#])"
        ))
        .unwrap(),
        Regex::new(&format!(
            r"^{BASE}(?:ondemand|shows)/(?P<type>audio)/(?P<id>[^/?#]+?-\d{{8}}-[\da-z]+)"
        ))
        .unwrap(),
        Regex::new(&format!(
            r"^{BASE}ondemand/(?P<type>video)/(?P<id>\d{{4}}[\da-z]\d+)"
        ))
        .unwrap(),
    ]
});
static PROGRAM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^{BASE}(?:shows|tv)/(?:(?P<type>audio)/programs/)?(?P<id>\w+)/?(?:\?(?:[^#]+&)?type=(?P<episode_type>clip|(?:radio|tv)Episode))?"
    ))
    .unwrap()
});

/// `NhkVodIE._match_valid_url`: `(lang, type, id)`.
fn vod_match(url: &str) -> Option<(String, Option<String>, String)> {
    VOD_URL
        .iter()
        .find_map(|pattern| pattern.captures(url))
        .map(|found| {
            (
                found["lang"].to_owned(),
                found.name("type").map(|kind| kind.as_str().to_owned()),
                found["id"].to_owned(),
            )
        })
}

/// `NhkVodIE._match_valid_url`.
pub fn vod_suitable(url: &str) -> bool {
    vod_match(url).is_some()
}

/// `NhkVodProgramIE.suitable`: not an `NhkVodIE` URL.
pub fn program_suitable(url: &str) -> bool {
    !vod_suitable(url) && PROGRAM_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("NHK {what} are not supported natively"))
}

/// `_call_api(m_id, lang, is_video, is_episode, is_clip)`.
fn call_api(
    client: &Client,
    name: &'static str,
    m_id: &str,
    lang: &str,
    (video, episode, clip): (bool, bool, bool),
) -> Result<Value> {
    let format = if video { "video" } else { "audio" };
    let kind = if clip { "clips" } else { "episodes" };
    let (page, extra) = if episode {
        (kind.to_owned(), String::new())
    } else {
        ("programs".to_owned(), format!("/{format}_{kind}"))
    };
    let url = Url::parse(&format!(
        "https://api.nhkworld.jp/showsapi/v1/{lang}/{format}_{page}/{m_id}{extra}"
    ))?;
    // `join_nonempty(m_id, lang)`.
    download_json(client, &url, name, &format!("{m_id}-{lang}"))
}

/// `urljoin(base, value)`: an absolute HTTP(S) URL, else `None`.
fn urljoin(base: &Url, value: Option<&Value>) -> Option<String> {
    let value = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    base.join(value)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.to_string())
}

/// `join_nonempty` of a value's Python `str`.
fn py_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(text) if text.is_empty() => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(false) => None,
        Value::Number(number) if number.as_f64() == Some(0.0) => None,
        Value::Number(number) => Some(number.to_string()),
        other => Some(other.to_string()),
    }
}

/// `('KEY', ..., 'name', {str})` (dropped when empty).
fn names(episode: &Value, key: &str) -> Option<Value> {
    let names: Vec<Value> = episode
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("name").filter(|name| name.is_string()).cloned())
        .collect();
    (!names.is_empty()).then_some(Value::Array(names))
}

/// `_extract_episode_info(url, episode)`.
fn episode_info(
    client: &Client,
    name: &'static str,
    url: &Url,
    episode: Option<Value>,
) -> Result<VideoInfo> {
    let Some((lang, kind, episode_id)) = vod_match(url.as_str()) else {
        return Err(unsupported("episode URLs that are not on-demand pages"));
    };
    let video = kind.as_deref() != Some("audio");
    let episode = match episode {
        Some(episode) => episode,
        None => call_api(
            client,
            name,
            &episode_id,
            &lang,
            (video, true, episode_id.starts_with("9999")),
        )?,
    };
    let video_id = [py_text(episode.get("id")), py_text(episode.get("lang"))]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("-");
    let title = episode
        .get("title")
        .filter(|title| !title.is_null())
        .cloned();
    // `(('video_program', 'audio_program'), any, 'title')`.
    let series = ["video_program", "audio_program"]
        .iter()
        .find_map(|key| episode.get(*key).filter(|value| !value.is_null()))
        .and_then(|program| program.get("title"))
        .filter(|title| !title.is_null())
        .cloned();
    let truthy = |value: &Option<Value>| value.as_ref().is_some_and(super::nuxt::truthy);
    let (title, series, episode_name) = match (truthy(&series), truthy(&title)) {
        (true, true) => (
            Some(Value::from(format!(
                "{} - {}",
                series.as_ref().and_then(Value::as_str).unwrap_or_default(),
                title.as_ref().and_then(Value::as_str).unwrap_or_default()
            ))),
            series,
            title,
        ),
        (true, false) => (series, None, None),
        _ => (title, series, None),
    };
    let thumbnails: Vec<Value> = episode
        .get("images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|image| image.get("url").is_some_and(super::nuxt::truthy))
        .map(|image| {
            let mut thumbnail = Map::new();
            if let Some(image_url) = urljoin(url, image.get("url")) {
                thumbnail.insert("url".into(), image_url.into());
            }
            for key in ["width", "height"] {
                let value = super::streamable::int_or_none(image.get(key));
                if !value.is_null() {
                    thumbnail.insert(key.into(), value);
                }
            }
            Value::Object(thumbnail)
        })
        .collect();
    let mut fields: Vec<(&str, Value)> = vec![
        ("id", Value::from(video_id.as_str())),
        ("title", title.unwrap_or(Value::Null)),
        ("series", series.unwrap_or(Value::Null)),
        ("episode", episode_name.unwrap_or(Value::Null)),
    ];
    let extra: [(&str, Option<Value>); 6] = [
        (
            "description",
            episode
                .get("description")
                .filter(|text| text.is_string())
                .cloned(),
        ),
        (
            "release_timestamp",
            episode
                .get("first_broadcasted_at")
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_timestamp(date, false))
                .map(Value::from),
        ),
        ("categories", names(&episode, "categories")),
        ("tags", names(&episode, "tags")),
        (
            "thumbnails",
            (!thumbnails.is_empty()).then_some(Value::Array(thumbnails)),
        ),
        (
            "webpage_url",
            urljoin(url, episode.get("url")).map(Value::from),
        ),
    ];
    fields.extend(
        extra
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value))),
    );
    fields.push(("extractor_key", VOD_NAME.into()));
    fields.push(("extractor", VOD_NAME.into()));

    // `traverse_obj(episode, (('video', 'audio'), {dict}, any)) or {}`.
    let empty = Value::Object(Map::new());
    let stream = ["video", "audio"]
        .iter()
        .find_map(|key| episode.get(*key).filter(|value| value.is_object()))
        .unwrap_or(&empty);
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    match stream.get("url").filter(|url| super::nuxt::truthy(url)) {
        None => crate::no_formats::raise_expected(
            name,
            None,
            Some(&episode_id),
            "Stream not found; it has most likely expired",
        )?,
        Some(stream_url) => {
            let stream_url = stream_url
                .as_str()
                .ok_or_else(|| unsupported("stream URLs that are not strings"))?;
            if video {
                let manifest = Url::parse(stream_url)?;
                let stream_info = super::extract_m3u8_fatal_master(
                    client, &manifest, name, &video_id, None, true,
                )?;
                formats = stream_info.formats;
                subtitles = stream_info.subtitles;
                fields.push(("formats", Value::Null));
                fields.push(("subtitles", Value::Null));
                let duration = super::streamable::int_or_none(stream.get("duration"));
                if !duration.is_null() {
                    fields.push(("duration", duration));
                }
                if let Some(timestamp) = stream
                    .get("published_at")
                    .and_then(Value::as_str)
                    .and_then(|date| crate::dates::unified_timestamp(date, false))
                {
                    fields.push(("timestamp", timestamp.into()));
                }
            } else {
                // `remove_end(stream_url, '.m4a')` under `vod-stream.nhk.jp`.
                let path = stream_url.strip_suffix(".m4a").unwrap_or(stream_url);
                let manifest = Url::parse("https://vod-stream.nhk.jp")?.join(path)?;
                let manifest = Url::parse(&format!("{manifest}/index.m3u8"))?;
                if let Some(stream_info) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    name,
                    &episode_id,
                    Some("hls"),
                    false,
                    false,
                ) {
                    formats = stream_info.formats;
                }
                for format in &mut formats {
                    // `_extract_m3u8_formats(..., 'm4a', ...)`.
                    format.ext = "m4a".into();
                    format.language = Some(lang.clone());
                    crate::info_order::mark_format_update(format, "language");
                }
                fields.push(("formats", Value::Null));
            }
        }
    }
    let mut info = dict_info(url, (VOD_NAME, VOD_NAME), fields, formats);
    info.subtitles = subtitles;
    if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
        shape.push_str(";thumbnail=url,width,height");
    }
    Ok(info)
}

impl Extractor for NhkVod {
    fn key(&self) -> &'static str {
        VOD_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !vod_suitable(url.as_str()) {
            return Ok(None);
        }
        announce(VOD_NAME, url);
        episode_info(client, VOD_NAME, url, None).map(Some)
    }
}

/// `_extract_meta_from_class_elements(class_values, html)`.
fn class_meta(classes: &[&str], html: &str) -> Result<Option<String>> {
    for class in classes {
        let elements = crate::html_elements::elements_text_and_html_by_class(class, html)
            .map_err(|error| unsupported(&format!("pages whose HTML does not parse ({error})")))?;
        if let Some((text, _)) = elements.into_iter().next() {
            let value = crate::html::clean_html(&text);
            if !value.is_empty() {
                return Ok(Some(value));
            }
        }
    }
    Ok(None)
}

impl Extractor for NhkVodProgram {
    fn key(&self) -> &'static str {
        PROGRAM_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !program_suitable(url.as_str()) {
            return Ok(None);
        }
        let found = PROGRAM_URL.captures(url.as_str()).expect("suitable");
        let lang = found["lang"].to_owned();
        let program_id = found["id"].to_owned();
        let audio = found.name("type").is_some();
        let clip = found.name("episode_type").map(|kind| kind.as_str()) == Some("clip");
        announce(PROGRAM_NAME, url);
        let episodes = call_api(
            client,
            PROGRAM_NAME,
            &program_id,
            &lang,
            (!audio, false, clip),
        )?;
        // `('items', lambda _, v: v['url'])`: queued for the generator.
        let items: Vec<Value> = episodes
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.get("url").is_some_and(super::nuxt::truthy))
            .cloned()
            .collect();
        let html = download_webpage(client, url, PROGRAM_NAME, &program_id)?;
        let title = class_meta(
            &[
                "p-programDetail__title",
                "pProgramHero__logoText",
                "tAudioProgramMain__title",
                "p-program-name",
            ],
            &html,
        )?;
        let description = class_meta(
            &[
                "p-programDetail__text",
                "pProgramHero__description",
                "tAudioProgramMain__info",
                "p-program-description",
            ],
            &html,
        )?;
        // `playlist_result(entries(), program_id, title, description,
        // series=title)`: `series`, then `id`, `title` (when truthy) and
        // `description` (when not `None`).
        let mut keys = vec!["series", "id"];
        let mut extra = Map::new();
        extra.insert(
            "series".into(),
            title.clone().map_or(Value::Null, Value::from),
        );
        if title.is_some() {
            keys.push("title");
        }
        if let Some(description) = &description {
            keys.push("description");
            extra.insert("description".into(), description.clone().into());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;{}{}",
                keys.join(","),
                crate::info_order::GENERATOR
            )
            .into(),
        );
        let state = json!({"kind": PROGRAM_NAME, "url": url.as_str(), "items": items});
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: program_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: PROGRAM_NAME.into(),
            extractor_key: PROGRAM_NAME.into(),
            nested: Vec::new(),
            pending: Some(crate::PendingEntries { state }),
            entries: Vec::new(),
            extra,
        })))
    }
}

/// Whether a playlist's pending state is an NHK program's episodes.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == PROGRAM_NAME
}

/// The episodes generator: each item's `_extract_episode_info`, up to
/// `limit` entries.
pub(crate) fn fetch_episodes(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let Value::Object(mut state) = state else {
        return Ok(None);
    };
    let page = Url::parse(state["url"].as_str().unwrap_or_default())?;
    let mut items: std::collections::VecDeque<Value> = match state.shift_remove("items") {
        Some(Value::Array(items)) => items.into(),
        _ => Default::default(),
    };
    while let Some(item) = items.pop_front() {
        if limit.is_some_and(|limit| entries.len() >= limit) {
            items.push_front(item);
            break;
        }
        let episode_url = urljoin(&page, item.get("url"))
            .ok_or_else(|| unsupported("episode URLs that are not URLs"))?;
        let episode_url = Url::parse(&episode_url)?;
        entries.push(episode_info(
            client,
            PROGRAM_NAME,
            &episode_url,
            Some(item),
        )?);
    }
    if items.is_empty() {
        return Ok(None);
    }
    state.insert("items".into(), Value::Array(items.into()));
    Ok(Some(Value::Object(state)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert_eq!(
            vod_match("https://www3.nhk.or.jp/nhkworld/en/shows/2049165/"),
            Some(("en".into(), None, "2049165".into()))
        );
        assert!(vod_suitable(
            "https://www3.nhk.or.jp/nhkworld/en/shows/audio/livinginjapan-20240901-1/"
        ));
        assert!(vod_suitable(
            "https://www3.nhk.or.jp/nhkworld/en/ondemand/video/2049126/"
        ));
        assert!(program_suitable(
            "https://www3.nhk.or.jp/nhkworld/en/shows/sumo/"
        ));
        assert!(program_suitable(
            "https://www3.nhk.or.jp/nhkworld/en/shows/japanrailway/?type=clip"
        ));
        assert!(!program_suitable(
            "https://www3.nhk.or.jp/nhkworld/en/shows/2049165/"
        ));
    }
}
