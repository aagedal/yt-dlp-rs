//! Upstream `FreeTvMoviesIE` and `FreeTvIE` (`yt_dlp/extractor/freetv.py`):
//! FreeTV movies and series through the WordPress `admin-ajax.php` API
//! (HLS playlists with subtitles); a series' seasons are read lazily as its
//! episodes are requested.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct FreeTvMovies;
pub struct FreeTv;

const MOVIES_NAME: &str = "FreeTvMovies";
const SERIES_NAME: &str = "freetv:series";
const SERIES_KEY: &str = "FreeTv";
const API: &str = "https://www.freetv.com/wordpress/wp-admin/admin-ajax.php";

static MOVIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?freetv\.com/peliculas/(?P<id>[^/]+)").unwrap()
});
static SERIES_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?freetv\.com/series/(?P<id>[^/]+)").unwrap());
static VIDEO_IDS: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r#"class=["'][^>]+postid-(?P<video_id>\d+)"#).unwrap(),
        Regex::new(r"<link[^>]+freetv.com/\?p=(?P<video_id>\d+)").unwrap(),
        Regex::new(r#"<div[^>]+data-params=["'][^>]+post_id=(?P<video_id>\d+)"#).unwrap(),
    ]
});
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<h1[^>]+class=["']synopis[^>]>(?P<title>[^<]+)"#).unwrap());
static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div[^>]+class=["']+synopis content[^>]><p>(?P<description>[^<]+)"#).unwrap()
});
static SEASON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<option[^>]+value=["'](\d+)["']"#).unwrap());

/// `FreeTvMoviesIE._match_id`.
pub fn movies_suitable(url: &str) -> bool {
    MOVIES_URL.is_match(url)
}

/// `FreeTvIE._match_id`.
pub fn suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("FreeTV {what} are not supported natively"))
}

/// `str(value)` of a JSON scalar.
fn py_str(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        _ => Err(unsupported("content IDs that are not scalars")),
    }
}

/// `_get_api_response(content_id, resource_type, postdata)`: the POST's
/// `data`.
fn api_response(
    client: &Client,
    name: &'static str,
    content_id: &str,
    resource: &str,
    postdata: &[(&str, &str)],
    temp_id: &str,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!(
        "[{name}] {content_id}: Downloading {content_id} {resource} JSON"
    ));
    let mut body = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in postdata {
        body.append_pair(key, value);
    }
    let mut response = super::direct::post(
        client,
        &Url::parse(API)?,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body.finish(),
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: name,
            id: content_id.to_owned(),
            what: "JSON metadata",
            source: Box::new(error),
        },
        error => error,
    })?;
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    let bytes = super::decode_content(response.headers(), bytes).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&bytes, response.headers());
    let data: Value = crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{name}] {content_id}: Failed to parse JSON  (caused by {error})"
        ))
    })?;
    data.get("data")
        .cloned()
        .ok_or_else(|| key_error(temp_id, "data"))
}

impl Extractor for FreeTvMovies {
    fn key(&self) -> &'static str {
        MOVIES_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = MOVIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(MOVIES_NAME, url);
        let webpage = download_webpage(client, url, MOVIES_NAME, &display_id)?;
        let content_id = VIDEO_IDS
            .iter()
            .find_map(|pattern| {
                pattern
                    .captures(&webpage)
                    .map(|found| found["video_id"].to_owned())
            })
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{MOVIES_NAME}] {display_id}: Unable to extract video id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let data = api_response(
            client,
            MOVIES_NAME,
            &content_id,
            "video",
            &[("action", "olyott_video_play"), ("contentID", &content_id)],
            &display_id,
        )?;
        let meta = data
            .get("displayMeta")
            .ok_or_else(|| key_error(&display_id, "displayMeta"))?;
        let video_id = meta
            .get("contentID")
            .cloned()
            .ok_or_else(|| key_error(&display_id, "contentID"))?;
        let video_url = meta
            .get("streamURLVideo")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&display_id, "streamURLVideo"))?;
        let id_text = py_str(&video_id)?;
        let stream = extract_m3u8_fatal_master(
            client,
            &Url::parse(video_url).map_err(|_| unsupported("stream URLs that are not URLs"))?,
            MOVIES_NAME,
            &id_text,
            None,
            true,
        )?;
        let forced_string = video_id.is_number();
        let mut info = dict_info(
            url,
            (MOVIES_NAME, MOVIES_NAME),
            vec![
                ("id", id_text.into()),
                ("title", meta.get("title").cloned().unwrap_or(Value::Null)),
                (
                    "description",
                    meta.get("desc").cloned().unwrap_or(Value::Null),
                ),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            stream.formats,
        );
        info.subtitles = stream.subtitles;
        if forced_string {
            crate::screen::warning(
                "\"id\" field is not a string - forcing string conversion, there is an error in extractor",
            );
        }
        Ok(Some(info))
    }
}

/// Whether a playlist's pending state is a FreeTV series'.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == SERIES_KEY
}

/// One `_extract_series_season` episode.
fn episode(
    client: &Client,
    page: &Url,
    episode: &Value,
    series_title: &Value,
    temp_id: &str,
) -> Result<VideoInfo> {
    let video_id = py_str(
        episode
            .get("contentID")
            .ok_or_else(|| key_error(temp_id, "contentID"))?,
    )?;
    let stream_url = episode
        .get("streamURL")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(temp_id, "streamURL"))?;
    let stream = extract_m3u8_fatal_master(
        client,
        &Url::parse(stream_url).map_err(|_| unsupported("stream URLs that are not URLs"))?,
        SERIES_NAME,
        &video_id,
        None,
        true,
    )?;
    let meta = |key: &str| {
        episode
            .pointer(&format!("/contentMeta/displayMeta/{key}"))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let int = |key: &str| match meta(key) {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Value::String(text) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    };
    let get = |key: &str| episode.get(key).cloned().unwrap_or(Value::Null);
    let mut info = dict_info(
        page,
        (SERIES_NAME, SERIES_KEY),
        vec![
            ("id", video_id.into()),
            ("title", get("fullTitle")),
            ("description", get("description")),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("thumbnail", get("thumbnail")),
            ("series", series_title.clone()),
            ("series_id", meta("seriesID")),
            ("season_id", meta("seasonID")),
            ("season_number", int("seasonNum")),
            ("episode_number", int("episodeNum")),
        ],
        stream.formats,
    );
    info.subtitles = stream.subtitles;
    Ok(info)
}

/// The series generator: seasons fetched as their episodes are requested.
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
    let temp_id = state["display_id"].as_str().unwrap_or_default().to_owned();
    let series_title = state.get("title").cloned().unwrap_or(Value::Null);
    let mut seasons: std::collections::VecDeque<Value> = match state.shift_remove("seasons") {
        Some(Value::Array(seasons)) => seasons.into(),
        _ => Default::default(),
    };
    let mut queue: std::collections::VecDeque<Value> = match state.shift_remove("queue") {
        Some(Value::Array(queue)) => queue.into(),
        _ => Default::default(),
    };
    loop {
        if limit.is_some_and(|limit| entries.len() >= limit) {
            break;
        }
        if let Some(item) = queue.pop_front() {
            entries.push(episode(client, &page, &item, &series_title, &temp_id)?);
            continue;
        }
        let Some(season_id) = seasons.pop_front() else {
            break;
        };
        let season_id = season_id.as_str().unwrap_or_default().to_owned();
        let data = api_response(
            client,
            SERIES_NAME,
            &season_id,
            "series",
            &[
                ("contentID", &season_id),
                ("action", "olyott_get_dynamic_series_content"),
                ("type", "list"),
                ("perPage", "1000"),
            ],
            &temp_id,
        )?;
        match data.get("1") {
            Some(Value::Array(items)) => queue.extend(items.iter().cloned()),
            Some(_) => return Err(unsupported("episode lists that are not lists")),
            None => return Err(key_error(&temp_id, "1")),
        }
    }
    if queue.is_empty() && seasons.is_empty() {
        return Ok(None);
    }
    state.insert("seasons".into(), Value::Array(seasons.into()));
    state.insert("queue".into(), Value::Array(queue.into()));
    Ok(Some(Value::Object(state)))
}

/// `_html_search_regex(..., fatal=False)`.
fn search_optional(webpage: &str, pattern: &Regex, name: &str) -> Option<String> {
    let found = pattern
        .captures(webpage)
        .map(|found| crate::html::clean_html(&found[1]));
    if found.is_none() {
        crate::screen::warning(&format!(
            "[{SERIES_NAME}] unable to extract {name}; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    found
}

impl Extractor for FreeTv {
    fn key(&self) -> &'static str {
        SERIES_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(SERIES_NAME, url);
        let webpage = download_webpage(client, url, SERIES_NAME, &display_id)?;
        let title = search_optional(&webpage, &TITLE, "title");
        let description = search_optional(&webpage, &DESCRIPTION, "description");
        let seasons: Vec<Value> = SEASON
            .captures_iter(&webpage)
            .map(|found| Value::from(found[1].to_owned()))
            .collect();
        let title = title.filter(|title| !title.is_empty());
        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        let mut extra = Map::new();
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
        let state = json!({
            "kind": SERIES_KEY,
            "url": url.as_str(),
            "display_id": display_id,
            "title": title.clone().map_or(Value::Null, Value::from),
            "seasons": seasons,
        });
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: SERIES_NAME.into(),
            extractor_key: SERIES_KEY.into(),
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
        assert!(movies_suitable(
            "https://www.freetv.com/peliculas/atrapame-si-puedes/"
        ));
        assert!(suitable("https://www.freetv.com/series/el-detective-l/"));
        assert!(!suitable("https://www.freetv.com/peliculas/x/"));
    }
}
