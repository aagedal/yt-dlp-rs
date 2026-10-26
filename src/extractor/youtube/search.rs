//! Upstream `YoutubeSearchIE` (`ytsearch[N|all]:QUERY`) and
//! `YoutubeSearchURLIE` (`/results?search_query=`), from
//! `yt_dlp/extractor/youtube/_search.py` and `_tab.py`'s `_search_results`.
use super::tab::{at, entry_info, extract_entries};
use super::watch;
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub(crate) const NAME: &str = "youtube:search";
pub(crate) const URL_NAME: &str = "youtube:search_url";
/// `_SEARCH_PARAMS`: videos only.
const SEARCH_PARAMS: &str = "EgIQAfABAQ==";

static SEARCH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^ytsearch(?P<prefix>|[1-9][0-9]*|all):(?P<query>[\s\S]+)$").unwrap()
});
static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?youtube\.com/(?:results|search)\?([^#]+&)?(?:search_query|q)=(?:[^&]+)(?:[&#]|$)")
        .unwrap()
});

/// The input text of a `ytsearch` pseudo-URL (the URL parser
/// percent-encodes non-ASCII characters, which are decoded again).
pub(crate) fn input_text(url: &Url) -> String {
    let text = url.as_str();
    let mut bytes = Vec::with_capacity(text.len());
    let raw = text.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%'
            && let Some(byte) = text
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .filter(|byte| *byte >= 0x80)
        {
            bytes.push(byte);
            index += 3;
            continue;
        }
        bytes.push(raw[index]);
        index += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `YoutubeSearchIE.suitable`.
pub fn suitable(url: &str) -> bool {
    SEARCH_RE.is_match(url)
}

/// `YoutubeSearchURLIE.suitable`.
pub fn url_suitable(url: &str) -> bool {
    URL_RE.is_match(url)
}

/// `_search_results(query, params, default_client)`, optionally limited to
/// `max` results (`islice`), as the state of upstream's generator: nothing
/// is requested until the playlist's entries are read.
fn search_state(
    extractor: &'static str,
    client_name: &str,
    query: &str,
    params: Option<&str>,
    max: Option<usize>,
) -> Value {
    let mut data = Map::new();
    data.insert("query".into(), query.into());
    if let Some(params) = params.filter(|params| !params.is_empty()) {
        data.insert("params".into(), params.into());
    }
    json!({
        "kind": "search",
        "extractor": extractor,
        "client": client_name,
        "display_id": format!("query \"{query}\""),
        "data": data,
        "max": max,
        "count": 0,
        "page": 1,
        "ytcfg": null,
        "previous_visitor": null,
        "done": false,
    })
}

/// The search API pages of a [`search_state`] until `limit` entries are
/// read (or its end): the remaining state unless read past its end.
pub(crate) fn fetch_pages(
    client: &Client,
    mut state: Value,
    limit: Option<usize>,
    out: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let extractor = match state["extractor"].as_str() {
        Some(URL_NAME) => URL_NAME,
        Some(MUSIC_URL_NAME) => MUSIC_URL_NAME,
        _ => NAME,
    };
    let client_name = state["client"].as_str().unwrap_or("web").to_owned();
    let display_id = state["display_id"].as_str().unwrap_or_default().to_owned();
    let max = state["max"].as_u64().map(|max| max as usize);
    loop {
        let mut count = state["count"].as_u64().unwrap_or(0) as usize;
        if limit.is_some_and(|limit| out.len() >= limit) {
            return Ok(Some(state));
        }
        if state["done"] == true || max.is_some_and(|max| count >= max) {
            return Ok(None);
        }
        if state["ytcfg"].is_null() {
            // `_download_ytcfg(default_client, display_id) if not
            // self.skip_webpage else {}`.
            let (home, note) = match client_name.as_str() {
                "web_music" => (
                    "https://music.youtube.com",
                    "Downloading web music client config",
                ),
                _ => ("https://www.youtube.com", "Downloading web client config"),
            };
            let home = Url::parse(home)?;
            let ytcfg = if super::tab::skip_webpage() {
                Map::new()
            } else {
                match super::super::download_webpage_note(
                    client,
                    &home,
                    extractor,
                    &display_id,
                    note,
                ) {
                    Ok(webpage) => watch::extract_ytcfg(&webpage),
                    Err(error) => {
                        crate::screen::warning(&error.to_string());
                        Map::new()
                    }
                }
            };
            super::tab::report_playlist_authcheck(&ytcfg, false, extractor, &display_id)?;
            state["ytcfg"] = Value::Object(ytcfg);
        }
        let ytcfg = state["ytcfg"].as_object().cloned().unwrap_or_default();
        let visitor = state["previous_visitor"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| watch::visitor_from(&state["ytcfg"]));
        let page = state["page"].as_u64().unwrap_or(1);
        let mut data = state["data"].as_object().cloned().unwrap_or_default();
        let search = watch::innertube_api_with(
            client,
            &client_name,
            "search",
            extractor,
            &format!("{display_id} page {page}"),
            &ytcfg,
            visitor.as_deref(),
            data.clone(),
            "Downloading API JSON",
        )?;
        let contents = [
            &[
                "contents",
                "twoColumnSearchResultsRenderer",
                "primaryContents",
                "sectionListRenderer",
                "contents",
            ][..],
            &[
                "onResponseReceivedCommands",
                "0",
                "appendContinuationItemsAction",
                "continuationItems",
            ][..],
            &[
                "contents",
                "tabbedSearchResultsRenderer",
                "tabs",
                "0",
                "tabRenderer",
                "content",
                "sectionListRenderer",
                "contents",
            ][..],
            &["continuationContents"][..],
        ]
        .iter()
        .find_map(|path| at(&search, path).filter(|value| !value.is_null()))
        .cloned()
        .unwrap_or(Value::Null);
        // `variadic`: a list stays a list.
        let contents = match contents {
            Value::Array(items) => items,
            Value::Null => Vec::new(),
            other => vec![other],
        };
        let mut found = Vec::new();
        let continuation = extract_entries(&json!({ "contents": contents }), &mut found);
        for entry in found {
            if max.is_some_and(|max| count >= max) {
                break;
            }
            out.push(entry_info(entry)?);
            count += 1;
        }
        state["count"] = count.into();
        state["page"] = (page + 1).into();
        match continuation {
            Some(continuation) => {
                data.extend(continuation);
                state["data"] = Value::Object(data);
                state["previous_visitor"] = watch::visitor_from(&search).into();
            }
            None => state["done"] = true.into(),
        }
    }
}

/// A search playlist whose entries are fetched as they are read.
fn lazy_search_playlist(
    state: Value,
    title: &str,
    webpage_url: Url,
    extractor: &'static str,
    key: &'static str,
) -> Result<ExtractResult> {
    let mut result = search_playlist(Vec::new(), title, webpage_url, extractor, key)?;
    if let ExtractResult::Playlist(playlist) = &mut result {
        playlist.pending = Some(crate::PendingEntries { state });
    }
    Ok(result)
}

/// `playlist_result(entries, query, query)`.
pub(super) fn search_playlist(
    entries: Vec<Map<String, Value>>,
    query: &str,
    webpage_url: Url,
    extractor: &'static str,
    key: &'static str,
) -> Result<ExtractResult> {
    let mut entries = entries
        .into_iter()
        .map(entry_info)
        .collect::<Result<Vec<_>>>()?;
    // An entry without an extractor shows the search's.
    for entry in &mut entries {
        if let Some(marker) = entry.extra.get_mut(crate::TRANSPARENT) {
            marker["playlist_extractor"] = serde_json::json!([extractor, key]);
        }
    }
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "youtube-tab-plain;id,title".into(),
    );
    Ok(ExtractResult::Playlist(crate::PlaylistInfo {
        id: query.to_owned(),
        title: query.to_owned(),
        webpage_url,
        extractor: extractor.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    }))
}

pub(super) fn announce(extractor: &'static str, url: &str) {
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(url)
    ));
}

pub struct YoutubeSearch;

impl super::super::Extractor for YoutubeSearch {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<crate::VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let text = input_text(url);
        let Some(found) = SEARCH_RE.captures(&text) else {
            return Ok(None);
        };
        super::login(NAME, "")?;
        announce(NAME, &text);
        super::initialize();
        let query = found["query"].to_owned();
        // `SearchInfoExtractor._real_extract` (`_MAX_RESULTS` is unbounded).
        let limit = match &found["prefix"] {
            "" => Some(1),
            "all" => None,
            count => count.parse().ok().or(Some(usize::MAX)),
        };
        let state = search_state(NAME, "web", &query, Some(SEARCH_PARAMS), limit);
        lazy_search_playlist(state, &query, url.clone(), NAME, "YoutubeSearch").map(Some)
    }
}

pub struct YoutubeSearchUrl;

impl super::super::Extractor for YoutubeSearchUrl {
    fn key(&self) -> &'static str {
        URL_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<crate::VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !url_suitable(url.as_str()) {
            return Ok(None);
        }
        super::login(URL_NAME, "")?;
        announce(URL_NAME, url.as_str());
        super::initialize();
        // `parse_qs`: blank values are dropped.
        let first = |key: &str| {
            url.query_pairs()
                .find(|(name, value)| name == key && !value.is_empty())
                .map(|(_, value)| value.into_owned())
        };
        let query = first("search_query")
            .or_else(|| first("q"))
            .unwrap_or_default();
        let params = first("sp");
        let state = search_state(URL_NAME, "web", &query, params.as_deref(), None);
        lazy_search_playlist(state, &query, url.clone(), URL_NAME, "YoutubeSearchURL").map(Some)
    }
}

pub(crate) const MUSIC_URL_NAME: &str = "youtube:music:search_url";

static MUSIC_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://music\.youtube\.com/search\?([^#]+&)?(?:search_query|q)=(?:[^&]+)(?:[&#]|$)",
    )
    .unwrap()
});

/// `YoutubeMusicSearchURLIE._SECTIONS`.
const MUSIC_SECTIONS: [(&str, &str); 6] = [
    ("albums", "EgWKAQIYAWoKEAoQAxAEEAkQBQ=="),
    ("artists", "EgWKAQIgAWoKEAoQAxAEEAkQBQ=="),
    ("community playlists", "EgeKAQQoAEABagoQChADEAQQCRAF"),
    ("featured playlists", "EgeKAQQoADgBagwQAxAJEAQQDhAKEAU=="),
    ("songs", "EgWKAQIIAWoKEAoQAxAEEAkQBQ=="),
    ("videos", "EgWKAQIQAWoKEAoQAxAEEAkQBQ=="),
];

/// `YoutubeMusicSearchURLIE.suitable`.
pub fn music_url_suitable(url: &str) -> bool {
    MUSIC_URL_RE.is_match(url)
}

/// `YoutubeMusicSearchURLIE`: a YouTube Music search, optionally of one
/// section (`sp`, or a `#section` fragment), with the `web_music` client.
pub struct YoutubeMusicSearchUrl;

impl super::super::Extractor for YoutubeMusicSearchUrl {
    fn key(&self) -> &'static str {
        MUSIC_URL_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<crate::VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let text = url.as_str();
        if !music_url_suitable(text) {
            return Ok(None);
        }
        super::login(MUSIC_URL_NAME, "")?;
        announce(MUSIC_URL_NAME, text);
        super::initialize();
        let first = |key: &str| super::urls::first_qs(text, key);
        let query = first("search_query")
            .or_else(|| first("q"))
            .unwrap_or_default();
        let mut params = first("sp");
        let section = match &params {
            Some(params) => Some(
                MUSIC_SECTIONS
                    .iter()
                    .find(|(_, value)| value == params)
                    .map_or(params.as_str(), |(name, _)| name)
                    .to_owned(),
            ),
            None => {
                // `unquote_plus(([*url.split('#'), ''])[1]).lower()`.
                let fragment = text.split('#').nth(1).unwrap_or("").replace('+', " ");
                let section = percent_encoding::percent_decode_str(&fragment)
                    .decode_utf8_lossy()
                    .to_lowercase();
                params = MUSIC_SECTIONS
                    .iter()
                    .find(|(name, _)| *name == section)
                    .map(|(_, value)| (*value).to_owned());
                params.is_some().then_some(section)
            }
        };
        // `join_nonempty(query, section, delim=' - ')`.
        let title = [Some(query.clone()), section]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" - ");
        let state = search_state(MUSIC_URL_NAME, "web_music", &query, params.as_deref(), None);
        lazy_search_playlist(
            state,
            &title,
            url.clone(),
            MUSIC_URL_NAME,
            "YoutubeMusicSearchURL",
        )
        .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_inputs_follow_upstream_patterns() {
        assert!(suitable("ytsearch:cats"));
        assert!(suitable("ytsearch10:cats and dogs"));
        assert!(suitable("ytsearchall:cats"));
        assert!(!suitable("ytsearch0:cats"));
        assert!(!suitable("ytsearch:"));
        assert!(url_suitable(
            "https://www.youtube.com/results?search_query=cats"
        ));
        assert!(url_suitable(
            "https://youtube.com/search?x=1&q=cats&sp=EgIQAg%253D%253D"
        ));
        assert!(!url_suitable(
            "https://www.youtube.com/results?search_query="
        ));
        let url = Url::parse("ytsearch5:caf\u{e9} %41").unwrap();
        assert_eq!(input_text(&url), "ytsearch5:caf\u{e9} %41");
    }
}
