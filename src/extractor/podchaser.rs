//! Upstream `PodchaserIE` (`yt_dlp/extractor/podchaser.py`): an episode's
//! podcast and `player_ids` API JSON as an untyped audio result, and a
//! podcast's paged `list/episode` API episodes as a playlist of them.
use super::{Extractor, download_json, post_json_note_headers};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Podchaser;

const NAME: &str = "Podchaser";
const PAGE_SIZE: usize = 100;

/// An info dict's fields in order.
type Fields = Vec<(&'static str, Value)>;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?podchaser\.com/podcasts/[\w-]+-(?P<podcast_id>\d+)(?:/episodes/[\w-]+-(?P<id>\d+))?",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Podchaser {what} are not supported natively"))
}

/// `(value, {int}, {str_or_none})`: integers only.
fn int_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_number)
        .filter(|number| number.is_i64() || number.is_u64())
        .map(ToString::to_string)
}

fn string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

/// `slug[:30].rstrip('-')`.
fn short_slug(slug: &str) -> String {
    slug.chars()
        .take(30)
        .collect::<String>()
        .trim_end_matches('-')
        .to_owned()
}

/// `_parse_episode(episode, podcast)`: the info dict's fields in order (the
/// audio URL, if any, separately).
fn parse_episode(
    episode: &Value,
    podcast: &Value,
    temp_id: &str,
) -> Result<(Fields, Option<String>)> {
    let id = int_str(episode.get("id")).ok_or_else(|| {
        let prefix = if temp_id.is_empty() {
            format!("[{NAME}]")
        } else {
            format!("[{NAME}] {temp_id}:")
        };
        Error::Reported(format!(
            "{prefix} Unable to extract episode ID; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    let mut fields: Fields = vec![("id", id.clone().into())];
    let mut push = |key: &'static str, value: Option<Value>| {
        if let Some(value) = value.filter(|value| !value.is_null()) {
            fields.push((key, value));
        }
    };
    push("title", string(episode.get("title")).map(Value::from));
    push(
        "description",
        string(episode.get("description")).map(Value::from),
    );
    let audio = episode
        .get("audio_url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none);
    push("url", audio.clone().map(Value::from));
    push(
        "thumbnail",
        episode
            .get("image_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map(Value::from),
    );
    push(
        "duration",
        Some(super::streamable::int_or_none(episode.get("length"))),
    );
    push(
        "timestamp",
        episode
            .get("air_date")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, true))
            .map(Value::from),
    );
    push(
        "average_rating",
        Some(super::clyp::float_or_none(episode.get("rating"))),
    );
    push("series", string(podcast.get("title")).map(Value::from));
    let series_id = int_str(podcast.get("id"));
    push("series_id", series_id.clone().map(Value::from));
    // `(('summary', None), 'categories', ..., 'text', {str}, filter, all,
    // {orderedSet})`: always a list.
    let mut categories: Vec<Value> = Vec::new();
    for source in [podcast.get("summary"), Some(podcast)] {
        let Some(Value::Array(items)) = source.and_then(|source| source.get("categories")) else {
            continue;
        };
        for item in items {
            if let Some(text) = item.get("text").and_then(Value::as_str)
                && !text.is_empty()
                && !categories.iter().any(|seen| seen == text)
            {
                categories.push(text.into());
            }
        }
    }
    push("categories", Some(Value::Array(categories)));
    let tags: Vec<Value> = match podcast.get("tags") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").filter(|text| text.is_string()).cloned())
            .collect(),
        _ => Vec::new(),
    };
    if !tags.is_empty() {
        push("tags", Some(Value::Array(tags)));
    }
    push("vcodec", Some("none".into()));
    if let Some(series_id) = series_id {
        let podcast_slug = string(podcast.get("slug")).unwrap_or_default();
        let podcast_slug = if podcast_slug.is_empty() {
            "podcast".to_owned()
        } else {
            podcast_slug
        };
        let episode_slug = string(episode.get("slug")).unwrap_or_default();
        let episode_slug = if episode_slug.is_empty() {
            "episode".to_owned()
        } else {
            episode_slug
        };
        push(
            "webpage_url",
            Some(
                format!(
                    "https://www.podchaser.com/podcasts/{}-{series_id}/{}-{id}",
                    short_slug(&podcast_slug),
                    short_slug(&episode_slug)
                )
                .into(),
            ),
        );
    }
    Ok((fields, audio))
}

/// The episode's untyped info dict with its single `url` format.
fn episode_info(episode: &Value, podcast: &Value, page: &Url, temp_id: &str) -> Result<VideoInfo> {
    let (fields, audio) = parse_episode(episode, podcast, temp_id)?;
    let mut formats = Vec::new();
    if let Some(audio) = audio {
        let ext = super::direct::determine_ext(&audio)
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        let mut format: crate::VideoFormat = serde_json::from_value(
            json!({"format_id": "", "url": audio, "ext": ext, "vcodec": "none"}),
        )
        .map_err(|error| Error::Reported(format!("[{NAME}] {temp_id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "url-only");
        formats.push(format);
    }
    let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
    let titled = keys.contains(&"title");
    let shape = format!(
        "untyped-dict;{}{}",
        keys.join(","),
        if titled { "" } else { "-untitled" }
    );
    Ok(crate::info_order::mark_info(
        super::dict_info(page, (NAME, NAME), fields, formats),
        &shape,
    ))
}

fn call_api(client: &Client, path: &str, id: &str) -> Result<Value> {
    download_json(
        client,
        &Url::parse(&format!("https://api.podchaser.com/{path}"))?,
        NAME,
        id,
    )
}

/// `_fetch_page(podcast_id, podcast, page)`: the page's episode dicts.
fn fetch_page(
    client: &Client,
    podcast_id: &str,
    podcast: &Value,
    page: &Url,
    pagenum: usize,
) -> Result<Vec<VideoInfo>> {
    // `json.dumps` with its default separators.
    let body = format!(
        "{{\"start\": {}, \"count\": {PAGE_SIZE}, \"sort_order\": \"SORT_ORDER_RECENT\", \"filters\": {{\"podcast_id\": {}}}, \"options\": {{}}}}",
        pagenum * PAGE_SIZE,
        Value::from(podcast_id)
    );
    let response = post_json_note_headers(
        client,
        &Url::parse("https://api.podchaser.com/list/episode")?,
        NAME,
        podcast_id,
        "Downloading JSON metadata",
        &[("Content-Type", "application/json;charset=utf-8")],
        body,
    )?;
    let entities = match response.get("entities") {
        Some(Value::Array(entities)) => entities,
        Some(_) => return Err(unsupported("episode lists that are not lists")),
        None => return Err(super::clyp::key_error(podcast_id, "entities")),
    };
    entities
        .iter()
        .map(|episode| episode_info(episode, podcast, page, ""))
        .collect()
}

/// Whether a playlist's pending state is a Podchaser podcast's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == "podchaser"
}

/// `OnDemandPagedList(functools.partial(self._fetch_page, ...), 100)` read
/// by upstream's `PlaylistEntries` (see [`super::paged`]).
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let podcast_id = state["podcast_id"].as_str().unwrap_or_default().to_owned();
    let page = Url::parse(state["url"].as_str().unwrap_or_default())?;
    let podcast = state["podcast"].clone();
    let mut fetch = |pagenum: usize| -> Result<Vec<VideoInfo>> {
        fetch_page(client, &podcast_id, &podcast, &page, pagenum)
    };
    let placeholder = || Ok(super::direct::build_at(page.clone(), &page, "mp3".into()));
    super::paged::fetch_requested(state, limit, entries, PAGE_SIZE, &mut fetch, &placeholder)
}

impl Extractor for Podchaser {
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
        let podcast_id = found["podcast_id"].to_owned();
        let episode_id = found.name("id").map(|id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let podcast = call_api(
            client,
            &format!("podcasts/{podcast_id}"),
            episode_id.as_deref().unwrap_or(&podcast_id),
        )?;
        let Some(episode_id) = episode_id else {
            let get = |key: &str| podcast.get(key).filter(|value| !value.is_null());
            // `str_or_none(podcast.get('id'))`.
            let id = match get("id") {
                None => None,
                Some(Value::String(id)) => Some(id.clone()),
                Some(Value::Number(id)) => Some(id.to_string()),
                Some(_) => return Err(unsupported("podcast IDs that are not scalars")),
            };
            let title = match get("title") {
                None => None,
                Some(Value::String(title)) => Some(title.clone()),
                Some(_) => return Err(unsupported("podcast titles that are not strings")),
            };
            let description = get("description").cloned();
            // `playlist_result`: a falsy ID or title is left out, a `None`
            // description too.
            let mut keys = Vec::new();
            if id.as_ref().is_some_and(|id| !id.is_empty()) {
                keys.push("id");
            }
            if title.as_ref().is_some_and(|title| !title.is_empty()) {
                keys.push("title");
            }
            let mut extra = Map::new();
            if let Some(description) = description {
                if !description.is_string() {
                    return Err(unsupported("podcast descriptions that are not strings"));
                }
                keys.push("description");
                extra.insert("description".into(), description);
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
            let state = json!({
                "kind": "podchaser",
                "podcast_id": podcast_id,
                "url": url.as_str(),
                "podcast": podcast,
                "random_access": true,
            });
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id: id.unwrap_or_default(),
                title: title.unwrap_or_default(),
                webpage_url: url.clone(),
                extractor: NAME.into(),
                extractor_key: NAME.into(),
                nested: Vec::new(),
                pending: Some(crate::PendingEntries { state }),
                entries: Vec::new(),
                extra,
            })));
        };
        let episode = call_api(
            client,
            &format!("podcasts/{podcast_id}/episodes/{episode_id}/player_ids"),
            &episode_id,
        )?;
        Ok(Some(ExtractResult::Video(episode_info(
            &episode,
            &podcast,
            url,
            &episode_id,
        )?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.podchaser.com/podcasts/cum-town-36924/episodes/ep-285-freeze-me-off-104365585"
        ));
        assert!(suitable(
            "https://www.podchaser.com/podcasts/the-bone-zone-28853"
        ));
        assert!(suitable(
            "https://www.podchaser.com/podcasts/sean-carrolls-mindscape-scienc-699349/episodes"
        ));
        assert!(!suitable("https://www.podchaser.com/podcasts/"));
    }

    #[test]
    fn slugs_are_cut_like_python() {
        assert_eq!(
            short_slug("sean-carrolls-mindscape-science-society"),
            "sean-carrolls-mindscape-scienc"
        );
        assert_eq!(
            short_slug("abcdefghijklmnopqrstuvwxyz-abc-d"),
            "abcdefghijklmnopqrstuvwxyz-abc"
        );
    }
}
