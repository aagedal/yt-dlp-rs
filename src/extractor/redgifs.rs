//! Upstream `RedGifsIE` (`yt_dlp/extractor/redgifs.py`): a GIF's
//! `api.redgifs.com` metadata (behind a temporary bearer token) and its
//! `gif`/`sd`/`hd` MP4 renditions, and the search (`RedGifsSearchIE`) and
//! user (`RedGifsUserIE`) listings as paged playlists of those GIFs.
use super::{Extractor, dict_format, dict_info, download_json_note, download_json_note_headers};
use crate::{Error, ExtractResult, PendingEntries, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct RedGifs;
pub struct RedGifsSearch;
pub struct RedGifsUser;

const NAME: &str = "RedGifs";
const SEARCH_NAME: &str = "RedGifsSearch";
const USER_NAME: &str = "RedGifsUser";
const LIST_KIND: &str = "RedGifsList";
const TOKEN_URL: &str = "https://api.redgifs.com/v2/auth/temporary";
/// `_PAGE_SIZE` of the search and user listings.
const PAGE_SIZE: usize = 80;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:www\.)?redgifs\.com/(?:watch|ifr)/|thumbs2\.redgifs\.com/)(?P<id>[^-/?#.]+)",
    )
    .unwrap()
});

static SEARCH_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?redgifs\.com/browse\?(?P<query>[^#]+)")
        .unwrap()
});
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?redgifs\.com/users/(?P<username>[^/?#]+)(?:\?(?P<query>[^#]+))?",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `RedGifsSearchIE._VALID_URL`.
pub fn search_suitable(url: &str) -> bool {
    SEARCH_URL.is_match(url)
}

/// `RedGifsUserIE._VALID_URL`.
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

/// `_FORMATS` in order: format id and its height (`None` for the source).
const FORMATS: [(&str, Option<i64>); 3] = [("gif", Some(250)), ("sd", Some(480)), ("hd", None)];

/// `_fetch_oauth_token`.
fn fetch_token(client: &Client, video_id: &str) -> Result<String> {
    let url = Url::parse(TOKEN_URL)?;
    let auth = download_json_note(client, &url, NAME, video_id, "Fetching temporary token")?;
    match auth.get("token").and_then(Value::as_str) {
        Some(token) if !token.is_empty() => Ok(token.to_owned()),
        _ => Err(Error::Reported(format!(
            "Unable to get temporary token; {}",
            crate::BUG_REPORTS_MESSAGE
        ))),
    }
}

/// `_call_api(ep, video_id, ...)`: `url` is the endpoint with its query.
/// Headers follow `_API_HEADERS` plus the per-video `x-customheader`.
fn call_api(client: &Client, token: &str, url: &Url, video_id: &str, note: &str) -> Result<Value> {
    let watch = format!("https://www.redgifs.com/watch/{video_id}");
    let bearer = format!("Bearer {token}");
    let data = download_json_note_headers(
        client,
        url,
        NAME,
        video_id,
        note,
        &[
            ("referer", "https://www.redgifs.com/"),
            ("origin", "https://www.redgifs.com"),
            ("content-type", "application/json"),
            ("authorization", bearer.as_str()),
            ("x-customheader", watch.as_str()),
        ],
    )?;
    if let Some(error) = data.get("error") {
        return Err(Error::Reported(format!(
            "[{NAME}] {video_id}: RedGifs said: {}",
            error_text(error)
        )));
    }
    Ok(data)
}

/// `f'{data["error"]}'`.
fn error_text(error: &Value) -> String {
    match error {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `int_or_none(value)`.
fn int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `_parse_gif_data`.
fn parse_gif(gif: &Value) -> Result<VideoInfo> {
    let video_id = gif.get("id").and_then(Value::as_str).unwrap_or_default();
    let orig_height = int(gif.get("height"));
    let aspect_ratio = match (gif.get("width").and_then(Value::as_f64), orig_height) {
        (Some(width), Some(height)) if height != 0 => Some(width / height as f64),
        _ => None,
    };

    let mut formats = Vec::new();
    for (rank, (format_id, height)) in FORMATS.into_iter().enumerate() {
        let Some(video_url) = gif
            .get("urls")
            .and_then(|urls| urls.get(format_id))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let Some(orig_height) = orig_height else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: unsupported GIF without a height"
            )));
        };
        let height = orig_height.min(height.unwrap_or(orig_height));
        let width = aspect_ratio
            .filter(|ratio| *ratio != 0.0)
            .map(|ratio| Value::from((height as f64 * ratio) as i64))
            .unwrap_or(Value::Null);
        formats.push(dict_format(
            vec![
                ("url", video_url.into()),
                ("format_id", format_id.into()),
                ("width", width),
                ("height", height.into()),
                ("quality", (rank as i64).into()),
            ],
            NAME,
            video_id,
        )?);
    }

    let tags: Vec<Value> = gif
        .get("tags")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let title = tags
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    let title = if title.is_empty() {
        "RedGifs".to_owned()
    } else {
        title
    };
    let text = |key: &str| gif.get(key).and_then(Value::as_str).map(Value::from);
    let number = |key: &str| int(gif.get(key)).map_or(Value::Null, Value::from);
    let fields = vec![
        ("id", video_id.into()),
        (
            "webpage_url",
            format!("https://redgifs.com/watch/{video_id}").into(),
        ),
        ("extractor_key", NAME.into()),
        ("extractor", NAME.into()),
        ("title", title.into()),
        ("timestamp", number("createDate")),
        ("uploader", text("userName").unwrap_or(Value::Null)),
        ("duration", number("duration")),
        ("view_count", number("views")),
        ("like_count", number("likes")),
        ("categories", Value::Array(tags.clone())),
        ("tags", gif.get("tags").cloned().unwrap_or(Value::Null)),
        ("age_limit", 18.into()),
        ("formats", Value::Null),
    ];
    let page = Url::parse(&format!("https://redgifs.com/watch/{video_id}"))?;
    Ok(dict_info(&page, (NAME, NAME), fields, formats))
}

impl Extractor for RedGifs {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_lowercase();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let token = fetch_token(client, &video_id)?;
        let url = Url::parse(&format!(
            "https://api.redgifs.com/v2/gifs/{video_id}?views=yes"
        ))?;
        let info = call_api(client, &token, &url, &video_id, "Downloading video info")?;
        let gif = info
            .get("gif")
            .ok_or_else(|| Error::Reported(format!("[{NAME}] {video_id}: no GIF data")))?;
        Ok(Some(parse_gif(gif)?))
    }
}

/// `parse_qs(query)`: blank values dropped, in order.
fn parse_qs(query: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(query.as_bytes())
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

/// `query.get(key)[0]`.
fn first<'a>(query: &'a [(String, String)], key: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// `_fetch_page(ep, video_id, query, page)`: the API page `page + 1` of the
/// pending list's endpoint, its GIFs as entries.
fn fetch_page(client: &Client, state: &Value, page: i64) -> Result<Vec<VideoInfo>> {
    let endpoint = state["endpoint"].as_str().unwrap_or_default();
    let item_id = state["item_id"].as_str().unwrap_or_default();
    let mut url = Url::parse(&format!("https://api.redgifs.com/v2/{endpoint}"))?;
    {
        let mut pairs = url.query_pairs_mut();
        for pair in state["query"].as_array().into_iter().flatten() {
            if let (Some(key), Some(value)) = (pair[0].as_str(), pair[1].as_str()) {
                pairs.append_pair(key, value);
            }
        }
        pairs.append_pair("page", &(page + 1).to_string());
    }
    let token = fetch_token(client, item_id)?;
    let data = call_api(
        client,
        &token,
        &url,
        item_id,
        &format!("Downloading JSON metadata page {}", page + 1),
    )?;
    let gifs = data
        .get("gifs")
        .and_then(Value::as_array)
        .ok_or_else(|| super::clyp::key_error(item_id, "gifs"))?;
    gifs.iter().map(parse_gif).collect()
}

/// `_fetch_page` for the pending list's pages (`OnDemandPagedList`).
pub(crate) fn fetch_list_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let source = state.clone();
    let mut fetch = |page: usize| fetch_page(client, &source, page as i64);
    let placeholder = || parse_gif(&json!({}));
    super::paged::fetch_requested(state, limit, entries, PAGE_SIZE, &mut fetch, &placeholder)
}

/// Whether a playlist's pending state is a RedGifs listing's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == LIST_KIND
}

/// `playlist_result(_paged_entries(...), ...)`: a listing whose page
/// number is in the URL fetches that single page (`page + 1` on the API),
/// otherwise the pages are read on demand.
fn list_playlist(
    client: &Client,
    url: &Url,
    (name, id, title): (&'static str, &str, &str),
    description: String,
    (endpoint, query): (String, Vec<(String, String)>),
    url_page: Option<i64>,
) -> Result<ExtractResult> {
    let state = json!({
        "kind": LIST_KIND,
        "endpoint": endpoint,
        "item_id": id,
        "query": query,
        "random_access": true,
    });
    let (entries, pending) = match url_page.filter(|page| *page != 0) {
        Some(page) => (fetch_page(client, &state, page)?, None),
        None => (Vec::new(), Some(PendingEntries { state })),
    };
    let mut extra = Map::new();
    extra.insert("description".into(), description.into());
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "playlist-dict;id,title,description".into(),
    );
    Ok(ExtractResult::Playlist(PlaylistInfo {
        id: id.to_owned(),
        title: title.to_owned(),
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: name.into(),
        nested: Vec::new(),
        pending,
        entries,
        extra,
    }))
}

impl Extractor for RedGifsSearch {
    fn key(&self) -> &'static str {
        SEARCH_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SEARCH_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let query_str = found["query"].to_owned();
        crate::screen::set_extractor(SEARCH_NAME);
        crate::screen::screen(&format!(
            "[{SEARCH_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let query = parse_qs(&query_str);
        let Some(tags) = first(&query, "tags") else {
            return Err(Error::Reported(format!(
                "[{SEARCH_NAME}] Invalid query tags"
            )));
        };
        let order = first(&query, "order").unwrap_or("trending");
        // `_prepare_api_query` with `search_text` set to the tags.
        let mut api = vec![
            ("search_text".to_owned(), tags.to_owned()),
            ("order".to_owned(), order.to_owned()),
        ];
        if let Some(kind) = first(&query, "type") {
            api.push(("type".to_owned(), kind.to_owned()));
        }
        let url_page = first(&query, "page").and_then(|page| page.parse::<i64>().ok());
        list_playlist(
            client,
            url,
            (SEARCH_NAME, &query_str, tags),
            format!("RedGifs search for {tags}, ordered by {order}"),
            ("gifs/search".to_owned(), api),
            url_page,
        )
        .map(Some)
    }
}

impl Extractor for RedGifsUser {
    fn key(&self) -> &'static str {
        USER_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = USER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let username = found["username"].to_owned();
        let query_str = found.name("query").map(|query| query.as_str().to_owned());
        crate::screen::set_extractor(USER_NAME);
        crate::screen::screen(&format!(
            "[{USER_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let playlist_id = match &query_str {
            Some(query) => format!("{username}?{query}"),
            None => username.clone(),
        };
        let query = parse_qs(query_str.as_deref().unwrap_or_default());
        let order = first(&query, "order").unwrap_or("recent");
        let mut api = vec![("order".to_owned(), order.to_owned())];
        if let Some(kind) = first(&query, "type") {
            api.push(("type".to_owned(), kind.to_owned()));
        }
        let url_page = first(&query, "page").and_then(|page| page.parse::<i64>().ok());
        list_playlist(
            client,
            url,
            (USER_NAME, &playlist_id, &username),
            format!("RedGifs user {username}, ordered by {order}"),
            (format!("users/{username}/search"), api),
            url_page,
        )
        .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.redgifs.com/watch/squeakyhelplesswisent"
        ));
        assert!(suitable(
            "https://thumbs2.redgifs.com/SqueakyHelplessWisent-mobile.mp4#t=0"
        ));
        assert!(suitable(
            "https://www.redgifs.com/ifr/squeakyhelplesswisent"
        ));
        assert!(!suitable("https://www.redgifs.com/browse"));
    }

    #[test]
    fn list_urls_follow_upstream_valid_url() {
        assert!(search_suitable(
            "https://www.redgifs.com/browse?tags=Lesbian"
        ));
        assert!(!search_suitable("https://www.redgifs.com/users/lamsinka89"));
        assert!(user_suitable("https://www.redgifs.com/users/lamsinka89"));
        assert!(user_suitable(
            "https://www.redgifs.com/users/lamsinka89?page=3"
        ));
        assert!(!user_suitable(
            "https://www.redgifs.com/watch/squeakyhelplesswisent"
        ));
    }
}
