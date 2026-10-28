//! Upstream `CiscoLiveSessionIE` and `CiscoLiveSearchIE`
//! (`yt_dlp/extractor/ciscolive.py`): Cisco Live on-demand library sessions
//! from the RainFocus API as `url_transparent` results of their Brightcove
//! videos; search pages are read lazily as their sessions are requested.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct CiscoLiveSession;
pub struct CiscoLiveSearch;

const SESSION_NAME: &str = "CiscoLiveSession";
const SEARCH_NAME: &str = "CiscoLiveSearch";
const API: &str = "https://events.rainfocus.com/api";
const PROFILE_ID: &str = "Na3vqYdAlJFSxhYTYQGuMbpafMqftalz";
const WIDGET_ID: &str = "n6l4Lo05R8fiy3RpUBm447dZN8uNWoye";

static SESSION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?ciscolive(?:\.cisco)?\.com/[^#]*#/session/(?P<id>[^/?&]+)")
        .unwrap()
});
static SEARCH_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?ciscolive(?:\.cisco)?\.com/(?:global/)?on-demand-library(?:\.html|/)",
    )
    .unwrap()
});

/// `CiscoLiveSessionIE.suitable`.
pub fn session_suitable(url: &str) -> bool {
    SESSION_URL.is_match(url)
}

/// `CiscoLiveSearchIE.suitable`: not session URLs.
pub fn search_suitable(url: &str) -> bool {
    !session_suitable(url) && SEARCH_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Cisco Live {what} are not supported natively"))
}

/// A `KeyError` wrapped by `extract` (`temp` is the URL's ID, if any).
fn key_error(temp: Option<&str>, key: &str) -> Error {
    let message = format!(
        "An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    );
    Error::Reported(match temp {
        Some(id) => format!("{id}: {message}"),
        None => message,
    })
}

/// `[NAME] ID: TEXT`, or `[NAME] TEXT` without an ID.
fn prefixed(name: &str, id: Option<&str>, text: &str) -> String {
    match id {
        Some(id) => format!("[{name}] {id}: {text}"),
        None => format!("[{name}] {text}"),
    }
}

/// `_call_api(ep, rf_id, query, referrer, note)`: the form-encoded POST
/// with the RainFocus headers.
fn call_api(
    client: &Client,
    name: &'static str,
    ep: &str,
    rf_id: Option<&str>,
    query: &[(String, String)],
    referrer: &str,
    note: &str,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&prefixed(name, rf_id, note));
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(query)
        .finish();
    let mut response = super::direct::post(
        client,
        &Url::parse(&format!("{API}/{ep}"))?,
        &[
            ("Origin", "https://ciscolive.cisco.com"),
            ("rfApiProfileId", PROFILE_ID),
            ("rfWidgetId", WIDGET_ID),
            ("Referer", referrer),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        body,
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => {
            let text = super::request_error_text(name, "\u{0}", "JSON metadata", &error);
            Error::Reported(match rf_id {
                Some(id) => text.replacen('\u{0}', id, 1),
                None => text.replacen(" \u{0}:", "", 1),
            })
        }
        error => error,
    })?;
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    let bytes = super::decode_content(response.headers(), bytes).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&bytes, response.headers());
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(prefixed(
            name,
            rf_id,
            &format!("Failed to parse JSON  (caused by {error})"),
        ))
    })
}

/// `try_get(rf_item, lambda x: x[a][0][b])`.
fn first_field<'a>(item: &'a Value, list: &str, key: &str) -> Option<&'a Value> {
    item.get(list)?.as_array()?.first()?.as_object()?.get(key)
}

/// `_parse_rf_item(rf_item)`: the `url_transparent` result's URL and
/// fields, in upstream's key order.
fn parse_rf_item(item: &Value, temp: Option<&str>) -> Result<(String, Map<String, Value>)> {
    let Some(object) = item.as_object() else {
        return Err(unsupported("session items that are not objects"));
    };
    let event_name = object.get("eventName").cloned().unwrap_or(Value::Null);
    let title = object
        .get("title")
        .cloned()
        .ok_or_else(|| key_error(temp, "title"))?;
    let description = match object.get("abstract") {
        Some(Value::String(text)) => Value::from(crate::html::clean_html(text)),
        None | Some(Value::Null) => Value::Null,
        Some(_) => return Err(unsupported("abstracts that are not strings")),
    };
    let presenter = first_field(item, "participants", "fullName")
        .cloned()
        .unwrap_or(Value::Null);
    let bc_id = match object.get("videos") {
        None => return Err(key_error(temp, "videos")),
        Some(Value::Array(videos)) => match videos.first() {
            None => return Err(Error::Reported("list index out of range".into())),
            Some(video) => match video.get("url") {
                Some(Value::String(id)) => id.clone(),
                Some(Value::Number(id)) => id.to_string(),
                Some(_) => return Err(unsupported("video IDs that are not strings")),
                None => return Err(key_error(temp, "url")),
            },
        },
        Some(_) => return Err(unsupported("video lists that are not lists")),
    };
    let target = format!(
        "http://players.brightcove.net/5647924234001/SyK2FdqjM_default/index.html?videoId={bc_id}"
    );
    // `float_or_none(length)`, in minutes.
    let duration = match first_field(item, "times", "length") {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    let duration = match duration {
        Some(duration) if duration != 0.0 => json!(duration * 60.0),
        Some(duration) => json!(duration),
        None => Value::Null,
    };
    let location = first_field(item, "times", "room")
        .cloned()
        .unwrap_or(Value::Null);
    let mut fields = Map::new();
    fields.insert("title".into(), title);
    fields.insert("description".into(), description);
    fields.insert("duration".into(), duration);
    fields.insert("creator".into(), presenter);
    fields.insert("location".into(), location);
    fields.insert("series".into(), event_name);
    Ok((target, fields))
}

impl Extractor for CiscoLiveSession {
    fn key(&self) -> &'static str {
        SESSION_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SESSION_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let rf_id = found["id"].to_owned();
        start(SESSION_NAME, url);
        let result = call_api(
            client,
            SESSION_NAME,
            "session",
            Some(&rf_id),
            &[("id".into(), rf_id.clone())],
            url.as_str(),
            "Downloading JSON metadata",
        )?;
        let item = match result.get("items") {
            None => return Err(key_error(Some(&rf_id), "items")),
            Some(Value::Array(items)) => items
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            Some(_) => return Err(unsupported("session item lists that are not lists")),
        };
        let (target, fields) = parse_rf_item(item, Some(&rf_id))?;
        // `{'_type': 'url_transparent', 'url': ..., 'ie_key': 'BrightcoveNew', ...}`.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": fields}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

/// Whether a playlist's pending state is a Cisco Live search's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == SEARCH_NAME
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    super::streamable::int_or_none(value).as_i64()
}

/// A search item's `url_transparent` entry.
fn search_entry(item: &Value) -> Result<VideoInfo> {
    let (target, fields) = parse_rf_item(item, None)?;
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("_type".into(), "url_transparent".into());
    entry.insert("url".into(), target.clone().into());
    entry.insert("ie_key".into(), "BrightcoveNew".into());
    entry.extend(fields.clone());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": fields,
            "ie_key": "BrightcoveNew",
            "extractor": "brightcove:new",
            "entry": entry,
        }),
    );
    Ok(info)
}

/// The search generator: `search` pages (`size`/`from` advanced by each
/// response) fetched as their sessions are requested, up to `limit`.
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let Value::Object(mut state) = state else {
        return Ok(None);
    };
    let referrer = state
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let query: Vec<(String, String)> = match state.get("query") {
        Some(Value::Array(pairs)) => pairs
            .iter()
            .filter_map(|pair| {
                Some((
                    pair.get(0)?.as_str()?.to_owned(),
                    pair.get(1)?.as_str()?.to_owned(),
                ))
            })
            .collect(),
        _ => Vec::new(),
    };
    let mut size = state.get("size").and_then(Value::as_i64).unwrap_or(50);
    let mut from = state.get("from").and_then(Value::as_i64).unwrap_or(0);
    let mut page_num = state.get("page").and_then(Value::as_i64).unwrap_or(1);
    let mut done = state.get("done").and_then(Value::as_bool).unwrap_or(false);
    let mut queue: std::collections::VecDeque<Value> = match state.shift_remove("queue") {
        Some(Value::Array(queue)) => queue.into(),
        _ => Default::default(),
    };
    while !limit.is_some_and(|limit| entries.len() >= limit) {
        if let Some(item) = queue.pop_front() {
            entries.push(search_entry(&item)?);
            continue;
        }
        if done {
            break;
        }
        let mut form = query.clone();
        form.push(("size".into(), size.to_string()));
        form.push(("from".into(), from.to_string()));
        let mut results = call_api(
            client,
            SEARCH_NAME,
            "search",
            None,
            &form,
            &referrer,
            &format!("Downloading search JSON page {page_num}"),
        )?;
        page_num += 1;
        if let Some(section) = results
            .pointer("/sectionList/0")
            .filter(|section| section.as_object().is_some_and(|map| !map.is_empty()))
        {
            results = section.clone();
        }
        let Some(results) = results.as_object() else {
            return Err(unsupported("search results that are not objects"));
        };
        let items = match results.get("items") {
            Some(Value::Array(items)) if !items.is_empty() => items,
            _ => {
                done = true;
                break;
            }
        };
        // `_check_bc_id_exists`: only items with an integer Brightcove ID.
        queue.extend(
            items
                .iter()
                .filter(|item| item.is_object())
                .filter(|item| int_or_none(first_field(item, "videos", "url")).is_some())
                .cloned(),
        );
        if let Some(new_size) = int_or_none(results.get("size")) {
            size = new_size;
        }
        if let Some(total) = int_or_none(results.get("total"))
            && from + size > total
        {
            done = true;
            continue;
        }
        from += size;
    }
    if queue.is_empty() && done {
        return Ok(None);
    }
    state.insert("size".into(), size.into());
    state.insert("from".into(), from.into());
    state.insert("page".into(), page_num.into());
    state.insert("done".into(), done.into());
    state.insert("queue".into(), Value::Array(queue.into()));
    Ok(Some(Value::Object(state)))
}

/// `str(list)` of `parse_qs` values, as `urlencode` (without `doseq`)
/// writes them.
fn py_list_repr(values: &[String]) -> String {
    let items: Vec<String> = values
        .iter()
        .map(|value| crate::batch::py_repr(value))
        .collect();
    format!("[{}]", items.join(", "))
}

impl Extractor for CiscoLiveSearch {
    fn key(&self) -> &'static str {
        SEARCH_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !search_suitable(url.as_str()) {
            return Ok(None);
        }
        start(SEARCH_NAME, url);
        // `parse_qs(url)` (blank values dropped, repeated keys grouped),
        // then `type`.
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for (key, value) in url.query_pairs() {
            if value.is_empty() {
                continue;
            }
            match groups.iter_mut().find(|(existing, _)| *existing == key) {
                Some((_, values)) => values.push(value.into_owned()),
                None => groups.push((key.into_owned(), vec![value.into_owned()])),
            }
        }
        let mut query: Vec<Value> = groups
            .iter()
            .map(|(key, values)| json!([key, py_list_repr(values)]))
            .collect();
        query.push(json!(["type", "session"]));
        let mut extra = Map::new();
        extra.insert("id".into(), Value::Null);
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;title{}", crate::info_order::GENERATOR).into(),
        );
        let state = json!({
            "kind": SEARCH_NAME,
            "url": url.as_str(),
            "query": query,
        });
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: String::new(),
            title: "Search query".into(),
            webpage_url: url.clone(),
            extractor: SEARCH_NAME.into(),
            extractor_key: SEARCH_NAME.into(),
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
        let session =
            "https://ciscolive.cisco.com/on-demand-library/?#/session/1423353499155001FoSs";
        assert!(session_suitable(session));
        assert!(!search_suitable(session));
        assert!(search_suitable(
            "https://www.ciscolive.com/global/on-demand-library.html?search.event=x#/"
        ));
        assert_eq!(py_list_repr(&["a".into(), "b'c".into()]), r#"['a', "b'c"]"#);
    }
}
