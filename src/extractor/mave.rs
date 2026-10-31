//! Upstream `MaveIE` and `MaveChannelIE` (`yt_dlp/extractor/mave.py`): a
//! podcast episode's and a channel's metadata from the Mave website API,
//! with each episode's audio file from the storage host.
use super::{Extractor, dict_info, direct, download_json_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Mave;
pub struct MaveChannel;

const NAME: &str = "mave";
const KEY: &str = "Mave";
const CHANNEL_NAME: &str = "mave:channel";
const CHANNEL_KEY: &str = "MaveChannel";
const API_BASE_URL: &str = "https://api.mave.digital/v1/website";
const API_BASE_STORAGE_URL: &str = "https://store.cloud.mts.ru/mave/";
const PAGE_SIZE: usize = 50;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?P<channel_id>[\w-]+)\.mave\.digital/ep-(?P<episode_code>\d+)",
    )
    .unwrap()
});
static CHANNEL_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?P<id>[\w-]+)\.mave\.digital/?(?:$|[?#])").unwrap());

/// `_match_valid_url` for the episode IE.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_load_channel_meta`: the `podcast` object.
fn load_channel_meta(client: &Client, channel_id: &str, display_id: &str) -> Result<Value> {
    let url = Url::parse(&format!("{API_BASE_URL}/{channel_id}/"))?;
    let meta = download_json_note(
        client,
        &url,
        NAME,
        display_id,
        "Downloading channel metadata",
    )?;
    Ok(meta.get("podcast").cloned().unwrap_or(Value::Null))
}

/// `_load_episode_meta`.
fn load_episode_meta(
    client: &Client,
    channel_id: &str,
    episode_code: &str,
    display_id: &str,
) -> Result<Value> {
    let url = Url::parse(&format!(
        "{API_BASE_URL}/{channel_id}/episodes/{episode_code}"
    ))?;
    download_json_note(
        client,
        &url,
        NAME,
        display_id,
        "Downloading episode metadata",
    )
}

/// `traverse_obj(value, key, {str})`.
fn text(value: &Value, key: &str) -> Option<Value> {
    value.get(key).filter(|value| value.is_string()).cloned()
}

/// `traverse_obj(value, key, {int_or_none})`.
fn int(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Number(number) if number.is_i64() => Some(Value::Number(number.clone())),
        Value::Number(number) => number.as_f64().map(|float| Value::from(float as i64)),
        Value::String(text) => text.trim().parse::<i64>().ok().map(Value::from),
        _ => None,
    }
}

/// `urljoin(API_BASE_STORAGE_URL)(path)`.
fn storage_url(path: Option<&Value>) -> Option<Value> {
    let path = path?.as_str()?;
    if path.is_empty() {
        return None;
    }
    let base = Url::parse(API_BASE_STORAGE_URL).ok()?;
    base.join(path).ok().map(|url| Value::from(url.as_str()))
}

/// `_create_entry`: the episode's info dict fields, in upstream's order.
fn entry_fields(
    channel_id: &str,
    channel_meta: &Value,
    episode_meta: &Value,
) -> Result<(String, Vec<(&'static str, Value)>)> {
    let code = episode_meta
        .get("code")
        .filter(|code| code.is_i64() || code.is_u64())
        .cloned()
        .ok_or_else(|| {
            Error::UnsupportedHtml5(
                "Mave episodes without an integer code are not supported natively".into(),
            )
        })?;
    let episode_code = code.to_string();
    let display_id = format!("{channel_id}-{episode_code}");
    let webpage_url = format!("https://{channel_id}.mave.digital/ep-{episode_code}");
    let reactions = episode_meta
        .get("reactions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let reaction_count = |kind: &str| {
        reactions
            .iter()
            .find(|reaction| reaction.get("type").and_then(Value::as_str) == Some(kind))
            .and_then(|reaction| int(reaction.get("count")))
    };
    let age_limit = episode_meta
        .get("is_explicit")
        .and_then(Value::as_bool)
        .filter(|explicit| *explicit)
        .map(|_| Value::from(18));
    let timestamp = episode_meta
        .get("publish_date")
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map(Value::from);
    let description = episode_meta
        .get("description")
        .and_then(Value::as_str)
        .map(|text| Value::from(crate::html::clean_html(text)));
    let fields: Vec<(&str, Option<Value>)> = vec![
        ("display_id", Some(display_id.clone().into())),
        ("extractor_key", Some(KEY.into())),
        ("extractor", Some(NAME.into())),
        ("webpage_url", Some(webpage_url.into())),
        ("channel_id", Some(channel_id.into())),
        (
            "channel_url",
            Some(format!("https://{channel_id}.mave.digital/").into()),
        ),
        ("vcodec", Some("none".into())),
        ("id", text(episode_meta, "id")),
        ("url", storage_url(episode_meta.get("audio"))),
        ("title", text(episode_meta, "title")),
        ("description", description),
        ("thumbnail", storage_url(episode_meta.get("image"))),
        ("duration", int(episode_meta.get("duration"))),
        ("season_number", int(episode_meta.get("season"))),
        ("episode_number", int(episode_meta.get("number"))),
        ("view_count", int(episode_meta.get("listenings"))),
        ("like_count", reaction_count("like")),
        ("dislike_count", reaction_count("dislike")),
        ("age_limit", age_limit),
        ("timestamp", timestamp),
        ("series_id", text(channel_meta, "id")),
        ("series", text(channel_meta, "title")),
        ("channel", text(channel_meta, "title")),
        ("uploader", text(channel_meta, "author")),
    ];
    // `traverse_obj` drops the `None` values.
    let fields = fields
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect();
    Ok((display_id, fields))
}

/// The episode as an info dict on its own webpage: its `url` is upstream's
/// bare `url` result, a url-only info.
fn entry_info(channel_id: &str, channel_meta: &Value, episode_meta: &Value) -> Result<VideoInfo> {
    let (_, fields) = entry_fields(channel_id, channel_meta, episode_meta)?;
    let webpage_url = fields
        .iter()
        .find(|(key, _)| *key == "webpage_url")
        .and_then(|(_, value)| value.as_str())
        .unwrap_or_default();
    let page = Url::parse(webpage_url)?;
    let audio = fields.iter().find_map(|(key, value)| match (*key, value) {
        ("url", Value::String(audio)) => Some(audio.clone()),
        _ => None,
    });
    match audio {
        Some(audio) => Ok(url_info(&page, &Url::parse(&audio)?, fields)),
        None => Ok(dict_info(&page, (NAME, KEY), fields, Vec::new())),
    }
}

/// Upstream's bare `url` result: the audio URL is the one format, and the
/// dict's keys keep `fields`' order with no `formats` key.
fn url_info(page: &Url, media: &Url, fields: Vec<(&str, Value)>) -> VideoInfo {
    // `determine_ext` lowercases the extension.
    let ext = direct::determine_ext(media.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    // The dict's own `vcodec` (a `none` audio track) also marks the format.
    let audio_only = fields
        .iter()
        .any(|(key, value)| *key == "vcodec" && value.as_str() == Some("none"));
    let keys: Vec<String> = fields.iter().map(|(key, _)| (*key).to_owned()).collect();
    let mut formats =
        direct::with_format(direct::build_at(media.clone(), media, ext.clone()), "").formats;
    for format in &mut formats {
        format.ext = ext.clone();
        format.format_id = String::new();
        if audio_only {
            format.vcodec = Some("none".into());
        }
    }
    let mut info = dict_info(page, (NAME, KEY), fields, formats);
    info.format_id = None;
    crate::info_order::mark_info(info, &format!("untyped-dict;{}", keys.join(",")))
}

impl Extractor for Mave {
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
        let channel_id = found["channel_id"].to_owned();
        let episode_code = found["episode_code"].to_owned();
        let display_id = format!("{channel_id}-{episode_code}");
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let channel_meta = load_channel_meta(client, &channel_id, &display_id)?;
        let episode_meta = load_episode_meta(client, &channel_id, &episode_code, &display_id)?;
        let info = entry_info(&channel_id, &channel_meta, &episode_meta)?;
        Ok(Some(ExtractResult::Video(info)))
    }
}

impl Extractor for MaveChannel {
    fn key(&self) -> &'static str {
        CHANNEL_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CHANNEL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let channel_id = found["id"].to_owned();
        crate::screen::set_extractor(CHANNEL_NAME);
        crate::screen::screen(&format!(
            "[{CHANNEL_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let channel_meta = load_channel_meta(client, &channel_id, &channel_id)?;
        let episodes_count = channel_meta
            .get("episodes_count")
            .and_then(Value::as_u64)
            .ok_or_else(|| super::clyp::key_error(&channel_id, "episodes_count"))?;
        let page_count = episodes_count.div_ceil(PAGE_SIZE as u64) as usize;

        // `InAdvancePagedList`: the pages are read in order and the list
        // ends at the first page with fewer than `PAGE_SIZE` entries.
        let mut entries = Vec::new();
        for page_num in 0..page_count {
            let mut page_url = Url::parse(&format!("{API_BASE_URL}/{channel_id}/episodes"))?;
            page_url = super::update_url_query(
                &page_url,
                &[
                    ("view", "all"),
                    ("page", &(page_num + 1).to_string()),
                    ("sort", "newest"),
                    ("format", "all"),
                ],
            );
            let page_data = download_json_note(
                client,
                &page_url,
                CHANNEL_NAME,
                &channel_id,
                &format!("Downloading page {}", page_num + 1),
            )?;
            let mut page_entries = Vec::new();
            let episodes = page_data
                .get("episodes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for episode in episodes {
                let truthy = |key: &str| episode.get(key).is_some_and(super::nuxt::truthy);
                if truthy("audio") && truthy("id") {
                    page_entries.push(entry_info(&channel_id, &channel_meta, &episode)?);
                }
            }
            let short = page_entries.len() < PAGE_SIZE;
            entries.extend(page_entries);
            if short {
                break;
            }
        }

        let mut extra = Map::new();
        // `{'_type': 'playlist', 'id', **title, **description, 'entries'}`.
        extra.insert(crate::info_order::INFO_SHAPE.into(), "rss-playlist".into());
        let title = text(&channel_meta, "title")
            .and_then(|title| title.as_str().map(str::to_owned))
            .unwrap_or_default();
        let description = text(&channel_meta, "description");
        if let Some(description) = description {
            extra.insert("description".into(), description);
        }
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: channel_id,
            title,
            webpage_url: url.clone(),
            extractor: CHANNEL_NAME.into(),
            extractor_key: CHANNEL_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// URL matching for `--list-extractors`.
pub fn channel_suitable(url: &str) -> bool {
    CHANNEL_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://ochenlichnoe.mave.digital/ep-25"));
        assert!(!suitable("https://ochenlichnoe.mave.digital/"));
        assert!(CHANNEL_URL.is_match("https://budem.mave.digital/"));
        assert!(CHANNEL_URL.is_match("https://budem.mave.digital?x=1"));
        assert!(!CHANNEL_URL.is_match("https://budem.mave.digital/ep-3"));
    }
}
