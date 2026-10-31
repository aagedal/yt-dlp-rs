//! Upstream `SpreakerIE` and `SpreakerShowIE` (`yt_dlp/extractor/spreaker.py`):
//! the `api.spreaker.com` v2 episode JSON as an untyped MP3 result, and a
//! show's paged episode list as a generator of the same results.
use super::{Extractor, download_json, download_json_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Spreaker;
pub struct SpreakerShow;

const NAME: &str = "Spreaker";
const SHOW_NAME: &str = "SpreakerShow";

static VALID_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        crate::extractor::url_regex(
            r"^https?://api\.spreaker\.com/(?:(?:download/)?episode|v2/episodes)/(?P<id>\d+)",
        )
        .unwrap(),
        crate::extractor::url_regex(
            r"^https?://(?:www\.)?spreaker\.com/episode/[^#?/]*?(?P<id>\d+)/?(?:[?#]|$)",
        )
        .unwrap(),
    ]
});
static SHOW_URLS: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r"^https?://api\.spreaker\.com/show/(?P<id>\d+)").unwrap(),
        Regex::new(r"^https?://(?:www\.)?spreaker\.com/podcast/[\w-]+--(?P<id>[\d]+)").unwrap(),
        Regex::new(r"^https?://(?:www\.)?spreaker\.com/show/(?P<id>\d+)/episodes/feed").unwrap(),
    ]
});

fn match_id(patterns: &[Regex], url: &str) -> Option<String> {
    patterns
        .iter()
        .find_map(|pattern| pattern.captures(url))
        .map(|found| found["id"].to_owned())
}

/// `SpreakerIE._match_id`.
pub fn suitable(url: &str) -> bool {
    match_id(&*VALID_URLS, url).is_some()
}

/// `SpreakerShowIE._match_id`.
pub fn show_suitable(url: &str) -> bool {
    match_id(&*SHOW_URLS, url).is_some()
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Spreaker {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `traverse_obj(parse_qs(url), ('key', 0))`.
fn query_key(url: &Url) -> Option<String> {
    url.query_pairs()
        .find(|(name, value)| name == "key" && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => text.clone().into(),
        Some(Value::Number(number)) => number.to_string().into(),
        Some(Value::Bool(flag)) => if *flag { "True" } else { "False" }.into(),
        _ => Value::Null,
    }
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

/// `float_or_none(value, scale=1000)`, `None` for zero (`or`).
fn milliseconds(value: Option<&Value>) -> Option<f64> {
    let number = match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        Value::Bool(flag) => Some(f64::from(u8::from(*flag))),
        _ => None,
    }?;
    Some(number / 1000.0).filter(|seconds| *seconds != 0.0)
}

/// `_extract_episode(data, episode_id)`; `error_id` is the extraction's
/// temporary ID.
fn extract_episode(
    data: &Value,
    episode_id: Option<&str>,
    webpage_url: &Url,
    extractor: &'static str,
    error_id: &str,
) -> Result<VideoInfo> {
    let data = data
        .as_object()
        .ok_or_else(|| unsupported("episodes that are not objects"))?;
    // A generator's `KeyError` is not wrapped in an extractor error.
    let missing = |key: &str| {
        if episode_id.is_some() {
            key_error(error_id, key)
        } else {
            Error::Reported(format!("'{key}'"))
        }
    };
    let title = match data.get("title") {
        Some(Value::String(title)) => title.clone(),
        Some(_) => return Err(unsupported("episode titles that are not strings")),
        None => return Err(missing("title")),
    };
    let download_url = match data.get("download_url") {
        Some(Value::String(media)) => media.clone(),
        Some(_) => return Err(unsupported("download URLs that are not strings")),
        None => return Err(missing("download_url")),
    };
    let string_at = |path: [&str; 2]| match data.get(path[0]).and_then(|value| value.get(path[1])) {
        Some(Value::String(text)) => Value::from(text.as_str()),
        _ => Value::Null,
    };
    let series = string_at(["show", "title"]);
    let uploader = string_at(["author", "fullname"]);
    let mut thumbnails = Vec::new();
    for image in ["image_original", "image_medium", "image"] {
        if let Some(url) = data
            .get(&format!("{image}_url"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            thumbnails.push(json!({"url": url}));
        }
    }
    // `try_get(data, (KEYs_count, stats.KEYs))`: the first path present.
    let stats = |key: &str| match data.get(&format!("{key}s_count")) {
        Some(value) => int_or_none(Some(value)),
        None => int_or_none(
            data.get("stats")
                .and_then(Value::as_object)
                .and_then(|stats| stats.get(&format!("{key}s"))),
        ),
    };
    let duration = milliseconds(data.get("duration")).or_else(|| milliseconds(data.get("length")));
    let id = match episode_id {
        Some(id) => id.to_owned(),
        None => match data.get("episode_id") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) if id.is_i64() || id.is_u64() => id.to_string(),
            Some(_) => return Err(unsupported("episode IDs that are not strings or integers")),
            None => return Err(missing("episode_id")),
        },
    };
    let timestamp = match data.get("published_at") {
        Some(Value::String(date)) => {
            crate::dates::unified_timestamp(date, true).map_or(Value::Null, Value::from)
        }
        None | Some(Value::Null) => Value::Null,
        Some(_) => return Err(unsupported("publish dates that are not strings")),
    };
    let description = data.get("description").cloned().unwrap_or(Value::Null);
    if !matches!(description, Value::Null | Value::String(_)) {
        return Err(unsupported("descriptions that are not strings"));
    }

    let media = Url::parse(&download_url)
        .map_err(|_| unsupported("download URLs that are not absolute"))?;
    let mut info = super::direct::with_format(
        super::direct::build_at(media.clone(), &media, "mp3".into()),
        "",
    );
    for format in &mut info.formats {
        format.ext = "mp3".into();
        format.format_id = "mp3".into();
        format.format = Some("MPEG Layer 3".into());
        format.extra.insert("container".into(), "mp3".into());
    }
    info.format_id = None;
    info.id = id;
    info.title = title;
    info.webpage_url = webpage_url.clone();
    info.direct = None;
    info.extractor = extractor.into();
    info.extractor_key = NAME.into();
    let mut keys = vec!["id", "url", "display_id", "title"];
    for (key, value) in [
        ("display_id", str_or_none(data.get("permalink"))),
        ("description", description),
        ("timestamp", timestamp),
        ("uploader", uploader.clone()),
        ("uploader_id", str_or_none(data.get("author_id"))),
        ("creator", uploader),
        ("duration", duration.map_or(Value::Null, Value::from)),
        ("view_count", stats("play")),
        ("like_count", stats("like")),
        ("comment_count", stats("message")),
        ("format", "MPEG Layer 3".into()),
        ("format_id", "mp3".into()),
        ("container", "mp3".into()),
        ("ext", "mp3".into()),
        ("thumbnails", Value::Array(thumbnails)),
        ("series", series),
    ] {
        // Processing drops a `None` `duration`.
        if key == "duration" && value.is_null() {
            continue;
        }
        if key != "display_id" {
            keys.push(key);
        }
        crate::metadata_parser::set_field(&mut info, key, value);
    }
    keys.push("extractor_key");
    Ok(crate::info_order::mark_info(
        info,
        &format!("untyped-dict;{}", keys.join(",")),
    ))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for Spreaker {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(episode_id) = match_id(&*VALID_URLS, url.as_str()) else {
            return Ok(None);
        };
        announce(NAME, url);
        let mut api = Url::parse(&format!(
            "https://api.spreaker.com/v2/episodes/{episode_id}"
        ))?;
        if let Some(key) = query_key(url) {
            api.query_pairs_mut().append_pair("key", &key);
        }
        let response = download_json(client, &api, NAME, &episode_id)?;
        let data = response
            .get("response")
            .ok_or_else(|| key_error(&episode_id, "response"))?
            .get("episode")
            .ok_or_else(|| key_error(&episode_id, "episode"))?;
        extract_episode(data, Some(&episode_id), url, NAME, &episode_id).map(Some)
    }
}

/// Whether a playlist's pending state is a Spreaker show's episodes.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == SHOW_NAME
}

/// `SpreakerShowIE._entries`: one JSON page at a time, each page's dict
/// results yielded as episodes, up to `limit` entries.
pub(crate) fn fetch_episodes(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let Value::Object(mut state) = state else {
        return Ok(None);
    };
    let show_id = state
        .get("show_id")
        .unwrap_or(&Value::Null)
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let page = Url::parse(
        state
            .get("url")
            .unwrap_or(&Value::Null)
            .as_str()
            .unwrap_or_default(),
    )?;
    let key = state
        .get("key")
        .unwrap_or(&Value::Null)
        .as_str()
        .map(str::to_owned);
    let mut queue: std::collections::VecDeque<Value> = match state.shift_remove("queue") {
        Some(Value::Array(queue)) => queue.into(),
        _ => Default::default(),
    };
    let mut page_num = state
        .get("page")
        .unwrap_or(&Value::Null)
        .as_u64()
        .unwrap_or(0);
    let mut done = state
        .get("done")
        .unwrap_or(&Value::Null)
        .as_bool()
        .unwrap_or(false);
    loop {
        if limit.is_some_and(|limit| entries.len() >= limit) {
            break;
        }
        if let Some(result) = queue.pop_front() {
            if result.is_object() {
                match extract_episode(&result, None, &page, SHOW_NAME, &show_id) {
                    Ok(entry) => entries.push(entry),
                    // The generator's `KeyError`, reported while reading the
                    // entries (`--no-abort-on-error`), ends it with a `None`
                    // entry.
                    Err(Error::Reported(text)) if text.starts_with('\'') => {
                        let mut entry = super::direct::build_at(page.clone(), &page, "mp3".into());
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
            continue;
        }
        if done {
            return Ok(None);
        }
        page_num += 1;
        let mut api = Url::parse(&format!("https://api.spreaker.com/show/{show_id}/episodes"))?;
        api.query_pairs_mut()
            .append_pair("page", &page_num.to_string())
            .append_pair("max_per_page", "100");
        if let Some(key) = &key {
            api.query_pairs_mut().append_pair("key", key);
        }
        let episodes = download_json_note(
            client,
            &api,
            SHOW_NAME,
            &show_id,
            &format!("Downloading JSON page {page_num}"),
        )?;
        let pager = episodes
            .get("response")
            .and_then(|response| response.get("pager"))
            .and_then(Value::as_object);
        let Some(pager) = pager else {
            return Ok(None);
        };
        let Some(Value::Array(results)) = pager.get("results").filter(|results| {
            results
                .as_array()
                .is_some_and(|results| !results.is_empty())
        }) else {
            return Ok(None);
        };
        queue.extend(results.iter().cloned());
        done = pager.get("last_page").is_some_and(|last| {
            last.as_u64() == Some(page_num) || last.as_f64() == Some(page_num as f64)
        });
    }
    if queue.is_empty() && done {
        return Ok(None);
    }
    state.insert("queue".into(), Value::Array(queue.into()));
    state.insert("page".into(), page_num.into());
    state.insert("done".into(), done.into());
    Ok(Some(Value::Object(state)))
}

impl Extractor for SpreakerShow {
    fn key(&self) -> &'static str {
        SHOW_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(show_id) = match_id(&*SHOW_URLS, url.as_str()) else {
            return Ok(None);
        };
        announce(SHOW_NAME, url);
        let state = json!({
            "kind": SHOW_NAME,
            "show_id": show_id,
            "url": url.as_str(),
            "key": query_key(url),
        });
        // `playlist_result(entries, playlist_id=show_id)`.
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;id{}", crate::info_order::GENERATOR).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: show_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: SHOW_NAME.into(),
            extractor_key: SHOW_NAME.into(),
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
        for (url, id) in [
            ("https://api.spreaker.com/episode/12534508", "12534508"),
            (
                "https://api.spreaker.com/download/episode/12534508/swm_ep15_how_to_market_your_music_part_2.mp3",
                "12534508",
            ),
            (
                "https://api.spreaker.com/v2/episodes/12534508?export=episode_segments",
                "12534508",
            ),
            (
                "https://www.spreaker.com/episode/grunge-music-origins-the-raw-sound-that-defined-a-generation--60269615",
                "60269615",
            ),
            ("https://www.spreaker.com/episode/60269615", "60269615"),
            ("https://www.spreaker.com/episode/a90s-b--123/?x", "123"),
        ] {
            assert_eq!(match_id(&*VALID_URLS, url).as_deref(), Some(id), "{url}");
        }
        assert!(!suitable("https://www.spreaker.com/episode/abc"));
        for (url, id) in [
            ("https://api.spreaker.com/show/4652058", "4652058"),
            (
                "https://www.spreaker.com/podcast/health-wealth--5918323",
                "5918323",
            ),
            (
                "https://www.spreaker.com/show/5887186/episodes/feed",
                "5887186",
            ),
        ] {
            assert_eq!(match_id(&*SHOW_URLS, url).as_deref(), Some(id), "{url}");
        }
        assert!(!show_suitable("https://www.spreaker.com/show/5887186"));
    }
}
