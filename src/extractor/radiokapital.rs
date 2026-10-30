//! Upstream `RadioKapitalIE` and `RadioKapitalShowIE`
//! (`yt_dlp/extractor/radiokapital.py`): Radio Kapitał episodes as
//! transparent `Mixcloud` results with the API's episode metadata, and shows
//! as lazily paged playlists of them.
use super::{Extractor, download_json_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct RadioKapital;
pub struct RadioKapitalShow;

const NAME: &str = "radiokapital";
const SHOW_NAME: &str = "radiokapital:show";
const MIXCLOUD: &str = "Mixcloud";
const PENDING_KIND: &str = "radiokapital:show";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?radiokapital\.pl/shows/[a-z\d-]+/(?P<id>[a-z\d-]+)",
    )
    .unwrap()
});
static SHOW_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?radiokapital\.pl/shows/(?P<id>[a-z\d-]+)/?(?:$|[?#])").unwrap()
});

/// `RadioKapitalIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `RadioKapitalShowIE._match_id`.
pub fn show_suitable(url: &str) -> bool {
    SHOW_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// A `KeyError`: wrapped by `extract()` for its URL's ID, else (raised
/// while reading the show's entries) its bare `'key'`.
fn key_error(id: Option<&str>, key: &str) -> Error {
    match id {
        Some(id) => Error::Reported(format!(
            "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
            crate::BUG_REPORTS_MESSAGE
        )),
        None => Error::Reported(format!("'{key}'")),
    }
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Radio Kapitał {what} are not supported natively"))
}

/// `_call_api(resource, video_id, note, qs)` (the request drops an empty
/// query's `?`, like upstream's URL normalization).
fn call_api(
    client: &Client,
    name: &'static str,
    resource: &str,
    video_id: &str,
    note: &str,
    qs: &[(&str, &str)],
) -> Result<Value> {
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(qs)
        .finish();
    let mut api = Url::parse(&format!(
        "https://www.radiokapital.pl/wp-json/kapital/v1/{resource}"
    ))?;
    if !query.is_empty() {
        api.set_query(Some(&query));
    }
    download_json_note(client, &api, name, video_id, note)
}

/// Python `text[start:end]` by code points.
fn slice(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

/// `_parse_episode(data)`: the `url_transparent` dict's fields in their key
/// order and its Mixcloud URL (see [`key_error`] for `id`).
fn parse_episode(data: &Value, id: Option<&str>) -> Result<(Map<String, Value>, String)> {
    let published = match data.get("published") {
        Some(Value::String(published)) => published,
        Some(_) => return Err(unsupported("publication dates that are not strings")),
        None => return Err(key_error(id, "published")),
    };
    let release = format!(
        "{}{}{}",
        slice(published, 6, 11),
        slice(published, 3, 6),
        slice(published, 0, 3)
    );
    let target = match data.get("mixcloud_url") {
        Some(Value::String(target)) => target.clone(),
        Some(_) => return Err(unsupported("Mixcloud URLs that are not strings")),
        None => return Err(key_error(id, "mixcloud_url")),
    };
    let title = match data.get("title") {
        Some(Value::String(title)) => Value::from(crate::html::unescape_html(title)),
        Some(Value::Null) => Value::Null,
        Some(_) => return Err(unsupported("titles that are not strings")),
        None => return Err(key_error(id, "title")),
    };
    let description = match data.get("content") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(content)) => crate::html::clean_html(content).into(),
        Some(_) => return Err(unsupported("contents that are not strings")),
    };
    // `traverse_obj(data, ('tags', ..., 'name'))`.
    let tags = match data.get("tags") {
        Some(Value::Array(tags)) => Value::Array(
            tags.iter()
                .filter_map(|tag| tag.get("name"))
                .filter(|name| !name.is_null())
                .cloned()
                .collect(),
        ),
        Some(Value::Object(tags)) => Value::Array(
            tags.values()
                .filter_map(|tag| tag.get("name"))
                .filter(|name| !name.is_null())
                .cloned()
                .collect(),
        ),
        _ => Value::Null,
    };
    let series = data
        .get("show")
        .and_then(|show| show.get("title"))
        .cloned()
        .unwrap_or(Value::Null);
    let mut fields = Map::new();
    for (key, value) in [
        ("title", title),
        ("description", description),
        ("tags", tags),
        ("release_date", release.into()),
        ("series", series),
    ] {
        fields.insert(key.into(), value);
    }
    Ok((fields, target))
}

/// A show's episode as a flat `url_transparent` entry.
fn episode_entry(data: &Value) -> Result<VideoInfo> {
    let (fields, target) = parse_episode(data, None)?;
    let mut entry = Map::new();
    entry.insert("_type".into(), "url_transparent".into());
    entry.insert("url".into(), target.clone().into());
    entry.insert("ie_key".into(), MIXCLOUD.into());
    entry.extend(fields.clone());
    let url = super::target_url(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // `MixcloudIE.get_temp_id(url)`: its pattern has no `id` group.
    info.id = String::new();
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
            "ie_key": MIXCLOUD,
            "extractor": "mixcloud",
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// Whether a playlist's pending state is a Radio Kapitał show's pages.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == PENDING_KIND
}

/// `RadioKapitalShowIE._entries`: episode list pages from 1 as `Downloading
/// episode list page #N`, each item parsed as it is reached, until a page's
/// `next` is `None`; up to `limit` entries. An error raised while reading
/// them (`--no-abort-on-error`) ends the generator with a `None` entry.
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let Value::Object(mut state) = state else {
        return Ok(None);
    };
    let series_id = state
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut page = state.get("page").and_then(Value::as_i64).unwrap_or(1);
    // The last page's `next` (`{"value": ...}`, or `{}` when missing),
    // checked once its items are read.
    let mut next: Option<Value> = state.shift_remove("next");
    let mut queue: std::collections::VecDeque<Value> = match state.shift_remove("queue") {
        Some(Value::Array(queue)) => queue.into(),
        _ => Default::default(),
    };
    let mut done = false;
    let mut read = || -> Result<()> {
        while !limit.is_some_and(|limit| entries.len() >= limit) {
            if let Some(item) = queue.pop_front() {
                entries.push(episode_entry(&item)?);
                continue;
            }
            // `if episode_list['next'] is None: break`.
            if let Some(next) = next.take() {
                match next.get("value") {
                    None => return Err(key_error(None, "next")),
                    Some(Value::Null) => {
                        done = true;
                        break;
                    }
                    Some(_) => {}
                }
            }
            let episode_list = call_api(
                client,
                SHOW_NAME,
                "episodes",
                &series_id,
                &format!("Downloading episode list page #{page}"),
                &[("show", series_id.as_str()), ("page", &page.to_string())],
            )?;
            match episode_list.get("items") {
                Some(Value::Array(items)) => queue.extend(items.iter().cloned()),
                Some(_) => return Err(unsupported("episode lists that are not lists")),
                None => return Err(key_error(None, "items")),
            }
            next = Some(match episode_list.get("next") {
                Some(value) => json!({"value": value}),
                None => json!({}),
            });
            page += 1;
        }
        Ok(())
    };
    match read() {
        Ok(()) => {}
        Err(error @ (Error::Reported(_) | Error::ExtractorRequest { .. })) => {
            // Raised outside `extract()`, the error has no `[extractor]`
            // prefix.
            let text = error.to_string();
            let text = text
                .strip_prefix(&format!("[{SHOW_NAME}] "))
                .map_or_else(|| text.clone(), str::to_owned);
            let page_url = Url::parse(state["url"].as_str().unwrap_or_default())?;
            let mut entry = super::direct::build_at(page_url.clone(), &page_url, String::new());
            entry.extra.insert(crate::FETCH_ERROR.into(), text.into());
            entry
                .extra
                .insert(crate::FETCH_ERROR_COUNT.into(), entries.len().into());
            entries.push(entry);
            return Ok(None);
        }
        Err(error) => return Err(error),
    }
    if queue.is_empty() && done {
        return Ok(None);
    }
    state.insert("page".into(), page.into());
    state.insert("queue".into(), Value::Array(queue.into()));
    if let Some(next) = next {
        state.insert("next".into(), next);
    }
    Ok(Some(Value::Object(state)))
}

impl Extractor for RadioKapital {
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
        let video_id = found["id"].to_owned();
        start(NAME, url);
        let episode = call_api(
            client,
            NAME,
            &format!("episodes/{video_id}"),
            &video_id,
            "Downloading JSON metadata",
            &[],
        )?;
        let (fields, target) = parse_episode(&episode, Some(&video_id))?;
        // `ie_key='Mixcloud'`: processing extracts the URL with Mixcloud only.
        if !super::mixcloud_suitable(&target) {
            return Err(Error::Reported(format!(
                "No suitable extractor ({MIXCLOUD}) found for URL {target}"
            )));
        }
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
        Ok(Some(ExtractResult::Video(follow)))
    }
}

impl Extractor for RadioKapitalShow {
    fn key(&self) -> &'static str {
        SHOW_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SHOW_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let series_id = found["id"].to_owned();
        start(SHOW_NAME, url);
        let show = call_api(
            client,
            SHOW_NAME,
            &format!("shows/{series_id}"),
            &series_id,
            "Downloading show metadata",
            &[],
        )?;
        let id = match show.get("id") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) if id.is_i64() || id.is_u64() => id.to_string(),
            Some(_) => return Err(unsupported("show IDs of other types")),
            None => return Err(key_error(Some(&series_id), "id")),
        };
        let title = match show.get("title") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => Some(title.clone()),
            Some(_) => return Err(unsupported("show titles that are not strings")),
        };
        let description = match show.get("content") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(content)) => crate::html::clean_html(content).into(),
            Some(_) => return Err(unsupported("show contents that are not strings")),
        };
        let mut extra = Map::new();
        if title.is_none() {
            extra.insert("title".into(), Value::Null);
        }
        extra.insert("description".into(), description);
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;;after=id,title,description{}",
                crate::info_order::GENERATOR
            )
            .into(),
        );
        let state = json!({
            "kind": PENDING_KIND,
            "id": series_id,
            "url": url.as_str(),
            "page": 1,
        });
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: SHOW_NAME.into(),
            extractor_key: "RadioKapitalShow".into(),
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
        assert!(suitable(
            "https://radiokapital.pl/shows/tutaj-sa-smoki/5-its-okay-to-be-immaterial"
        ));
        assert!(!suitable("https://radiokapital.pl/shows/wesz"));
        assert!(show_suitable("https://radiokapital.pl/shows/wesz"));
        assert!(show_suitable("https://radiokapital.pl/shows/wesz/?x=1"));
        assert!(!show_suitable(
            "https://radiokapital.pl/shows/tutaj-sa-smoki/5-its-okay-to-be-immaterial"
        ));
    }

    #[test]
    fn release_dates_slice_like_python() {
        let (fields, _) = parse_episode(
            &json!({"published": "20.05.2021", "mixcloud_url": "u", "title": "t"}),
            None,
        )
        .unwrap();
        assert_eq!(fields["release_date"], "202105.20.");
    }
}
