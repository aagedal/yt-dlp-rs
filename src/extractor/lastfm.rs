//! Upstream `LastFMIE`, `LastFMPlaylistIE` and `LastFMUserIE`
//! (`yt_dlp/extractor/lastfm.py`): a track page's YouTube player link as a
//! `url` result, and artist, tag and user playlist pages' `data-youtube-url`
//! links as a lazily paged playlist of YouTube `url` results.
use super::{Extractor, download_webpage, download_webpage_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct LastFm;
pub struct LastFmPlaylist;
pub struct LastFmUser;

const NAME: &str = "LastFM";
const PLAYLIST_NAME: &str = "LastFMPlaylist";
const USER_NAME: &str = "LastFMUser";
const PENDING_KIND: &str = "LastFM";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?last\.fm/music(?:/[^/]+){2}/(?P<id>[^/#?]+)")
        .unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?last\.fm/(music|tag)/(?P<id>[^/]+)(?:/[^/]+)?/?(?:[?#]|$)")
        .unwrap()
});
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?last\.fm/user/[^/]+/playlists/(?P<id>[^/#?]+)").unwrap()
});
static PLAYLINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)class="header-new-playlink"\s+href="([^"]+)""#).unwrap());
static YOUTUBE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-youtube-url="([^"]+)""#).unwrap());
static PY_INT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[+-]?\d+(?:_\d+)*$").unwrap());

/// `LastFMIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `LastFMPlaylistIE._match_id`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `LastFMUserIE._match_id`.
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// Python `int(text)` (surrounding whitespace, a sign and digit-separating
/// underscores allowed), or `None` (`int_or_none`).
fn py_int(text: &str) -> Option<i64> {
    let text = text.trim();
    PY_INT
        .is_match(text)
        .then(|| text.replace('_', "").parse().ok())
        .flatten()
}

/// `traverse_obj(parse_qs(url), ('page', -1, {int_or_none}))`.
fn single_page(url: &Url) -> Option<i64> {
    url.query_pairs()
        .filter(|(key, value)| key == "page" && !value.is_empty())
        .last()
        .and_then(|(_, value)| py_int(&value))
}

/// `url_result(url, 'Youtube')`.
fn youtube_entry(target: &str) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // `YoutubeIE.get_temp_id(url)`.
    info.id = super::youtube::temp_id(target).unwrap_or_default();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": "Youtube",
            "extractor": "youtube",
            "entry": {"ie_key": "Youtube", "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

/// Whether a playlist's pending state is a Last.fm playlist's pages.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == PENDING_KIND
}

/// `LastFMPlaylistBaseIE._entries` through `playlist_from_matches`: pages
/// from the URL's `page` (or 1) as `Downloading page N`, each page's
/// `data-youtube-url` links (first occurrences only, `orderedSet(lazy=True)`)
/// as YouTube `url` results, until a page has none or after the single
/// requested page; up to `limit` entries.
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let Value::Object(mut state) = state else {
        return Ok(None);
    };
    let name = if state.get("ie").and_then(Value::as_str) == Some(USER_NAME) {
        USER_NAME
    } else {
        PLAYLIST_NAME
    };
    let field = |key: &str| state.get(key).cloned().unwrap_or(Value::Null);
    let url = Url::parse(field("url").as_str().unwrap_or_default())?;
    let playlist_id = field("id").as_str().unwrap_or_default().to_owned();
    let single = field("single").as_bool().unwrap_or(false);
    let mut page = field("page").as_i64().unwrap_or(1);
    let mut done = field("done").as_bool().unwrap_or(false);
    let mut seen: Vec<Value> = match state.shift_remove("seen") {
        Some(Value::Array(seen)) => seen,
        _ => Vec::new(),
    };
    let mut queue: std::collections::VecDeque<Value> = match state.shift_remove("queue") {
        Some(Value::Array(queue)) => queue.into(),
        _ => Default::default(),
    };
    while !limit.is_some_and(|limit| entries.len() >= limit) {
        if let Some(item) = queue.pop_front() {
            if seen.contains(&item) {
                continue;
            }
            entries.push(youtube_entry(item.as_str().unwrap_or_default())?);
            seen.push(item);
            continue;
        }
        if done {
            break;
        }
        let page_url = super::update_url_query(&url, &[("page", &page.to_string())]);
        let webpage = download_webpage_note(
            client,
            &page_url,
            name,
            &playlist_id,
            &format!("Downloading page {page}"),
        )?;
        let videos: Vec<Value> = YOUTUBE_URL
            .captures_iter(&webpage)
            .map(|found| Value::from(&found[1]))
            .collect();
        done = single || videos.is_empty();
        queue.extend(videos);
        page += 1;
    }
    if queue.is_empty() && done {
        return Ok(None);
    }
    state.insert("page".into(), page.into());
    state.insert("done".into(), done.into());
    state.insert("seen".into(), Value::Array(seen));
    state.insert("queue".into(), Value::Array(queue.into()));
    Ok(Some(Value::Object(state)))
}

/// `LastFMPlaylistBaseIE._real_extract`: `playlist_from_matches(entries,
/// playlist_id, ie='Youtube')`, read lazily.
fn playlist(name: &'static str, url: &Url, playlist_id: String) -> ExtractResult {
    let first = single_page(url);
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;id{}", crate::info_order::GENERATOR).into(),
    );
    let state = json!({
        "kind": PENDING_KIND,
        "ie": name,
        "url": url.as_str(),
        "id": playlist_id,
        // `itertools.count(single_page or 1)`.
        "page": first.filter(|page| *page != 0).unwrap_or(1),
        "single": first.is_some_and(|page| page != 0),
    });
    ExtractResult::Playlist(PlaylistInfo {
        id: playlist_id,
        title: String::new(),
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: name.into(),
        nested: Vec::new(),
        pending: Some(crate::PendingEntries { state }),
        entries: Vec::new(),
        extra,
    })
}

impl Extractor for LastFm {
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
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let player_url = PLAYLINK
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract player_url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // `url_result(player_url, 'Youtube')`.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow
            .extra
            .insert(crate::FOLLOW_URL.into(), player_url.into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

impl Extractor for LastFmPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        start(PLAYLIST_NAME, url);
        Ok(Some(playlist(PLAYLIST_NAME, url, found["id"].to_owned())))
    }
}

impl Extractor for LastFmUser {
    fn key(&self) -> &'static str {
        USER_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = USER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        start(USER_NAME, url);
        Ok(Some(playlist(USER_NAME, url, found["id"].to_owned())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.last.fm/music/Oasis/_/Wonderwall"));
        assert!(suitable(
            "https://www.last.fm/music/Guns+N%27+Roses/_/Sweet+Child+o%27+Mine"
        ));
        assert!(!playlist_suitable(
            "https://www.last.fm/music/Oasis/_/Wonderwall"
        ));
        for url in [
            "https://www.last.fm/music/Oasis/(What%27s+the+Story)+Morning+Glory%3F",
            "https://www.last.fm/music/Oasis",
            "https://www.last.fm/music/Oasis/",
            "https://www.last.fm/music/Oasis?top_tracks_date_preset=ALL#top-tracks",
            "https://www.last.fm/music/Oasis/+tracks?page=2",
            "https://www.last.fm/tag/rock/tracks",
        ] {
            assert!(playlist_suitable(url), "{url}");
            assert!(!suitable(url), "{url}");
        }
        assert!(user_suitable(
            "https://www.last.fm/user/mehq/playlists/12319471"
        ));
        assert!(!playlist_suitable(
            "https://www.last.fm/user/mehq/playlists/12319471"
        ));
    }

    #[test]
    fn single_pages_follow_int_or_none() {
        let page = |url: &str| single_page(&Url::parse(url).unwrap());
        assert_eq!(
            page("https://www.last.fm/music/Oasis/+tracks?page=2"),
            Some(2)
        );
        assert_eq!(
            page("https://www.last.fm/music/Oasis?page=1&page=3"),
            Some(3)
        );
        assert_eq!(page("https://www.last.fm/music/Oasis?page=x"), None);
        assert_eq!(page("https://www.last.fm/music/Oasis?page=1_0"), Some(10));
        assert_eq!(page("https://www.last.fm/music/Oasis?page="), None);
    }
}
