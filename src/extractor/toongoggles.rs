//! Upstream `ToonGogglesIE` (`yt_dlp/extractor/toongoggles.py`): Toon Goggles
//! episodes as transparent Kaltura `url` results with the API's episode
//! metadata, and shows as playlists of them.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct ToonGoggles;

const NAME: &str = "ToonGoggles";
const KALTURA: &str = "Kaltura";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?toongoggles\.com/shows/(?P<show_id>\d+)(?:/[^/]+/episodes/(?P<episode_id>\d+))?",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`: the `id` group, which this URL pattern lacks.
fn key_error(key: &str) -> Error {
    Error::Reported(format!(
        "An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Toon Goggles {what} are not supported natively"))
}

/// `_call_api(action, page_id, query)`.
fn call_api(client: &Client, action: &str, page_id: &str, query: &[(&str, &str)]) -> Result<Value> {
    let mut query = query.to_vec();
    query.extend([
        ("for_ng", "1"),
        ("for_web", "1"),
        ("show_meta", "1"),
        ("version", "7.0"),
    ]);
    let api = super::update_url_query(
        &Url::parse(&format!("http://api.toongoggles.com/{action}"))?,
        &query,
    );
    download_json(client, &api, NAME, page_id)
}

fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

/// `_parse_episode_data(episode_data)`: the `url_transparent` dict's
/// fields in their key order, and its Kaltura URL.
fn episode_fields(episode: &Value) -> Result<(Map<String, Value>, String)> {
    let title = episode
        .get("episode_name")
        .cloned()
        .ok_or_else(|| key_error("episode_name"))?;
    let id = episode
        .get("episode_id")
        .cloned()
        .ok_or_else(|| key_error("episode_id"))?;
    let entry_id = match episode.get("entry_id") {
        Some(Value::String(entry_id)) => entry_id.clone(),
        Some(_) => return Err(unsupported("entry IDs that are not strings")),
        None => return Err(key_error("entry_id")),
    };
    let target = format!("kaltura:513551:{entry_id}");
    let get = |key: &str| episode.get(key).cloned().unwrap_or(Value::Null);
    let duration = episode
        .get("hms")
        .and_then(Value::as_str)
        .and_then(crate::parse_duration)
        .map_or(Value::Null, |duration| json!(duration));
    let mut fields = Map::new();
    for (key, value) in [
        ("id", id),
        ("title", title.clone()),
        ("thumbnail", get("thumbnail_url")),
        ("description", get("description")),
        ("duration", duration),
        ("series", get("show_name")),
        ("season_number", int_or_none(episode.get("season_num"))),
        ("episode_id", get("episode_id")),
        ("episode", title),
        ("episode_number", int_or_none(episode.get("episode_num"))),
        ("categories", get("categories")),
    ] {
        fields.insert(key.into(), value);
    }
    Ok((fields, target))
}

/// A show's episode as a flat `url_transparent` entry.
fn episode_entry(episode: &Value) -> Result<VideoInfo> {
    let (fields, target) = episode_fields(episode)?;
    let mut entry = Map::new();
    entry.insert("_type".into(), "url_transparent".into());
    for (key, value) in &fields {
        entry.insert(key.clone(), value.clone());
        if key == "title" {
            entry.insert("url".into(), target.clone().into());
        }
    }
    entry.insert("ie_key".into(), KALTURA.into());
    let url = super::target_url(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = match &fields["id"] {
        Value::String(id) => id.clone(),
        Value::Number(id) => id.to_string(),
        _ => String::new(),
    };
    let present: Map<String, Value> = fields
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .collect();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": Value::Object(present),
            "ie_key": KALTURA,
            "extractor": KALTURA,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for ToonGoggles {
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
        let show_id = found["show_id"].to_owned();
        let episode_id = found.name("episode_id").map(|id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        if let Some(episode_id) = episode_id {
            let data = call_api(
                client,
                "search",
                &episode_id,
                &[("filter", "episode"), ("id", episode_id.as_str())],
            )?;
            let episode = match data.get("objects") {
                // An uncaught `IndexError` is reported as its message.
                Some(Value::Array(objects)) => objects
                    .first()
                    .ok_or_else(|| Error::Reported("list index out of range".into()))?,
                Some(_) => return Err(unsupported("episode lists that are not lists")),
                None => return Err(key_error("objects")),
            };
            let (fields, target) = episode_fields(episode)?;
            let present: Map<String, Value> = fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .collect();
            let mut follow = super::direct::build_at(url.clone(), url, String::new());
            follow.formats.clear();
            follow.direct = None;
            follow.extra.insert(
                crate::FOLLOW_TRANSPARENT.into(),
                json!({"url": target, "fields": Value::Object(present)}),
            );
            return Ok(Some(ExtractResult::Video(follow)));
        }
        let data = call_api(
            client,
            "getepisodesbyshow",
            &show_id,
            &[("max", "1000000000"), ("showid", show_id.as_str())],
        )?;
        let episodes = match data.get("objects") {
            None => Vec::new(),
            Some(Value::Array(objects)) => objects.clone(),
            Some(_) => return Err(unsupported("episode lists that are not lists")),
        };
        let entries = episodes
            .iter()
            .map(episode_entry)
            .collect::<Result<Vec<_>>>()?;
        let title = match data.get("show_name") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => Some(title.clone()).filter(|title| !title.is_empty()),
            Some(_) => return Err(unsupported("show names that are not strings")),
        };
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            if title.is_some() {
                "playlist-dict;id,title"
            } else {
                "playlist-dict;id"
            }
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: show_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
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
            "http://www.toongoggles.com/shows/217143/bernard-season-2/episodes/217147/football"
        ));
        assert!(suitable(
            "http://www.toongoggles.com/shows/227759/om-nom-stories-around-the-world"
        ));
        assert!(!suitable("http://www.toongoggles.com/movies/1"));
    }
}
