//! Upstream `MonstercatIE` (`yt_dlp/extractor/monstercat.py`): a Monstercat
//! release page's track table as a playlist of `track-stream` MP3 entries
//! carrying the release's title, cover, artists and date.
use super::{Extractor, dict_info, download_webpage};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Monstercat;

const NAME: &str = "Monstercat";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://www\.monstercat\.com/release/(?P<id>\d{12}|MC[A-Z]+\d+)",
    )
    .unwrap()
});
static ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<tr[^<]*>").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `find_element(cls=...)`: the first element's content.
fn by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()
        .and_then(|found| found.into_iter().next())
        .map(|(text, _)| text)
}

/// `find_element(cls=..., html=True)`: the first element's source.
fn html_by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()
        .and_then(|found| found.into_iter().next())
        .map(|(_, whole)| whole)
}

/// `traverse_obj(html, ({find_element(...)}, {clean_html}, all))`.
fn all_clean(class: &str, html: &str) -> Value {
    Value::Array(
        by_class(class, html)
            .map(|text| crate::html::clean_html(&text))
            .into_iter()
            .map(Value::from)
            .collect(),
    )
}

/// `re.findall(r'<tr[^<]*>((?:(?!</tr>)[\w\W])+)', table)`.
fn rows(table: &str) -> Vec<&str> {
    let mut rows = Vec::new();
    let mut offset = 0;
    while let Some(found) = ROW.find(&table[offset..]) {
        let start = offset + found.end();
        let end = table[start..]
            .find("</tr>")
            .map_or(table.len(), |end| start + end);
        if end > start {
            rows.push(&table[start..end]);
            offset = end;
        } else {
            offset = start;
        }
        if offset >= table.len() {
            break;
        }
    }
    rows
}

/// Whether a playlist's pending state is a Monstercat release's tracks.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == NAME
}

/// One track row as an entry, or `None` (after upstream's warning) without
/// its IDs.
fn track_entry(
    page: &Url,
    row: &str,
    meta: &Map<String, Value>,
    url_id: &str,
) -> Result<Option<VideoInfo>> {
    let track_title = by_class("d-inline-flex flex-column", row)
        .map(|text| {
            let head = text
                .split_once(" <span")
                .map_or(text.as_str(), |(head, _)| head);
            crate::html::clean_html(head)
        })
        .map_or(Value::Null, Value::from);
    let ids = html_by_class("btn-play cursor-pointer mr-small", row)
        .map(|element| crate::html_elements::extract_attributes(&element))
        .unwrap_or_default();
    let text_attr = |key: &str| {
        ids.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let track_number = by_class("py-xsmall", row).and_then(|text| text.trim().parse::<i64>().ok());
    let (Some(track_id), Some(release_id)) =
        (text_attr("data-track-id"), text_attr("data-release-id"))
    else {
        crate::screen::warning(&format!(
            "[{NAME}] Skipping track {}, ID(s) not found",
            track_number.map_or_else(|| "None".to_owned(), |number| number.to_string())
        ));
        return Ok(None);
    };
    let stream =
        format!("https://www.monstercat.com/api/release/{release_id}/track-stream/{track_id}");
    // `{**album_meta, 'title': ..., ...}`: `title` keeps its place.
    let mut fields: Vec<(&str, Value)> = [
        "title",
        "album",
        "thumbnail",
        "album_artists",
        "release_date",
    ]
    .iter()
    .map(|key| (*key, meta.get(*key).cloned().unwrap_or(Value::Null)))
    .collect();
    fields[0].1 = track_title.clone();
    fields.extend([
        ("track", track_title),
        (
            "track_number",
            track_number.map_or(Value::Null, Value::from),
        ),
        ("artists", all_clean("d-block fs-xxsmall", row)),
        ("url", stream.clone().into()),
        ("id", track_id.into()),
        ("ext", "mp3".into()),
    ]);
    let mut format: crate::VideoFormat =
        serde_json::from_value(json!({"format_id": "", "url": stream, "ext": "mp3"}))
            .map_err(|error| crate::Error::Reported(format!("{url_id}: {error}")))?;
    crate::info_order::mark_format(&mut format, "url-only");
    let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
    let shape = format!("untyped-dict;{}", keys.join(","));
    Ok(Some(crate::info_order::mark_info(
        dict_info(page, (NAME, NAME), fields, vec![format]),
        &shape,
    )))
}

/// `_extract_tracks`: rows read until `limit` entries.
pub(crate) fn fetch_tracks(
    _client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let Value::Object(mut state) = state else {
        return Ok(None);
    };
    let page = Url::parse(state["url"].as_str().unwrap_or_default())?;
    let url_id = state["id"].as_str().unwrap_or_default().to_owned();
    let meta = state["meta"].as_object().cloned().unwrap_or_default();
    let mut rows: std::collections::VecDeque<Value> = match state.shift_remove("rows") {
        Some(Value::Array(rows)) => rows.into(),
        _ => Default::default(),
    };
    while !limit.is_some_and(|limit| entries.len() >= limit) {
        let Some(row) = rows.pop_front() else {
            break;
        };
        if let Some(entry) = track_entry(&page, row.as_str().unwrap_or_default(), &meta, &url_id)? {
            entries.push(entry);
        }
    }
    if rows.is_empty() {
        return Ok(None);
    }
    state.insert("rows".into(), Value::Array(rows.into()));
    Ok(Some(Value::Object(state)))
}

impl Extractor for Monstercat {
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
        let url_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let html = download_webpage(client, url, NAME, &url_id)?;
        let table = by_class("table table-small", &html)
            .filter(|table| !table.is_empty())
            .unwrap_or_default();
        let title = crate::html_elements::element_text_and_html_by_tag("h1", &html)
            .ok()
            .map(|(text, _)| crate::html::clean_html(&text))
            .map_or(Value::Null, Value::from);
        let release_date = by_class("font-italic mb-medium d-tablet-none d-phone-block", &html)
            .and_then(|text| {
                text.split_once("Released ")
                    .map(|(_, rest)| rest.trim().to_owned())
            })
            .and_then(|date| crate::dates::unified_strdate(&date, true))
            .map_or(Value::Null, Value::from);
        let album_meta = vec![
            ("title", title.clone()),
            ("album", title.clone()),
            (
                "thumbnail",
                format!("https://www.monstercat.com/release/{url_id}/cover").into(),
            ),
            (
                "album_artists",
                all_clean(
                    "h-normal text-uppercase mb-desktop-medium mb-smallish",
                    &html,
                ),
            ),
            ("release_date", release_date),
        ];
        // `_extract_tracks` is a generator: rows are read as entries are
        // requested (a skipped row warns only when reached).
        let row_items: Vec<Value> = rows(&table)
            .into_iter()
            .map(|row| Value::from(row.to_owned()))
            .collect();
        let meta: Map<String, Value> = album_meta
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect();
        // `playlist_result(entries, playlist_id=url_id, **album_meta)`.
        let mut extra = Map::new();
        let mut keys = Vec::new();
        for (key, value) in &album_meta {
            keys.push(*key);
            if *key != "title" || !value.is_string() {
                extra.insert((*key).into(), value.clone());
            }
        }
        keys.push("id");
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;{}{}",
                keys.join(","),
                crate::info_order::GENERATOR
            )
            .into(),
        );
        let url_id_text = url_id.clone();
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: url_id,
            title: title.as_str().unwrap_or_default().to_owned(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            nested: Vec::new(),
            pending: Some(crate::PendingEntries {
                state: json!({
                    "kind": NAME,
                    "url": url.as_str(),
                    "id": url_id_text,
                    "meta": meta,
                    "rows": row_items,
                }),
            }),
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
        assert!(suitable("https://www.monstercat.com/release/742779548009"));
        assert!(suitable("https://www.monstercat.com/release/MCRAB001"));
        assert!(!suitable("https://www.monstercat.com/release/abc"));
    }

    #[test]
    fn rows_end_at_the_closing_tag() {
        assert_eq!(
            rows("<tr class=x><td>a</td></tr><tr><td>b</td>"),
            ["<td>a</td>", "<td>b</td>"]
        );
        assert!(rows("<table></table>").is_empty());
    }
}
