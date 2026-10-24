//! Upstream `IHeartRadioIE` and `IHeartRadioPodcastIE`
//! (`yt_dlp/extractor/iheart.py`): the `api.iheart.com` v3 podcast episode
//! JSON (its cleaned media URL as the only format) and a podcast's episode
//! list as `iheartradio:ID` entries.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct IHeartRadio;
pub struct IHeartRadioPodcast;

const NAME: &str = "IHeartRadio";
const IE_NAME: &str = "iheartradio";
const PODCAST_NAME: &str = "IHeartRadioPodcast";
const PODCAST_IE_NAME: &str = "iheartradio:podcast";
const API: &str = "https://api.iheart.com/api/v3/podcast/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:https?://(?:www\.)?iheart\.com/podcast/[^/]+/episode/(?P<display_id>[^/?&#]+)-|iheartradio:)(?P<id>\d+)").unwrap()
});

static PODCAST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?iheart(?:podcastnetwork)?\.com/podcast/[^/?&#]+-(?P<id>\d+)/?(?:[?#&]|$)",
    )
    .unwrap()
});

/// `IHeartRadioIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `IHeartRadioPodcastIE._match_valid_url`.
pub fn podcast_suitable(url: &str) -> bool {
    PODCAST_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("iHeartRadio {what} are not supported natively"))
}

/// `int_or_none(value, scale)`.
fn int_or_none(value: Option<&Value>, scale: i64) -> Option<i64> {
    let value = match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))?,
        Value::String(text) => text.trim().parse().ok()?,
        Value::Bool(flag) => i64::from(*flag),
        _ => return None,
    };
    Some(value.div_euclid(scale))
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.into()),
        _ => None,
    }
}

/// `_extract_episode`: thumbnail, description, timestamp and duration.
fn episode_fields(episode: &Map<String, Value>) -> Result<Vec<(&'static str, Value)>> {
    let description = match episode.get("description") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(text)) => Value::String(crate::html::clean_html(text)),
        Some(_) => return Err(unsupported("descriptions that are not strings")),
    };
    Ok(vec![
        (
            "thumbnail",
            episode.get("imageUrl").cloned().unwrap_or(Value::Null),
        ),
        ("description", description),
        (
            "timestamp",
            int_or_none(episode.get("startDate"), 1000).map_or(Value::Null, Value::from),
        ),
        (
            "duration",
            int_or_none(episode.get("duration"), 1).map_or(Value::Null, Value::from),
        ),
    ])
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for IHeartRadio {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let episode_id = found["id"].to_owned();
        start(IE_NAME, url);
        let api = Url::parse(&format!("{API}episodes/{episode_id}"))?;
        let data = download_json(client, &api, IE_NAME, &episode_id)?;
        let episode = data
            .get("episode")
            .and_then(Value::as_object)
            .ok_or_else(|| unsupported("episode responses without an episode"))?;
        let fields = episode_fields(episode)?;
        let title = match episode.get("title") {
            Some(Value::String(title)) => title.clone(),
            _ => return Err(unsupported("episodes without a string title")),
        };
        let media_url = match episode.get("mediaUrl") {
            Some(Value::String(media)) => super::playerfm::clean_podcast_url(media),
            _ => return Err(unsupported("episodes without a string media URL")),
        };
        let media =
            Url::parse(&media_url).map_err(|_| unsupported("media URLs that are not absolute"))?;
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.id = episode_id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = IE_NAME.into();
        info.extractor_key = NAME.into();
        let mut keys = Vec::new();
        for (key, value) in fields {
            match (key, value) {
                ("thumbnail", Value::String(thumbnail)) => {
                    info.metadata.thumbnail = Some(thumbnail);
                }
                ("thumbnail", Value::Null) => {
                    info.extra.insert("thumbnail".into(), Value::Null);
                }
                ("thumbnail", _) => return Err(unsupported("thumbnails that are not strings")),
                // `duration: None` is dropped.
                ("duration", Value::Null) => continue,
                (key, value) => crate::metadata_parser::set_field(&mut info, key, value),
            }
            keys.push(key);
        }
        keys.extend(["id", "title", "url"]);
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("untyped-dict;{}", keys.join(",")),
        )))
    }
}

/// A podcast's `_type: url` episode entry (`iheartradio:ID`).
fn podcast_entry(episode: &Map<String, Value>, episode_id: &str) -> Result<VideoInfo> {
    let target = format!("iheartradio:{episode_id}");
    let mut entry = Map::new();
    for (key, value) in episode_fields(episode)? {
        entry.insert(key.into(), value);
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("id".into(), episode_id.into());
    entry.insert(
        "title".into(),
        episode.get("title").cloned().unwrap_or(Value::Null),
    );
    entry.insert("url".into(), target.clone().into());
    entry.insert("ie_key".into(), NAME.into());
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = episode_id.to_owned();
    info.title = entry
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": IE_NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for IHeartRadioPodcast {
    fn key(&self) -> &'static str {
        PODCAST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PODCAST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let podcast_id = found["id"].to_owned();
        start(PODCAST_IE_NAME, url);
        let api = Url::parse(&format!(
            "{API}podcasts/{podcast_id}/episodes?limit=1000000000"
        ))?;
        let data = download_json(client, &api, PODCAST_IE_NAME, &podcast_id)?;
        let episodes = data
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| unsupported("episode lists without data"))?;
        let mut entries = Vec::new();
        for episode in episodes {
            let episode = episode
                .as_object()
                .ok_or_else(|| unsupported("episodes that are not objects"))?;
            let Some(episode_id) = str_or_none(episode.get("id")).filter(|id| !id.is_empty())
            else {
                continue;
            };
            entries.push(podcast_entry(episode, &episode_id)?);
        }
        // `_call_api(path, podcast_id, False) or {}`.
        let api = Url::parse(&format!("{API}podcasts/{podcast_id}"))?;
        let podcast = match download_json(client, &api, PODCAST_IE_NAME, &podcast_id) {
            Ok(podcast) => podcast,
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{PODCAST_IE_NAME}] {podcast_id}: Unable to download JSON metadata: {}",
                    crate::upstream_error_text(&source)
                ));
                Value::Null
            }
            Err(Error::Reported(message)) if message.contains(": Failed to parse JSON ") => {
                crate::screen::warning(&message);
                Value::Null
            }
            Err(error) => return Err(error),
        };
        let title = match podcast.get("title") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => Some(title.clone()).filter(|title| !title.is_empty()),
            Some(_) => return Err(unsupported("podcast titles that are not strings")),
        };
        let mut extra = Map::new();
        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        } else {
            extra.insert("title".into(), Value::Null);
        }
        if let Some(description) = podcast.get("description").filter(|value| !value.is_null()) {
            extra.insert("description".into(), description.clone());
            keys.push("description");
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: podcast_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: PODCAST_IE_NAME.into(),
            extractor_key: PODCAST_NAME.into(),
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
            "https://www.iheart.com/podcast/105-behind-the-bastards-29236323/episode/part-one-alexander-lukashenko-the-dictator-70346499/?embed=true"
        ));
        assert!(suitable("iheartradio:70346499"));
        assert!(!suitable(
            "https://www.iheart.com/podcast/105-behind-the-bastards-29236323/"
        ));
        assert!(podcast_suitable(
            "https://www.iheart.com/podcast/105-behind-the-bastards-29236323/"
        ));
        assert!(podcast_suitable(
            "https://www.iheartpodcastnetwork.com/podcast/x-555"
        ));
        assert!(!podcast_suitable(
            "https://www.iheart.com/podcast/105-behind-the-bastards-29236323/episode/x-1/"
        ));
    }
}
