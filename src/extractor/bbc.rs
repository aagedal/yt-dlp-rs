//! Upstream `BBCCoUkPlaylistIE` (`yt_dlp/extractor/bbc.py`): a programme's
//! clips, episodes or broadcasts page: its `data-pid` programmes as `BBCCoUk`
//! url entries, following the `pagination_next` links (a `page` query
//! parameter stops after that page). `BBCCoUkIPlayerEpisodesIE` and
//! `BBCCoUkIPlayerGroupIE`: a programme's episodes (iBL GraphQL API) or a
//! group's episodes (iBL v1 API) as `BBCCoUk` url entries, an
//! `OnDemandPagedList` or one explicit `page`. `BBCCoUkArticleIE`: an
//! article's `Clip` resources as url entries. `BBCIE` and `BBCCoUkIE` are
//! in `bbccouk.rs`.
use super::{Extractor, download_webpage, download_webpage_note, search_warning};
use crate::extractor::UrlRegex;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BbcCoUkPlaylist;

const NAME: &str = "bbc.co.uk:playlist";
const KEY: &str = "BBCCoUkPlaylist";
/// `BBCCoUkIE.ie_key()`.
const ENTRY_KEY: &str = "BBCCoUk";
/// `BBCCoUkIE._ID_REGEX`.
const ID_REGEX: &str = "(?:[pbml][0-9a-z]{7}|w[0-9a-z]{7,14})";

static VALID_URL: LazyLock<UrlRegex> = LazyLock::new(|| {
    UrlRegex::new(format!(
        r"^https?://(?:www\.)?bbc\.co\.uk/programmes/(?P<id>{ID_REGEX})/(?:episodes|broadcasts|clips)"
    ))
});
static VIDEO_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r#"data-pid=["']({ID_REGEX})"#)).unwrap());
/// `_search_regex(<li ... class="pagination_+next" ... href=...)`, group 1 or 2.
static NEXT_PAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<li[^>]+class=(?:"pagination_+next"|'pagination_+next')[^>]*><a[^>]+href=(?:"([^"]+)"|'([^']+)')"#,
    )
    .unwrap()
});

/// `BBCCoUkPlaylistIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `url_result('http://www.bbc.co.uk/programmes/ID', ie=BBCCoUkIE)` as a playlist entry.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ENTRY_KEY.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ENTRY_KEY,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for BbcCoUkPlaylist {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let mut webpage = download_webpage(client, url, NAME, &playlist_id)?;

        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            search_warning(NAME, "OpenGraph title");
        }
        let description = crate::html::og_search_property(&webpage, "description");

        // `'page' in parse_qs(query)`: a non-blank `page` stops after the first page.
        let single_page = url
            .query_pairs()
            .any(|(key, value)| key == "page" && !value.is_empty());
        let mut entries = Vec::new();
        for page_num in 2.. {
            let mut seen = Vec::new();
            for found in VIDEO_ID.captures_iter(&webpage) {
                seen.push(found[1].to_owned());
            }
            for video_id in seen {
                entries.push(url_entry(&format!(
                    "http://www.bbc.co.uk/programmes/{video_id}"
                ))?);
            }
            if single_page {
                break;
            }
            let Some(next) = NEXT_PAGE
                .captures(&webpage)
                .and_then(|found| found.get(1).or_else(|| found.get(2)))
                .map(|found| found.as_str().to_owned())
            else {
                break;
            };
            let next_url = url.join(&next)?;
            webpage = download_webpage_note(
                client,
                &next_url,
                NAME,
                &playlist_id,
                &format!("Downloading page {page_num}"),
            )?;
        }

        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        if description.is_some() {
            keys.push("description");
        }
        let mut extra = Map::new();
        if let Some(description) = description {
            extra.insert("description".into(), description.into());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// Upstream `BBCCoUkIPlayerEpisodesIE`.
pub struct BbcCoUkIPlayerEpisodes;
/// Upstream `BBCCoUkIPlayerGroupIE`.
pub struct BbcCoUkIPlayerGroup;

/// What `BBCCoUkIPlayerEpisodesIE` and `BBCCoUkIPlayerGroupIE` set for
/// their shared `BBCCoUkIPlayerPlaylistBaseIE`.
struct IPlayer {
    name: &'static str,
    key: &'static str,
    /// The `iplayer/KIND/` path segment.
    kind: &'static str,
    /// `_PAGE_SIZE`.
    page_size: usize,
    /// `_DESCRIPTION_KEY`.
    description_key: &'static str,
}

const EPISODES: IPlayer = IPlayer {
    name: "bbc.co.uk:iplayer:episodes",
    key: "BBCCoUkIPlayerEpisodes",
    kind: "episodes",
    page_size: 100,
    description_key: "synopsis",
};

const GROUP: IPlayer = IPlayer {
    name: "bbc.co.uk:iplayer:group",
    key: "BBCCoUkIPlayerGroup",
    kind: "group",
    page_size: 200,
    description_key: "synopses",
};

/// `per_page` for an explicit `page` query parameter.
const EXPLICIT_PAGE_SIZE: i64 = 36;

static EPISODES_URL: LazyLock<UrlRegex> = LazyLock::new(|| {
    UrlRegex::new(format!(
        r"^https?://(?:www\.)?bbc\.co\.uk/iplayer/episodes/(?P<id>{ID_REGEX})"
    ))
});
static GROUP_URL: LazyLock<UrlRegex> = LazyLock::new(|| {
    UrlRegex::new(format!(
        r"^https?://(?:www\.)?bbc\.co\.uk/iplayer/group/(?P<id>{ID_REGEX})"
    ))
});

/// `BBCCoUkIPlayerEpisodesIE._match_valid_url`.
pub fn iplayer_episodes_suitable(url: &str) -> bool {
    EPISODES_URL.is_match(url)
}

/// `BBCCoUkIPlayerGroupIE._match_valid_url`.
pub fn iplayer_group_suitable(url: &str) -> bool {
    GROUP_URL.is_match(url)
}

/// A Python exception the iPlayer code raises, before it is reported (in
/// `extract()`, or while the playlist's entries are read).
enum Raised {
    /// A `KeyError`'s key.
    Key(String),
    /// Another exception's message (`AttributeError`, `TypeError`).
    Other(String),
    Native(Error),
}

impl From<Error> for Raised {
    fn from(error: Error) -> Self {
        Self::Native(error)
    }
}

impl From<url::ParseError> for Raised {
    fn from(error: url::ParseError) -> Self {
        Self::Native(error.into())
    }
}

impl Raised {
    /// Raised in `extract()`: a `KeyError` is an extractor error with the
    /// URL's ID; other exceptions print bare.
    fn in_extract(self, id: &str) -> Error {
        match self {
            Self::Key(key) => Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError({})); {}",
                crate::output::traverse::py_repr_str(&key),
                crate::BUG_REPORTS_MESSAGE
            )),
            Self::Other(text) => Error::Reported(text),
            Self::Native(error) => error,
        }
    }

    /// Raised while reading the entries (`--no-abort-on-error`): the
    /// exception's `str()`.
    fn in_entries(self) -> Error {
        match self {
            Self::Key(key) => Error::Reported(crate::output::traverse::py_repr_str(&key)),
            Self::Other(text) => Error::Reported(text),
            Self::Native(error) => error,
        }
    }
}

fn unsupported(what: &str) -> Raised {
    Raised::Native(Error::UnsupportedHtml5(format!(
        "BBC iPlayer {what} are not supported natively"
    )))
}

/// `value[key]`.
fn subscript<'a>(value: &'a Value, key: &str) -> std::result::Result<&'a Value, Raised> {
    match value {
        Value::Object(map) => map.get(key).ok_or_else(|| Raised::Key(key.to_owned())),
        Value::Array(_) => Err(Raised::Other(
            "list indices must be integers or slices, not str".into(),
        )),
        Value::String(_) => Err(Raised::Other(
            "string indices must be integers, not 'str'".into(),
        )),
        _ => Err(Raised::Other(format!(
            "'{}' object is not subscriptable",
            crate::format_select::type_name(value)
        ))),
    }
}

/// `value.get(key)`.
fn get<'a>(value: &'a Value, key: &str) -> std::result::Result<Option<&'a Value>, Raised> {
    match value {
        Value::Object(map) => Ok(map.get(key).filter(|value| !value.is_null())),
        _ => Err(Raised::Other(format!(
            "'{}' object has no attribute 'get'",
            crate::format_select::type_name(value)
        ))),
    }
}

/// `try_get(value, lambda x: x[KEY][DEFAULT_KEY])`: `None` for any
/// lookup error.
fn get_default<'a>(value: &'a Value, key: &str, default_key: &str) -> Option<&'a Value> {
    value
        .get(key)
        .filter(|value| value.is_object())?
        .get(default_key)
        .filter(|value| !value.is_null())
}

/// The value as an entry field (`None` as `null`).
fn field(value: Option<&Value>) -> Value {
    value.cloned().unwrap_or(Value::Null)
}

impl IPlayer {
    fn valid_url(&self) -> &'static UrlRegex {
        if self.kind == "episodes" {
            &EPISODES_URL
        } else {
            &GROUP_URL
        }
    }

    fn episodes(&self) -> bool {
        self.kind == "episodes"
    }

    /// `_call_api(pid, per_page, page, series_id)`.
    fn call_api(
        &self,
        client: &Client,
        pid: &str,
        per_page: i64,
        page: i64,
        series_id: Option<&str>,
    ) -> std::result::Result<Value, Raised> {
        let data = if self.episodes() {
            let mut variables = Map::new();
            variables.insert("id".into(), pid.into());
            variables.insert("page".into(), page.into());
            variables.insert("perPage".into(), per_page.into());
            if let Some(series_id) = series_id {
                variables.insert("sliceId".into(), series_id.into());
            }
            let body = crate::output::python_json_dumps(&json!({
                "id": "5692d93d5aac8d796a0305e895e61551",
                "variables": Value::Object(variables),
            }));
            super::post_json_note_headers(
                client,
                &Url::parse("https://graph.ibl.api.bbc.co.uk/")?,
                self.name,
                pid,
                "Downloading JSON metadata",
                &[("Content-Type", "application/json")],
                body,
            )?
        } else {
            let mut api = Url::parse(&format!(
                "http://ibl.api.bbc.co.uk/ibl/v1/groups/{pid}/episodes"
            ))?;
            api.query_pairs_mut()
                .append_pair("page", &page.to_string())
                .append_pair("per_page", &per_page.to_string());
            super::download_json(client, &api, self.name, pid)?
        };
        if self.episodes() {
            Ok(subscript(subscript(&data, "data")?, "programme")?.clone())
        } else {
            Ok(subscript(&data, "group_episodes")?.clone())
        }
    }

    /// `_get_description(data)`: the first truthy of the synopses' `large`,
    /// `medium` and `small`.
    fn description(&self, data: &Value) -> std::result::Result<Option<Value>, Raised> {
        let synopsis = get(data, self.description_key)?;
        let Some(synopsis) = synopsis.filter(|synopsis| super::nuxt::truthy(synopsis)) else {
            return Ok(None);
        };
        let Value::Object(synopsis) = synopsis else {
            return Err(unsupported("synopses that are not objects"));
        };
        Ok(["large", "medium", "small"]
            .iter()
            .filter_map(|key| synopsis.get(*key))
            .find(|value| super::nuxt::truthy(value))
            .cloned())
    }

    /// `_fetch_page(programme_id, per_page, series_id, page)`, read to its
    /// end or its exception (after the entries before it).
    fn fetch_page(
        &self,
        client: &Client,
        pid: &str,
        per_page: i64,
        series_id: Option<&str>,
        page: i64,
        entries: &mut Vec<VideoInfo>,
    ) -> std::result::Result<(), Raised> {
        let data = self.call_api(client, pid, per_page, page + 1, series_id)?;
        let elements = if self.episodes() {
            subscript(subscript(&data, "entities")?, "results")?
        } else {
            subscript(&data, "elements")?
        };
        let elements = match elements {
            Value::Array(elements) => elements,
            Value::Null => {
                return Err(Raised::Other("'NoneType' object is not iterable".into()));
            }
            _ => return Err(unsupported("element lists that are not lists")),
        };
        let empty = Value::Object(Map::new());
        for element in elements {
            let episode = if self.episodes() {
                get(element, "episode")?
                    .filter(|episode| super::nuxt::truthy(episode))
                    .unwrap_or(&empty)
            } else {
                element
            };
            let Some(episode_id) = get(episode, "id")?.filter(|id| super::nuxt::truthy(id)) else {
                continue;
            };
            let image = if self.episodes() {
                get_default(episode, "image", "default")
            } else {
                get_default(episode, "images", "standard")
            };
            let thumbnail = match image.filter(|image| super::nuxt::truthy(image)) {
                Some(Value::String(image)) => Value::from(image.replace("{recipe}", "raw")),
                Some(image) => {
                    return Err(Raised::Other(format!(
                        "'{}' object has no attribute 'replace'",
                        crate::format_select::type_name(image)
                    )));
                }
                None => Value::Null,
            };
            let categories = match get_default(episode, "labels", "category") {
                Some(category) if super::nuxt::truthy(category) => json!([category]),
                _ => Value::Null,
            };
            let Value::String(episode_id) = episode_id else {
                return Err(Raised::Other(format!(
                    "can only concatenate str (not \"{}\") to str",
                    crate::format_select::type_name(episode_id)
                )));
            };
            let (title, series) = if self.episodes() {
                (
                    get_default(episode, "subtitle", "default"),
                    get_default(episode, "title", "default"),
                )
            } else {
                (get(episode, "subtitle")?, get(episode, "title")?)
            };
            let description = self.description(episode)?;
            let target = format!("https://www.bbc.co.uk/iplayer/episode/{episode_id}");
            let mut entry = Map::new();
            entry.insert("_type".into(), "url".into());
            entry.insert("id".into(), episode_id.as_str().into());
            entry.insert("title".into(), field(title));
            entry.insert("url".into(), target.as_str().into());
            entry.insert("thumbnail".into(), thumbnail);
            entry.insert("description".into(), description.unwrap_or(Value::Null));
            entry.insert("categories".into(), categories);
            entry.insert("series".into(), field(series));
            entry.insert("ie_key".into(), ENTRY_KEY.into());
            entries.push(self.episode_entry(&target, episode_id, entry)?);
        }
        Ok(())
    }

    /// An episode's `url` result.
    fn episode_entry(
        &self,
        target: &str,
        id: &str,
        entry: Map<String, Value>,
    ) -> Result<VideoInfo> {
        let url = Url::parse(target)?;
        let mut info = super::direct::build_at(url.clone(), &url, String::new());
        info.formats.clear();
        info.direct = None;
        info.id = id.to_owned();
        if let Some(Value::String(title)) = entry.get("title") {
            info.title = title.clone();
        }
        info.extra.insert(
            crate::TRANSPARENT.into(),
            json!({
                "url": target,
                "force_videoid": null,
                "fields": {},
                "ie_key": ENTRY_KEY,
                "extractor": self.name,
                "entry": Value::Object(entry),
            }),
        );
        Ok(info)
    }

    fn extract_playlist(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = self.valid_url().captures(url.as_str()) else {
            return Ok(None);
        };
        let pid = found["id"].to_owned();
        crate::screen::set_extractor(self.name);
        crate::screen::screen(&format!(
            "[{}] Extracting URL: {}",
            self.name,
            crate::screen::extracting_url(url.as_str())
        ));
        // `parse_qs(url)` drops blank values.
        let query = |name: &str| {
            url.query_pairs()
                .find(|(key, value)| key == name && !value.is_empty())
                .map(|(_, value)| value.into_owned())
        };
        let series_id = query("seriesId");
        let page = query("page").map(|page| python_int(&page)).transpose()?;
        let data = self
            .call_api(client, &pid, 1, 1, None)
            .map_err(|raised| raised.in_extract(&pid))?;
        let playlist_data = if self.episodes() {
            data
        } else {
            subscript(&data, "group")
                .map_err(|raised| raised.in_extract(&pid))?
                .clone()
        };
        let title = if self.episodes() {
            get_default(&playlist_data, "title", "default")
        } else {
            get(&playlist_data, "title").map_err(|raised| raised.in_extract(&pid))?
        };
        let title = match title.filter(|title| super::nuxt::truthy(title)) {
            Some(Value::String(title)) => Some(title.clone()),
            Some(_) => {
                return Err(unsupported("titles that are not strings").in_extract(&pid));
            }
            None => None,
        };
        let description = self
            .description(&playlist_data)
            .map_err(|raised| raised.in_extract(&pid))?;

        // `fetch_page(int(page) - 1)` is a generator read after extraction;
        // otherwise an `OnDemandPagedList`.
        let state = match page {
            Some(page) => json!({
                "kind": self.key,
                "pid": pid,
                "series_id": series_id,
                "page": page,
                "url": url.as_str(),
            }),
            None => json!({
                "kind": self.key,
                "pid": pid,
                "series_id": series_id,
                "random_access": true,
            }),
        };
        let pending = Some(crate::PendingEntries { state });

        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        let mut extra = Map::new();
        if let Some(description) = description {
            keys.push("description");
            extra.insert("description".into(), description);
        }
        // Both lists' full counts are only known once a selection reads
        // past their end.
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
            id: pid,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: self.name.into(),
            extractor_key: self.key.into(),
            nested: Vec::new(),
            pending,
            entries: Vec::new(),
            extra,
        })))
    }
}

/// An exception's text as reported while reading entries: outside
/// `extract()` it has no `[extractor]` prefix.
fn generator_error_text(error: Error) -> Result<String> {
    // `_handle_extraction_exceptions` re-raises a cancellation.
    if matches!(error, Error::UnsupportedHtml5(_) | Error::DownloadCancelled) {
        return Err(error);
    }
    Ok(super::paged::entries_error_text(&error))
}

/// `int(page)`.
fn python_int(text: &str) -> Result<i64> {
    crate::dates::py_int_checked(text).map_err(|error| match error {
        crate::dates::PyIntError::Invalid => Error::Reported(format!(
            "invalid literal for int() with base 10: {}",
            crate::output::traverse::py_repr_str(text)
        )),
        crate::dates::PyIntError::Unsupported => Error::UnsupportedHtml5(
            "BBC iPlayer page numbers beyond 64 bits or with non-ASCII characters are not supported natively"
                .into(),
        ),
    })
}

/// Whether a playlist's pending state is a BBC iPlayer episodes or group
/// listing's.
pub(crate) fn is_iplayer_pending_state(state: &Value) -> bool {
    state["kind"] == EPISODES.key || state["kind"] == GROUP.key
}

/// `OnDemandPagedList(functools.partial(self._fetch_page, pid,
/// self._PAGE_SIZE, series_id), self._PAGE_SIZE)`.
pub(crate) fn fetch_iplayer_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let ie = if state["kind"] == EPISODES.key {
        &EPISODES
    } else {
        &GROUP
    };
    let pid = state["pid"].as_str().unwrap_or_default().to_owned();
    let series_id = state["series_id"].as_str().map(str::to_owned);
    if let Some(page) = state["page"].as_i64() {
        // One page's generator, whose exception ends it with a `None` entry.
        if limit.is_some_and(|limit| entries.len() >= limit) {
            return Ok(Some(state));
        }
        if let Err(raised) = ie.fetch_page(
            client,
            &pid,
            EXPLICIT_PAGE_SIZE,
            series_id.as_deref(),
            page - 1,
            entries,
        ) {
            let text = generator_error_text(raised.in_entries())?;
            let url = Url::parse(state["url"].as_str().unwrap_or_default())?;
            let mut entry = super::direct::build_at(url.clone(), &url, String::new());
            entry.extra.insert(crate::FETCH_ERROR.into(), text.into());
            entry
                .extra
                .insert(crate::FETCH_ERROR_COUNT.into(), entries.len().into());
            entries.push(entry);
        }
        return Ok(None);
    }
    let mut fetch = |page: usize| {
        let mut fetched = Vec::new();
        ie.fetch_page(
            client,
            &pid,
            ie.page_size as i64,
            series_id.as_deref(),
            page as i64,
            &mut fetched,
        )
        .map_err(Raised::in_entries)?;
        Ok(fetched)
    };
    let placeholder =
        || ie.episode_entry("https://www.bbc.co.uk/iplayer/episode/None", "", Map::new());
    super::paged::fetch_requested(
        state,
        limit,
        entries,
        ie.page_size,
        &mut fetch,
        &placeholder,
    )
}

impl Extractor for BbcCoUkIPlayerEpisodes {
    fn key(&self) -> &'static str {
        EPISODES.key
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        EPISODES.extract_playlist(client, url)
    }
}

impl Extractor for BbcCoUkIPlayerGroup {
    fn key(&self) -> &'static str {
        GROUP.key
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        GROUP.extract_playlist(client, url)
    }
}

/// Upstream `BBCCoUkArticleIE`.
pub struct BbcCoUkArticle;

const ARTICLE_NAME: &str = "bbc.co.uk:article";
const ARTICLE_KEY: &str = "BBCCoUkArticle";

static ARTICLE_URL: LazyLock<UrlRegex> = LazyLock::new(|| {
    UrlRegex::new(r"^https?://(?:www\.)?bbc\.co\.uk/programmes/articles/(?P<id>[a-zA-Z0-9]+)")
});
static CLIP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div[^>]+typeof="Clip"[^>]+resource="([^"]+)""#).unwrap());

/// `BBCCoUkArticleIE._match_valid_url`.
pub fn article_suitable(url: &str) -> bool {
    ARTICLE_URL.is_match(url)
}

/// `url_result(url)` (no `ie`).
fn plain_url_entry(target: &str) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": null,
            "entry": {"_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for BbcCoUkArticle {
    fn key(&self) -> &'static str {
        ARTICLE_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ARTICLE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(ARTICLE_NAME);
        crate::screen::screen(&format!(
            "[{ARTICLE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, ARTICLE_NAME, &playlist_id)?;

        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            search_warning(ARTICLE_NAME, "OpenGraph title");
        }
        let Some(description) = crate::html::og_search_property(&webpage, "description") else {
            search_warning(ARTICLE_NAME, "OpenGraph description");
            return Err(Error::Reported(
                "'NoneType' object has no attribute 'strip'".into(),
            ));
        };
        let description = crate::batch::py_strip(&description).to_owned();

        let entries = CLIP
            .captures_iter(&webpage)
            .map(|found| plain_url_entry(&found[1]))
            .collect::<Result<Vec<_>>>()?;

        // `playlist_result` keeps a truthy title and a non-`None` description.
        let title = title.filter(|title| !title.is_empty());
        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        keys.push("description");
        let mut extra = Map::new();
        extra.insert("description".into(), description.into());
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: ARTICLE_NAME.into(),
            extractor_key: ARTICLE_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
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
            "http://www.bbc.co.uk/programmes/b00mfl7n/clips?page=1"
        ));
        assert!(suitable(
            "http://www.bbc.co.uk/programmes/b05rcz9v/broadcasts/2016/06"
        ));
        assert!(!suitable("http://www.bbc.co.uk/programmes/b00mfl7n"));
        assert!(!suitable(
            "http://www.bbc.co.uk/programmes/articles/3jNQLTMrPlYGTBn0WV6M2MS/x"
        ));
        assert!(article_suitable(
            "http://www.bbc.co.uk/programmes/articles/3jNQLTMrPlYGTBn0WV6M2MS/not-your-typical-role-model"
        ));
        assert!(iplayer_episodes_suitable(
            "https://www.bbc.co.uk/iplayer/episodes/b094m5t9/doctor-foster?seriesId=b094m6nv"
        ));
        assert!(!iplayer_episodes_suitable(
            "https://www.bbc.co.uk/iplayer/episode/b094m5t9"
        ));
        assert!(iplayer_group_suitable(
            "http://www.bbc.co.uk/iplayer/group/p02tcc32"
        ));
        assert!(!iplayer_group_suitable(
            "http://www.bbc.co.uk/iplayer/episodes/p02tcc32"
        ));
    }

    #[test]
    fn page_numbers_follow_python_int() {
        assert_eq!(python_int(" +1_0 ").unwrap(), 10);
        assert_eq!(python_int("-2").unwrap(), -2);
        for invalid in ["", "two", "1__0", "_1", "1_", "1.0", "--1"] {
            assert_eq!(
                python_int(invalid).unwrap_err().to_string(),
                format!(
                    "invalid literal for int() with base 10: {}",
                    crate::output::traverse::py_repr_str(invalid)
                )
            );
        }
        assert!(matches!(
            python_int("\u{661}"),
            Err(Error::UnsupportedHtml5(_))
        ));
    }
}
