//! Upstream `SimplecastIE`, `SimplecastEpisodeIE` and `SimplecastPodcastIE`
//! (`yt_dlp/extractor/simplecast.py`, through `SimplecastBaseIE`): the
//! `api.simplecast.com` episode JSON (by UUID, or by the episode page URL
//! through the search API) as an untyped audio result, a podcast site's
//! episodes as a generator of the same results, and `SimplecastIE`'s iframe
//! embeds.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Simplecast;
pub struct SimplecastEpisode;
pub struct SimplecastPodcast;

pub(crate) const NAME: &str = "simplecast";
pub(crate) const KEY: &str = "Simplecast";
const EPISODE_NAME: &str = "simplecast:episode";
const EPISODE_KEY: &str = "SimplecastEpisode";
const PODCAST_NAME: &str = "simplecast:podcast";
const PODCAST_KEY: &str = "SimplecastPodcast";

const UUID: &str = r"[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^https?://(?:api\.simplecast\.com/episodes|player\.simplecast\.com)/(?P<id>{UUID})"
    ))
    .unwrap()
});
/// `SimplecastEpisodeIE._VALID_URL` without its `(?!api\.)` look-ahead.
static EPISODE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://([^/]+)\.simplecast\.com/episodes/(?P<id>[^/?&#]+)").unwrap()
});
/// `SimplecastPodcastIE._VALID_URL`'s scheme and host part.
static PODCAST_HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://([^/]+)").unwrap());
static PODCAST_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/episodes/[^/?&#]+").unwrap());
/// `_EMBED_REGEX`.
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"<iframe[^>]+src=["'](?P<url>https?://(?:embed\.simplecast\.com/[0-9a-f]{{8}}|player\.simplecast\.com/{UUID}))"#
    ))
    .unwrap()
});
static SEASON_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"https?://api.simplecast.com/seasons/({UUID})")).unwrap()
});
static CHANNEL_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(https?://[^/]+\.simplecast\.com)").unwrap());

/// `SimplecastIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id`'s ID of a suitable URL.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `SimplecastEpisodeIE._match_valid_url`: the matched text and its ID.
fn episode_match(url: &str) -> Option<(String, String)> {
    let found = EPISODE_URL.captures(url)?;
    // `(?!api\.)` at the host's start.
    if url[found.get(1)?.start()..].starts_with("api.") {
        return None;
    }
    Some((found[0].to_owned(), found["id"].to_owned()))
}

/// `SimplecastEpisodeIE._match_valid_url`.
pub fn episode_suitable(url: &str) -> bool {
    episode_match(url).is_some()
}

/// `SimplecastPodcastIE._match_id`: `(?!(?:api|cdn|embed|feeds|player)\.)`,
/// the greedy `[^/]+` subdomain before `.simplecast.com`, then
/// `(?!/episodes/[^/?&#]+)`.
fn podcast_id(url: &str) -> Option<String> {
    let host = PODCAST_HOST.captures(url)?.get(1)?;
    let subdomain_start = host.start();
    let rest = &url[subdomain_start..];
    if ["api.", "cdn.", "embed.", "feeds.", "player."]
        .iter()
        .any(|prefix| rest.starts_with(prefix))
    {
        return None;
    }
    // Backtracking from the longest subdomain.
    let host_text = host.as_str();
    let mut ends: Vec<usize> = host_text
        .match_indices(".simplecast.com")
        .map(|(index, _)| index)
        .filter(|index| *index > 0)
        .collect();
    ends.reverse();
    for end in ends {
        let after = &rest[end + ".simplecast.com".len()..];
        if !PODCAST_EPISODE.is_match(after) {
            return Some(host_text[..end].to_owned());
        }
    }
    None
}

/// `SimplecastPodcastIE._match_valid_url`.
pub fn podcast_suitable(url: &str) -> bool {
    podcast_id(url).is_some()
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Ok(embed) = url.join(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Simplecast {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    super::lnk::truthy(value)
}

/// `int_or_none`.
fn int_or_none(value: Option<&Value>) -> Value {
    let value = match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        Some(Value::Bool(flag)) => Some(i64::from(*flag)),
        _ => None,
    };
    value.map_or(Value::Null, Value::from)
}

/// `dict.get(key) or {}` of an object (`None` for a falsy value).
fn object<'a>(value: Option<&'a Value>, what: &str) -> Result<Option<&'a Map<String, Value>>> {
    match value {
        Some(Value::Object(map)) => Ok(Some(map)),
        value if truthy(value) => Err(unsupported(&format!("{what} that are not objects"))),
        _ => Ok(None),
    }
}

/// A string-or-`None` field kept as is.
fn text(value: Option<&Value>, what: &str) -> Result<Value> {
    match value {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::String(text)) => Ok(text.as_str().into()),
        Some(_) => Err(unsupported(&format!("{what} that are not strings"))),
    }
}

/// `SimplecastBaseIE._parse_episode`; `error_id` is the extraction's
/// temporary ID (`None` inside the podcast generator, whose `KeyError` is
/// not wrapped in an extractor error).
fn parse_episode(
    episode: &Value,
    page: &Url,
    (extractor, key): (&'static str, &'static str),
    error_id: Option<&str>,
) -> Result<VideoInfo> {
    let episode = episode
        .as_object()
        .ok_or_else(|| unsupported("episodes that are not objects"))?;
    let missing = |name: &str| match error_id {
        Some(id) => key_error(id, name),
        None => Error::Reported(format!("'{name}'")),
    };
    let episode_id = match episode.get("id") {
        Some(Value::String(id)) => id.clone(),
        Some(_) => return Err(unsupported("episode IDs that are not strings")),
        None => return Err(missing("id")),
    };
    // `episode['title'].strip()`.
    let title = match episode.get("title") {
        Some(Value::String(title)) => title.trim().to_owned(),
        Some(_) => return Err(unsupported("episode titles that are not strings")),
        None => return Err(missing("title")),
    };
    let audio_file = object(episode.get("audio_file"), "audio files")?;
    let audio_get = |name: &str| audio_file.and_then(|audio| audio.get(name));
    // `audio_file.get('url') or episode.get('audio_file_url') or
    // episode['enclosure_url']`.
    let audio_url = [audio_get("url"), episode.get("audio_file_url")]
        .into_iter()
        .find(|value| truthy(*value))
        .flatten();
    let audio_url = match audio_url {
        Some(value) => value,
        None => episode
            .get("enclosure_url")
            .ok_or_else(|| missing("enclosure_url"))?,
    };
    let Value::String(audio_url) = audio_url else {
        return Err(unsupported("audio URLs that are not strings"));
    };
    let media = super::playerfm::clean_podcast_url(audio_url);

    let season = object(episode.get("season"), "seasons")?;
    let season_get = |name: &str| season.and_then(|season| season.get(name));
    let season_id = match season_get("href") {
        Some(Value::String(href)) if !href.is_empty() => SEASON_ID
            .captures(href)
            .map_or(Value::Null, |found| found[1].into()),
        value if truthy(value) => return Err(unsupported("season links that are not strings")),
        _ => Value::Null,
    };

    let webpage_url = text(episode.get("episode_url"), "episode URLs")?;
    let channel_url = match webpage_url.as_str() {
        Some(webpage) if !webpage.is_empty() => CHANNEL_URL
            .captures(webpage)
            .map_or(Value::Null, |found| found[1].into()),
        _ => Value::Null,
    };
    let Some(webpage) = webpage_url.as_str() else {
        return Err(unsupported("episodes without episode URLs"));
    };
    let webpage =
        Url::parse(webpage).map_err(|_| unsupported("episode URLs that are not absolute"))?;
    // `try_get(episode, lambda x: x['podcast']['title'])`.
    let series = episode
        .get("podcast")
        .and_then(|podcast| podcast.get("title"))
        .cloned()
        .unwrap_or(Value::Null);
    let description = match episode.get("description") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(text)) => text.trim().into(),
        Some(_) => return Err(unsupported("descriptions that are not strings")),
    };
    let timestamp = match episode.get("published_at") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(date)) => {
            crate::dates::parse_iso8601(date).map_or(Value::Null, Value::from)
        }
        Some(_) => return Err(unsupported("publish dates that are not strings")),
    };
    let thumbnail = text(episode.get("image_url"), "images")?;
    // `audio_file.get('size') or episode.get('audio_file_size')`.
    let size = [audio_get("size"), episode.get("audio_file_size")]
        .into_iter()
        .find(|value| truthy(*value))
        .flatten()
        .or_else(|| episode.get("audio_file_size"));

    let fields: Vec<(&str, Value)> = vec![
        ("id", episode_id.as_str().into()),
        (
            "display_id",
            episode.get("slug").cloned().unwrap_or(Value::Null),
        ),
        ("title", title.into()),
        ("url", media.as_str().into()),
        ("webpage_url", webpage.as_str().into()),
        ("channel_url", channel_url),
        ("series", series),
        ("season_number", int_or_none(season_get("number"))),
        ("season_id", season_id),
        ("thumbnail", thumbnail),
        ("episode_id", episode_id.as_str().into()),
        ("episode_number", int_or_none(episode.get("number"))),
        ("description", description),
        ("timestamp", timestamp),
        ("duration", int_or_none(episode.get("duration"))),
        ("filesize", int_or_none(size)),
    ];
    let media = Url::parse(&media).map_err(|_| unsupported("audio URLs that are not absolute"))?;
    let ext =
        super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
    let mut format: crate::VideoFormat =
        serde_json::from_value(json!({"format_id": "", "url": media.as_str(), "ext": ext}))
            .map_err(|error| Error::Reported(format!("{episode_id}: {error}")))?;
    crate::info_order::mark_format(&mut format, "url-only");
    format.filesize = fields
        .iter()
        .find(|(name, _)| *name == "filesize")
        .and_then(|(_, value)| value.as_u64());
    let keys: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    let shape = format!("untyped-dict;{}", keys.join(","));
    let mut info = super::dict_info(page, (extractor, key), fields, vec![format]);
    info.webpage_url = webpage;
    Ok(crate::info_order::mark_info(info, &shape))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `_call_search_api(resource, resource_id, resource_url)`.
fn call_search_api(
    client: &Client,
    resource: &str,
    (extractor, id): (&'static str, &str),
    resource_url: &str,
) -> Result<Value> {
    let api = Url::parse(&format!("https://api.simplecast.com/{resource}s/search"))?;
    super::cloudycdn::post_json(
        client,
        &api,
        (extractor, id),
        super::login::urlencode(&[("url", resource_url)]),
    )
}

impl Extractor for Simplecast {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(episode_id) = temp_id(url.as_str()) else {
            return Ok(None);
        };
        announce(NAME, url);
        let api = Url::parse(&format!("https://api.simplecast.com/episodes/{episode_id}"))?;
        let episode = download_json(client, &api, NAME, &episode_id)?;
        parse_episode(&episode, url, (NAME, KEY), Some(&episode_id)).map(Some)
    }
}

impl Extractor for SimplecastEpisode {
    fn key(&self) -> &'static str {
        EPISODE_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some((matched, id)) = episode_match(url.as_str()) else {
            return Ok(None);
        };
        announce(EPISODE_NAME, url);
        let episode = call_search_api(client, "episode", (EPISODE_NAME, &id), &matched)?;
        parse_episode(&episode, url, (EPISODE_NAME, EPISODE_KEY), Some(&id)).map(Some)
    }
}

/// Whether a playlist's pending state is a Simplecast podcast's episodes.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == PODCAST_KEY
}

/// `SimplecastPodcastIE`'s `entries()` generator: the episodes JSON (on
/// the first read), then each `collection` item's episode with the
/// podcast's title as series, up to `limit` entries.
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
    let podcast_id = state["podcast_id"].as_str().unwrap_or_default().to_owned();
    let series = state.get("title").cloned().unwrap_or(Value::Null);
    let mut queue: std::collections::VecDeque<Value> = match state.shift_remove("queue") {
        Some(Value::Array(queue)) => queue.into(),
        _ => {
            let api = Url::parse(&format!(
                "https://api.simplecast.com/podcasts/{podcast_id}/episodes"
            ))?;
            let episodes = download_json(client, &api, PODCAST_NAME, &podcast_id)?;
            // `episodes.get('collection') or []`.
            match episodes.get("collection") {
                Some(Value::Array(collection)) => collection.iter().cloned().collect(),
                value if truthy(value) => {
                    return Err(unsupported("episode collections that are not lists"));
                }
                _ => Default::default(),
            }
        }
    };
    while let Some(episode) = queue.pop_front() {
        if limit.is_some_and(|limit| entries.len() >= limit) {
            queue.push_front(episode);
            break;
        }
        match parse_episode(&episode, &page, (PODCAST_NAME, PODCAST_KEY), None) {
            Ok(mut entry) => {
                // `info['series'] = podcast_title` (an unchanged value stays).
                if entry.extra.get("series") != Some(&series) {
                    crate::metadata_parser::set_field(&mut entry, "series", series.clone());
                }
                entries.push(entry);
            }
            // The generator's `KeyError`, reported while reading the
            // entries (`--no-abort-on-error`), ends it with a `None` entry.
            Err(Error::Reported(text)) if text.starts_with('\'') => {
                let mut entry = super::direct::build_at(page.clone(), &page, String::new());
                entry.extra.insert(crate::FETCH_ERROR.into(), text.into());
                entry
                    .extra
                    .insert(crate::FETCH_ERROR_COUNT.into(), entries.len().into());
                entries.push(entry);
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
    }
    if queue.is_empty() {
        return Ok(None);
    }
    state.insert("queue".into(), Value::Array(queue.into()));
    Ok(Some(Value::Object(state)))
}

impl Extractor for SimplecastPodcast {
    fn key(&self) -> &'static str {
        PODCAST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(subdomain) = podcast_id(url.as_str()) else {
            return Ok(None);
        };
        announce(PODCAST_NAME, url);
        let site = call_search_api(client, "site", (PODCAST_NAME, &subdomain), url.as_str())?;
        let podcast = site
            .get("podcast")
            .ok_or_else(|| key_error(&subdomain, "podcast"))?;
        let podcast_id = match podcast.get("id") {
            Some(Value::String(id)) => id.clone(),
            Some(_) => return Err(unsupported("podcast IDs that are not strings")),
            None => return Err(key_error(&subdomain, "id")),
        };
        let title = match podcast.get("title") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => Some(title.clone()),
            Some(_) => return Err(unsupported("podcast titles that are not strings")),
        };
        let state = json!({
            "kind": PODCAST_KEY,
            "podcast_id": podcast_id,
            "title": title,
            "url": url.as_str(),
        });
        // `playlist_result(entries(), podcast_id, podcast_title)`.
        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;{}{}",
                keys.join(","),
                crate::info_order::GENERATOR
            )
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: podcast_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: PODCAST_NAME.into(),
            extractor_key: PODCAST_KEY.into(),
            nested: Vec::new(),
            pending: Some(crate::PendingEntries { state }),
            entries: Vec::new(),
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        let uuid = "b6dc49a2-9404-4853-9aa9-9cfc097be876";
        assert!(suitable(&format!(
            "https://api.simplecast.com/episodes/{uuid}"
        )));
        assert!(suitable(&format!("https://player.simplecast.com/{uuid}")));
        assert!(!suitable("https://player.simplecast.com/b6dc49a2"));
        assert_eq!(
            episode_match(
                "https://the-re-bind-io-podcast.simplecast.com/episodes/errant-signal?x=1"
            ),
            Some((
                "https://the-re-bind-io-podcast.simplecast.com/episodes/errant-signal".into(),
                "errant-signal".into()
            ))
        );
        assert!(!episode_suitable(&format!(
            "https://api.simplecast.com/episodes/{uuid}"
        )));
        assert_eq!(
            podcast_id("https://the-re-bind-io-podcast.simplecast.com").as_deref(),
            Some("the-re-bind-io-podcast")
        );
        assert_eq!(
            podcast_id("https://the-re-bind-io-podcast.simplecast.com/episodes").as_deref(),
            Some("the-re-bind-io-podcast")
        );
        assert!(!podcast_suitable(
            "https://the-re-bind-io-podcast.simplecast.com/episodes/x"
        ));
        assert!(!podcast_suitable("https://player.simplecast.com/x"));
        assert!(!podcast_suitable(&format!(
            "https://api.simplecast.com/episodes/{uuid}"
        )));
        let page = Url::parse("https://blog.invalid/").unwrap();
        let html = format!(
            r#"<iframe height="200" src="https://player.simplecast.com/{uuid}?dark=true"></iframe><iframe src='https://embed.simplecast.com/b6dc49a2'></iframe>"#
        );
        let found: Vec<String> = embed_urls(&page, &html)
            .iter()
            .map(Url::to_string)
            .collect();
        assert_eq!(found, [format!("https://player.simplecast.com/{uuid}")]);
    }
}
