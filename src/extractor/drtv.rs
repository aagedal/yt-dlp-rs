//! Upstream `DRTVIE`, `DRTVLiveIE`, `DRTVSeasonIE` and `DRTVSeriesIE`
//! (`yt_dlp/extractor/drtv.py`): DR TV programs through an anonymous
//! dr-massive token (`_real_initialize`), the page's `window.__data` item (or
//! the backup items API) and the account videos API (HLS master playlists per
//! access service, API subtitle tracks or the manifests' subtitles, the
//! Denmark-only geo error), live channels through the mu-online channel API
//! (HLS), and season/series pages of the dr-massive page API as `url`
//! results.
//!
//! Not supported natively: API values of other types than upstream's tests
//! show where upstream would stringify or fail with a Python `TypeError`.
use super::{
    Extractor, dict_info, download_json, download_json_note, download_json_note_headers,
    download_webpage, extract_m3u8_master_optional, search_warning, update_url_query,
};
use crate::{
    Error, ExtractResult, PlaylistInfo, Result, SubtitleFormat, Subtitles, VideoFormat, VideoInfo,
};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct Drtv;
pub struct DrtvLive;
pub struct DrtvSeason;
pub struct DrtvSeries;

const NAME: &str = "drtv";
const KEY: &str = "DRTV";
const LIVE_NAME: &str = "drtv:live";
const LIVE_KEY: &str = "DRTVLive";
const SEASON_NAME: &str = "drtv:season";
const SEASON_KEY: &str = "DRTVSeason";
const SERIES_NAME: &str = "drtv:series";
const SERIES_KEY: &str = "DRTVSeries";

const SERIES_API: &str = "https://production-cdn.dr-massive.com/api/page?device=web_browser&item_detail_expand=all&lang=da&max_list_prefetch=3&path=";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www\.)?dr\.dk/tv/se(?:/ondemand)?/(?:[^/?#]+/)*|(?:www\.)?(?:dr\.dk|dr-massive\.com)/drtv/(?:se|episode|program)/)(?P<id>[\da-z_-]+)",
    )
    .unwrap()
});
static LIVE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?dr\.dk/(?:tv|TV)/live/(?P<id>[\da-z-]+)").unwrap()
});
static SEASON_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:dr\.dk|dr-massive\.com)/drtv/saeson/(?P<display_id>[\w-]+)_(?P<id>\d+)",
    )
    .unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:dr\.dk|dr-massive\.com)/drtv/serie/(?P<display_id>[\w-]+)_(?P<id>\d+)",
    )
    .unwrap()
});
static DATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"window\.__data\s*=").unwrap());

/// `DRTVIE._TOKEN` (class-wide, like upstream).
static TOKEN: Mutex<Option<String>> = Mutex::new(None);

/// `DRTVIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `DRTVLiveIE._match_valid_url`.
pub fn live_suitable(url: &str) -> bool {
    LIVE_URL.is_match(url)
}

/// `DRTVSeasonIE._match_valid_url`.
pub fn season_suitable(url: &str) -> bool {
    SEASON_URL.is_match(url)
}

/// `DRTVSeriesIE._match_valid_url`.
pub fn series_suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("DR TV {what} are not supported natively"))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// `traverse_obj(value, (k1, k2, ...))` of object keys, `None` and `{}`
/// results dropped.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .try_fold(value, |value, key| value.get(*key))
        .filter(|found| !found.is_null() && *found != &Value::Object(Map::new()))
}

/// `int_or_none`: Python's `int()` of numbers, booleans and integer text.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().replace('_', "").parse::<i64>().ok(),
        _ => None,
    }
}

/// A string-or-`None` value.
fn text(value: Option<&Value>, what: &str) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(unsupported(&format!("{what} that are not strings"))),
    }
}

/// The values `...` iterates: a list's items or a dict's values.
fn children(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map.values().collect(),
        _ => Vec::new(),
    }
}

/// `traverse_obj(token_response, (lambda _, x: x['type'] == 'UserAccount',
/// 'value', {str}), get_all=False)`.
fn anonymous_token(response: &Value) -> Option<String> {
    children(response)
        .into_iter()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("UserAccount"))
        .find_map(|entry| entry.get("value").and_then(Value::as_str))
        .map(str::to_owned)
}

/// A random `uuid.uuid4()`.
fn uuid4() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for (index, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_usize(index);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos()),
        );
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// `traverse_obj(json_data, ('cache', 'page', ..., (None, ('entries', 0)),
/// 'item', {dict}), get_all=False)`.
fn page_item(data: &Value) -> Option<&Value> {
    let page = data.get("cache")?.get("page")?;
    children(page).into_iter().find_map(|value| {
        let entry = value
            .get("entries")
            .and_then(Value::as_array)
            .and_then(|entries| entries.first());
        [value.get("item"), entry.and_then(|entry| entry.get("item"))]
            .into_iter()
            .flatten()
            .find(|item| item.as_object().is_some_and(|map| !map.is_empty()))
    })
}

/// `SUBTITLE_LANGS.get(lang, lang)`.
fn subtitle_language(language: &str) -> &str {
    match language {
        "DanishLanguageSubtitles" => "da",
        "ForeignLanguageSubtitles" => "da_foreign",
        "CombinedLanguageSubtitles" => "da_combined",
        other => other,
    }
}

fn add_subtitle(subtitles: &mut Subtitles, language: String, track: SubtitleFormat) {
    match subtitles
        .entries
        .iter_mut()
        .find(|(key, _)| key.as_deref() == Some(language.as_str()))
    {
        Some((_, tracks)) => tracks.push(track),
        None => subtitles.entries.push((Some(language), vec![track])),
    }
}

/// The streams' formats and subtitles (`_real_extract`'s loop).
fn stream_formats(
    client: &Client,
    stream_data: &Value,
    video_id: &str,
) -> Result<(Vec<VideoFormat>, Subtitles)> {
    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    // `traverse_obj(stream_data, (lambda _, x: x['url']))`.
    for stream in children(stream_data) {
        if !stream.get("url").is_some_and(truthy) || !stream.is_object() {
            continue;
        }
        let mut format_id = match stream.get("format") {
            None => "na".to_owned(),
            Some(Value::String(format)) => format.clone(),
            Some(_) => return Err(unsupported("stream formats that are not strings")),
        };
        let access_service = stream.get("accessService").and_then(Value::as_str);
        let mut preference: Option<i64> = None;
        let mut subtitle_suffix = String::new();
        match access_service {
            Some(service @ ("SpokenSubtitles" | "SignLanguage" | "VisuallyInterpreted")) => {
                preference = Some(-1);
                format_id.push_str(&format!("-{service}"));
                subtitle_suffix = format!("-{service}");
            }
            Some("StandardVideo") => preference = Some(1),
            _ => {}
        }
        let Some(Value::String(stream_url)) = stream.get("url") else {
            return Err(unsupported("stream URLs that are not strings"));
        };
        let manifest =
            Url::parse(stream_url).map_err(|_| unsupported("stream URLs that are not absolute"))?;
        let (fmts, subs) = match extract_m3u8_master_optional(
            client,
            &manifest,
            NAME,
            video_id,
            Some(&format_id),
            true,
            false,
        ) {
            Some(stream) => (stream.formats, stream.subtitles),
            None => (Vec::new(), Subtitles::default()),
        };
        formats.extend(fmts.into_iter().map(|mut format| {
            if let Some(preference) = preference {
                format
                    .extra
                    .insert("preference".into(), Value::from(preference));
            }
            format
        }));

        // `traverse_obj(stream, ('subtitles', lambda _, v:
        // url_or_none(v['link']), {dict}))`.
        let api_subtitles: Vec<&Value> = stream
            .get("subtitles")
            .map(children)
            .unwrap_or_default()
            .into_iter()
            .filter(|track| {
                track.is_object()
                    && track
                        .get("link")
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                        .is_some()
            })
            .collect();
        if api_subtitles.is_empty() {
            super::svt::merge_subtitles(&mut subtitles, subs);
        }
        for track in api_subtitles {
            // `sub_track.get('language') or 'da'`.
            let language = match track.get("language") {
                Some(value) if !truthy(value) => "da".to_owned(),
                None => "da".to_owned(),
                Some(Value::String(language)) => language.clone(),
                Some(_) => return Err(unsupported("subtitle languages that are not strings")),
            };
            let link = track["link"].as_str().unwrap_or_default();
            let url = Url::parse(link)
                .map_err(|_| unsupported("subtitle links that are not absolute"))?;
            // `mimetype2ext(sub_track.get('format')) or 'vtt'`.
            let ext = track
                .get("format")
                .and_then(Value::as_str)
                .map(|mime| super::direct::mimetype2ext_or(mime, None))
                .filter(|ext| !ext.is_empty())
                .unwrap_or_else(|| "vtt".to_owned());
            add_subtitle(
                &mut subtitles,
                format!("{}{subtitle_suffix}", subtitle_language(&language)),
                SubtitleFormat {
                    name: None,
                    url,
                    ext,
                    protocol: None,
                    fragments: Vec::new(),
                    extra: Map::new(),
                },
            );
        }
    }
    Ok((formats, subtitles))
}

/// `traverse_obj(item, {...})`'s metadata fields in upstream order.
fn item_fields(item: &Value) -> Result<Vec<(&'static str, Value)>> {
    let mut fields = Vec::new();
    let mut raw = |key: &'static str, value: Option<&Value>| {
        if let Some(value) = value.filter(|value| !value.is_null()) {
            fields.push((key, value.clone()));
        }
    };
    let title = text(path(item, &["title"]), "titles")?;
    raw("title", title.map(Value::from).as_ref());
    raw("alt_title", path(item, &["contextualTitle"]));
    raw("description", path(item, &["description"]));
    raw("thumbnail", path(item, &["images", "wallpaper"]));
    let release = path(item, &["customFields", "BroadcastTimeDK"])
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map(Value::from);
    raw("release_timestamp", release.as_ref());
    let int = |keys: &[&str]| int_or_none(path(item, keys)).map(Value::from);
    raw("duration", int(&["duration"]).as_ref());
    raw("series", path(item, &["season", "show", "title"]));
    raw("season", path(item, &["season", "title"]));
    raw("season_number", int(&["season", "seasonNumber"]).as_ref());
    raw("season_id", path(item, &["seasonId"]));
    raw("episode", path(item, &["episodeName"]));
    raw("episode_number", int(&["episodeNumber"]).as_ref());
    raw("release_year", int(&["releaseYear"]).as_ref());
    Ok(fields)
}

impl Extractor for Drtv {
    fn key(&self) -> &'static str {
        KEY
    }

    /// `_GEO_BYPASS = False` and `_GEO_COUNTRIES = ['DK']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&["DK"])
    }

    /// `_real_initialize`: the anonymous token (once per run).
    fn real_initialize(&self, client: &Client, session: &super::login::Session) -> Result<()> {
        if TOKEN.lock().is_ok_and(|token| token.is_some()) {
            return Ok(());
        }
        let url = Url::parse(
            "https://isl.dr-massive.com/api/authorization/anonymous-sso?device=phone_android&lang=da&supportFallbackToken=true",
        )?;
        // `json.dumps(...)` with its default separators.
        let body = format!(
            "{{\"deviceId\": \"{}\", \"scopes\": [\"Catalog\"], \"optout\": true}}",
            uuid4()
        );
        let page = session.request(
            client,
            &url,
            (
                Some("Downloading anonymous token"),
                "Unable to download JSON metadata",
            ),
            &[("content-type", "application/json")],
            Some(body),
            &[],
        )?;
        let response = crate::json::from_str(&page.text).map_err(|error| {
            session.error(&format!("Failed to parse JSON  (caused by {error})"))
        })?;
        let Some(token) = anonymous_token(&response) else {
            return Err(session.bug("Unable to get anonymous token"));
        };
        if let Ok(mut slot) = TOKEN.lock() {
            *slot = Some(token);
        }
        Ok(())
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let url_slug = found["id"].to_owned();
        self.initialize(client, NAME, &url_slug)?;
        announce(NAME, url);
        let webpage = download_webpage(client, url, NAME, &url_slug)?;

        // `_search_json(r'window\.__data\s*=', ..., fatal=False) or {}`.
        let json_data = match super::search_json_detailed(&DATA, &webpage, &str::to_owned) {
            Some(Ok(data)) => data,
            Some(Err(error)) => {
                // `ExtractorError(f'{id}: Failed to parse JSON', cause=e)`.
                let cause = error.map_or_else(String::new, |error| {
                    format!(
                        " (caused by JSONDecodeError({}))",
                        crate::batch::py_repr(&error)
                    )
                });
                crate::screen::warning(&format!(
                    "[{NAME}] {url_slug}: Unable to extract data - Failed to parse JSON: {url_slug}: Failed to parse JSON{cause}; {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
                Value::Null
            }
            None => {
                search_warning(NAME, "data");
                Value::Null
            }
        };
        let (item, item_id) = match page_item(&json_data) {
            Some(item) => {
                let Some(Value::String(item_id)) = item.get("id") else {
                    return Err(unsupported("page items without a string ID"));
                };
                (item.clone(), item_id.clone())
            }
            None => {
                let item_id = url_slug
                    .rsplit_once('_')
                    .map_or(url_slug.as_str(), |(_, id)| id)
                    .to_owned();
                let api = update_url_query(
                    &Url::parse(&format!(
                        "https://production-cdn.dr-massive.com/api/items/{item_id}"
                    ))?,
                    &[
                        ("device", "web_browser"),
                        ("expand", "all"),
                        ("ff", "idp,ldp,rpt"),
                        ("geoLocation", "dk"),
                        ("isDeviceAbroad", "false"),
                        ("lang", "da"),
                        ("segments", "drtv,optedout"),
                        ("sub", "Anonymous"),
                    ],
                );
                let item = download_json_note(
                    client,
                    &api,
                    NAME,
                    &item_id,
                    "Attempting to download backup item data",
                )?;
                (item, item_id)
            }
        };

        // `try_call(lambda: item['customId'].rsplit(':', 1)[-1]) or item_id`.
        let video_id = item
            .get("customId")
            .and_then(Value::as_str)
            .map(|custom| custom.rsplit_once(':').map_or(custom, |(_, id)| id))
            .filter(|id| !id.is_empty())
            .unwrap_or(&item_id)
            .to_owned();
        let token = TOKEN
            .lock()
            .ok()
            .and_then(|token| token.clone())
            .unwrap_or_else(|| "None".into());
        let api = update_url_query(
            &Url::parse(&format!(
                "https://production.dr-massive.com/api/account/items/{item_id}/videos"
            ))?,
            &[
                ("delivery", "stream"),
                ("device", "web_browser"),
                ("ff", "idp,ldp,rpt"),
                ("lang", "da"),
                ("resolution", "HD-1080"),
                ("sub", "Anonymous"),
            ],
        );
        let authorization = format!("Bearer {token}");
        let stream_data = download_json_note_headers(
            client,
            &api,
            NAME,
            &video_id,
            "Downloading stream data",
            &[("authorization", &authorization)],
        )?;
        let (formats, subtitles) = stream_formats(client, &stream_data, &video_id)?;

        if formats.is_empty()
            && path(&item, &["season", "customFields", "IsGeoRestricted"]).is_some_and(truthy)
        {
            return Err(crate::geo::restricted(
                &format!(
                    "[{NAME}] {url_slug}: This video is not available from your location due to geo restriction"
                ),
                &["DK"],
            ));
        }

        let mut fields = vec![
            ("id", Value::from(video_id)),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        fields.extend(item_fields(&item)?);
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

impl Extractor for DrtvLive {
    fn key(&self) -> &'static str {
        LIVE_KEY
    }

    /// `_GEO_COUNTRIES = ['DK']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["DK"])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = LIVE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let channel_id = found["id"].to_owned();
        announce(LIVE_NAME, url);
        let api = Url::parse(&format!(
            "https://www.dr.dk/mu-online/api/1.0/channel/{channel_id}"
        ))?;
        let channel_data = download_json(client, &api, LIVE_NAME, &channel_id)?;
        if !channel_data.is_object() {
            return Err(unsupported("channel responses that are not objects"));
        }
        let title = match channel_data.get("Title") {
            None => return Err(key_error(&channel_id, "Title")),
            Some(Value::String(title)) => title.clone(),
            Some(_) => return Err(unsupported("channel titles that are not strings")),
        };

        let mut formats = Vec::new();
        let servers = match channel_data.get("StreamingServers") {
            None => &Vec::new(),
            Some(Value::Array(servers)) => servers,
            Some(_) => return Err(unsupported("streaming servers that are not lists")),
        };
        for streaming_server in servers {
            if !streaming_server.is_object() {
                return Err(unsupported("streaming servers that are not objects"));
            }
            let server = match streaming_server.get("Server") {
                Some(server) if !truthy(server) => continue,
                None => continue,
                Some(Value::String(server)) => server.clone(),
                Some(_) => return Err(unsupported("streaming server URLs that are not strings")),
            };
            let link_type = streaming_server.get("LinkType").and_then(Value::as_str);
            let qualities = match streaming_server.get("Qualities") {
                None => &Vec::new(),
                Some(Value::Array(qualities)) => qualities,
                Some(_) => return Err(unsupported("stream qualities that are not lists")),
            };
            for quality in qualities {
                let streams = match quality.get("Streams") {
                    None if quality.is_object() => &Vec::new(),
                    Some(Value::Array(streams)) => streams,
                    _ => return Err(unsupported("quality streams that are not lists")),
                };
                for stream in streams {
                    if !stream.is_object() {
                        return Err(unsupported("streams that are not objects"));
                    }
                    let stream_path = match stream.get("Stream") {
                        Some(stream_path) if !truthy(stream_path) => continue,
                        None => continue,
                        Some(Value::String(stream_path)) => stream_path.clone(),
                        Some(_) => return Err(unsupported("stream paths that are not strings")),
                    };
                    match link_type {
                        Some("HLS") => {
                            let base = Url::parse(&format!("{server}/{stream_path}"))
                                .map_err(|_| unsupported("stream URLs that are not absolute"))?;
                            let stream_url = update_url_query(&base, &[("b", "")]);
                            if let Some(stream) = extract_m3u8_master_optional(
                                client,
                                &stream_url,
                                LIVE_NAME,
                                &channel_id,
                                Some("HLS"),
                                false,
                                true,
                            ) {
                                formats.extend(stream.formats);
                            }
                        }
                        Some("HDS") => {
                            let base = Url::parse(&format!("{server}/{stream_path}"))
                                .map_err(|_| unsupported("stream URLs that are not absolute"))?;
                            let manifest = update_url_query(&base, &[("hdcore", "3.7.0")]);
                            formats.extend(super::extract_f4m_optional(
                                client,
                                LIVE_NAME,
                                &manifest,
                                &channel_id,
                                Some("HDS"),
                            )?);
                        }
                        _ => {}
                    }
                }
            }
        }

        let fields = vec![
            ("id", Value::from(channel_id)),
            ("title", Value::from(title)),
            (
                "thumbnail",
                channel_data
                    .get("PrimaryImageUri")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            ("formats", Value::Null),
            ("is_live", Value::Bool(true)),
        ];
        Ok(Some(dict_info(url, (LIVE_NAME, LIVE_KEY), fields, formats)))
    }
}

/// A raw `{'_type': 'url', 'url': ..., 'ie_key': ..., ...}` playlist entry.
fn url_entry(
    target: &str,
    (ie_key, extractor): (&'static str, &'static str),
    temp_id: Option<&str>,
    fields: Vec<(&str, Value)>,
) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // `get_temp_id(url)`: `None` (empty) when the URL does not match.
    info.id = temp_id.unwrap_or_default().to_owned();
    let mut entry = Map::new();
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    entry.insert("ie_key".into(), ie_key.into());
    for (key, value) in fields {
        entry.insert(key.into(), value);
    }
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": extractor,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `f'https://www.dr.dk/drtv{path}'` (a missing path is `None`).
fn drtv_url(path: Option<&Value>, temp_id: &str, required: bool) -> Result<String> {
    match path {
        None if required => Err(key_error(temp_id, "path")),
        None | Some(Value::Null) => Ok("https://www.dr.dk/drtvNone".into()),
        Some(Value::String(path)) => Ok(format!("https://www.dr.dk/drtv{path}")),
        Some(_) => Err(unsupported("paths that are not strings")),
    }
}

/// `traverse_obj(data, ('entries', 0, 'item', KEY...))`.
fn page_value<'a>(data: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let item = data
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| entries.first())?;
    path(item, &[&["item"], keys].concat())
}

/// The season/series playlist dict: `id`, `display_id`, `title`,
/// `alt_title`, `series`, `entries` (and the season's `season_number`).
fn playlist(
    page: &Url,
    (name, key): (&'static str, &'static str),
    id: String,
    display_id: String,
    data: &Value,
    entries: Vec<VideoInfo>,
    season_number: Option<Value>,
) -> Result<ExtractResult> {
    let title = text(page_value(data, &["title"]), "titles")?;
    let mut extra = Map::new();
    let mut keys = vec!["_type", "id", "display_id"];
    extra.insert("display_id".into(), display_id.into());
    if title.is_some() {
        keys.push("title");
    }
    if let Some(alt_title) = page_value(data, &["contextualTitle"]) {
        keys.push("alt_title");
        extra.insert("alt_title".into(), alt_title.clone());
    }
    if let Some(series) = &title {
        keys.push("series");
        extra.insert("series".into(), series.clone().into());
    }
    keys.push("entries");
    if let Some(season_number) = season_number {
        keys.push("season_number");
        extra.insert("season_number".into(), season_number);
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    Ok(ExtractResult::Playlist(PlaylistInfo {
        id,
        title: title.unwrap_or_default(),
        webpage_url: page.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    }))
}

/// `traverse_obj(data, ('entries', 0, 'item', *KEYS))` iterated: a list.
fn page_list<'a>(data: &'a Value, keys: &[&str]) -> Result<&'a Vec<Value>> {
    match page_value(data, keys) {
        Some(Value::Array(items)) => Ok(items),
        _ => Err(unsupported("pages without an item list")),
    }
}

fn opt(value: Option<&Value>) -> Value {
    value.cloned().unwrap_or(Value::Null)
}

impl Extractor for DrtvSeason {
    fn key(&self) -> &'static str {
        SEASON_KEY
    }

    /// `_GEO_COUNTRIES = ['DK']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["DK"])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SEASON_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (display_id, season_id) = (found["display_id"].to_owned(), found["id"].to_owned());
        announce(SEASON_NAME, url);
        let api = Url::parse(&format!("{SERIES_API}/saeson/{display_id}_{season_id}"))?;
        let data = download_json(client, &api, SEASON_NAME, &display_id)?;
        let series = opt(page_value(&data, &["title"]));
        let season_number = page_value(&data, &["seasonNumber"]).cloned();
        let mut entries = Vec::new();
        for episode in page_list(&data, &["episodes", "items"])? {
            if !episode.is_object() {
                return Err(unsupported("season episodes that are not objects"));
            }
            let target = drtv_url(episode.get("path"), &season_id, true)?;
            let temp_id = VALID_URL
                .captures(&target)
                .map(|found| found["id"].to_owned());
            entries.push(url_entry(
                &target,
                (KEY, NAME),
                temp_id.as_deref(),
                vec![
                    ("title", opt(episode.get("title"))),
                    ("alt_title", opt(episode.get("contextualTitle"))),
                    ("episode", opt(episode.get("episodeName"))),
                    ("description", opt(episode.get("shortDescription"))),
                    ("series", series.clone()),
                    (
                        "season_number",
                        season_number.clone().unwrap_or(Value::Null),
                    ),
                    ("episode_number", opt(episode.get("episodeNumber"))),
                ],
            )?);
        }
        Ok(Some(playlist(
            url,
            (SEASON_NAME, SEASON_KEY),
            season_id,
            display_id,
            &data,
            entries,
            season_number,
        )?))
    }
}

impl Extractor for DrtvSeries {
    fn key(&self) -> &'static str {
        SERIES_KEY
    }

    /// `_GEO_COUNTRIES = ['DK']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["DK"])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (display_id, series_id) = (found["display_id"].to_owned(), found["id"].to_owned());
        announce(SERIES_NAME, url);
        let api = Url::parse(&format!("{SERIES_API}/serie/{display_id}_{series_id}"))?;
        let data = download_json(client, &api, SERIES_NAME, &display_id)?;
        let series = opt(page_value(&data, &["title"]));
        let season_number = opt(page_value(&data, &["seasonNumber"]));
        let mut entries = Vec::new();
        for season in page_list(&data, &["show", "seasons", "items"])? {
            if !season.is_object() {
                return Err(unsupported("series seasons that are not objects"));
            }
            let target = drtv_url(season.get("path"), &series_id, false)?;
            let temp_id = SEASON_URL
                .captures(&target)
                .map(|found| found["id"].to_owned());
            entries.push(url_entry(
                &target,
                (SEASON_KEY, SEASON_NAME),
                temp_id.as_deref(),
                vec![
                    ("title", opt(season.get("title"))),
                    ("alt_title", opt(season.get("contextualTitle"))),
                    ("series", series.clone()),
                    ("season_number", season_number.clone()),
                ],
            )?);
        }
        Ok(Some(playlist(
            url,
            (SERIES_NAME, SERIES_KEY),
            series_id,
            display_id,
            &data,
            entries,
            None,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for (url, id) in [
            (
                "https://www.dr.dk/tv/se/boern/ultra/klassen-ultra/klassen-darlig-taber-10",
                "klassen-darlig-taber-10",
            ),
            (
                "https://www.dr.dk/tv/se/historien-om-danmark/-/historien-om-danmark-stenalder",
                "historien-om-danmark-stenalder",
            ),
            (
                "https://www.dr.dk/drtv/se/frank-and-kastaniegaarden_71769",
                "frank-and-kastaniegaarden_71769",
            ),
            (
                "https://www.dr.dk/drtv/episode/bonderoeven_71769",
                "bonderoeven_71769",
            ),
            (
                "https://dr-massive.com/drtv/se/bonderoeven_71769",
                "bonderoeven_71769",
            ),
            (
                "https://www.dr.dk/drtv/program/jagten_220924",
                "jagten_220924",
            ),
        ] {
            assert_eq!(&VALID_URL.captures(url).unwrap()["id"], id, "{url}");
            assert!(!live_suitable(url) && !season_suitable(url) && !series_suitable(url));
        }
        assert!(live_suitable("https://www.dr.dk/tv/live/dr1"));
        assert!(live_suitable("https://www.dr.dk/TV/live/dr2"));
        let season = SEASON_URL
            .captures("https://www.dr.dk/drtv/saeson/frank-and-kastaniegaarden_9008")
            .unwrap();
        assert_eq!(
            (&season["display_id"], &season["id"]),
            ("frank-and-kastaniegaarden", "9008")
        );
        assert!(series_suitable(
            "https://www.dr.dk/drtv/serie/frank-and-kastaniegaarden_6954"
        ));
        assert!(!suitable(
            "https://www.dr.dk/drtv/serie/frank-and-kastaniegaarden_6954"
        ));
    }

    #[test]
    fn anonymous_token_is_the_first_user_account_value() {
        let response = json!([
            {"type": "Other", "value": "a"},
            {"value": "b"},
            {"type": "UserAccount", "value": 1},
            {"type": "UserAccount", "value": "c"},
        ]);
        assert_eq!(anonymous_token(&response).as_deref(), Some("c"));
        assert_eq!(anonymous_token(&json!({"type": "UserAccount"})), None);
        assert_eq!(anonymous_token(&json!([])), None);
    }

    #[test]
    fn page_items_prefer_each_pages_item_then_its_first_entry() {
        let data = json!({"cache": {"page": {
            "/a": {"item": {}},
            "/b": {"entries": [{"item": {"id": "2"}}]},
            "/c": {"item": {"id": "3"}},
        }}});
        assert_eq!(page_item(&data).unwrap()["id"], "2");
        assert!(page_item(&json!({"cache": {"page": []}})).is_none());
    }

    #[test]
    fn uuids_have_the_v4_shape() {
        let id = uuid4();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
    }
}
