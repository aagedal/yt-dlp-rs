//! Upstream `yt_dlp/extractor/nrk.py`: `NRKIE` (the psapi playback
//! manifest's HLS and MP3 assets, WebVTT subtitles and playback metadata,
//! with the programs API's series fields), the `nrk:ID` delegating
//! `NRKTVIE`, `NRKTVDirekteIE`, `NRKRadioPodkastIE` and `NRKTVEpisodeIE`,
//! and the `NRKTVSeasonIE`, `NRKTVSeriesIE` and `NRKPlaylistIE` playlists.
//! `NRKSkoleIE` and `NRKTVEpisodesIE` (legacy endpoints that no longer
//! answer) are not ported.
use super::clyp::key_error;
use super::{
    Extractor, dict_format, dict_info, download_json_note_headers, download_webpage,
    download_webpage_handle, extract_m3u8_master_optional, search_json,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Nrk;
pub struct NrkTv;
pub struct NrkTvDirekte;
pub struct NrkRadioPodkast;
pub struct NrkTvEpisode;
pub struct NrkTvSeason;
pub struct NrkTvSeries;
pub struct NrkPlaylist;

const NAME: &str = "NRK";
const TV_NAME: &str = "NRKTV";
const DIREKTE_NAME: &str = "NRKTVDirekte";
const PODKAST_NAME: &str = "NRKRadioPodkast";
const EPISODE_NAME: &str = "NRKTVEpisode";
const SEASON_NAME: &str = "NRKTVSeason";
const SERIES_NAME: &str = "NRKTVSeries";
const PLAYLIST_NAME: &str = "NRKPlaylist";

/// `_call_api`'s `Accept` header (needed for working stream URLs).
const ACCEPT: &str =
    "application/vnd.nrk.psapi+json; version=9; player=tv-player; device=player-core";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?:nrk:|https?://(?:(?:www\.)?nrk\.no/video/(?:PS\*|[^_]+_)|v8[-.]psapi\.nrk\.no/mediaelement/))(?P<id>[^?#&]+)",
    )
    .unwrap()
});
static TV_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:tv|radio)\.nrk(?:super)?\.no/(?:[^/]+/)*(?P<id>[a-zA-Z]{4}\d{8})")
        .unwrap()
});
static EPISODE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<id>[a-zA-Z]{4}\d{8})$").unwrap());
static DIREKTE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:tv|radio)\.nrk\.no/direkte/(?P<id>[^/?#&]+)").unwrap()
});
static PODKAST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://radio\.nrk\.no/pod[ck]ast/(?:[^/]+/)+(?P<id>l_[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12})",
    )
    .unwrap()
});
static EPISODE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://tv\.nrk\.no/serie/(?P<id>[^/?#]+/sesong/(?P<season_number>\d+)/episode/(?P<episode_number>\d+))",
    )
    .unwrap()
});
static SEASON_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?P<domain>tv|radio)\.nrk\.no/(?P<serie_kind>serie|pod[ck]ast)/(?P<serie>[^/]+)/(?:(?:sesong/)?(?P<id>\d+)|sesong/(?P<id_2>[^/?#&]+))",
    )
    .unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?P<domain>(?:tv|radio)\.nrk|(?:tv\.)?nrksuper)\.no/(?P<serie_kind>serie|pod[ck]ast)/(?P<id>[^/]+)",
    )
    .unwrap()
});
// `(?!video|skole)` is checked separately.
static PLAYLIST_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?nrk\.no/(?P<rest>.*)").unwrap());
static PLAYLIST_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[^/]+/)+(?P<id>[^/]+)").unwrap());
static PLAYLIST_ITEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"class="[^"]*\brich\b[^"]*"[^>]+data-video-id="([^"]+)""#).unwrap()
});
static PAGE_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script\b[^>]+\bid="pageData"[^>]*>"#).unwrap());
static AKAMAI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://[^/]+\.akamaihd\.net/i/").unwrap());
static BANDWIDTH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:bw_(?:low|high)=\d+|no_audio_only)&?").unwrap());
static CDN_REPL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"://(?:nrkod\d{1,2}-httpcache0-47115-cacheod0\.dna\.ip-only\.net/47115-cacheod0|nrk-od-no\.telenorcdn\.net|minicdn-od\.nrk\.no/od/nrkhd-osl-rr\.netwerk\.no/no)/",
    )
    .unwrap()
});
static SEASON_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Sesong\s+(\d+)").unwrap());
static EPISODE_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)\.").unwrap());
static DESCRIPTION_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\((\d+)\s*:\s*\d+\)").unwrap());

/// NRKIE `_match_id` (the `nrk:` pseudo-URLs included).
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// NRKTVIE `_match_id`.
pub fn tv_suitable(url: &str) -> bool {
    TV_URL.is_match(url)
}

/// NRKTVDirekteIE `_match_id`.
pub fn direkte_suitable(url: &str) -> bool {
    DIREKTE_URL.is_match(url)
}

/// NRKRadioPodkastIE `_match_id`.
pub fn podkast_suitable(url: &str) -> bool {
    PODKAST_URL.is_match(url)
}

/// NRKTVEpisodeIE `_match_valid_url`.
pub fn episode_suitable(url: &str) -> bool {
    EPISODE_URL.is_match(url)
}

/// NRKTVSeasonIE `suitable`: not an episode, program or podcast episode.
pub fn season_suitable(url: &str) -> bool {
    !(tv_suitable(url) || episode_suitable(url) || podkast_suitable(url))
        && SEASON_URL.is_match(url)
}

/// NRKTVSeriesIE `suitable`: not a season, episode, program or podcast
/// episode.
pub fn series_suitable(url: &str) -> bool {
    !(tv_suitable(url) || episode_suitable(url) || podkast_suitable(url) || season_suitable(url))
        && SERIES_URL.is_match(url)
}

/// NRKPlaylistIE `_match_id`.
fn playlist_id(url: &str) -> Option<String> {
    let rest = PLAYLIST_URL.captures(url)?.name("rest")?.as_str();
    if rest.starts_with("video") || rest.starts_with("skole") {
        return None;
    }
    PLAYLIST_PATH
        .captures(rest)
        .map(|found| found["id"].to_owned())
}

/// NRKPlaylistIE `_match_id`.
pub fn playlist_suitable(url: &str) -> bool {
    playlist_id(url).is_some()
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("NRK {what} are not supported natively"))
}

fn start(extractor: &'static str, url: &Url) {
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// The HTTP status of a failed request (`e.cause.status`).
fn http_status(error: &Error) -> Option<u16> {
    match error {
        Error::ExtractorRequest { source, .. } => http_status(source),
        Error::Http(source) | Error::HttpRetryAfter { source, .. } => {
            source.status().map(|status| status.as_u16())
        }
        _ => None,
    }
}

/// `urljoin('https://psapi.nrk.no/', path)` with the `query`.
fn api_url(path: &str, query: &[(&str, &str)]) -> Result<Url> {
    let url = Url::parse("https://psapi.nrk.no/")?.join(path)?;
    Ok(if query.is_empty() {
        url
    } else {
        super::update_url_query(&url, query)
    })
}

/// `_call_api(path, video_id, item, note, query=query)` (fatal).
fn call_api(
    client: &Client,
    extractor: &'static str,
    path: &str,
    id: &str,
    note: &str,
    query: &[(&str, &str)],
) -> Result<Value> {
    let url = api_url(path, query)?;
    download_json_note_headers(client, &url, extractor, id, note, &[("Accept", ACCEPT)])
}

/// `_call_api(..., fatal=False)`: `None` after upstream's warning.
fn call_api_optional(
    client: &Client,
    extractor: &'static str,
    path: &str,
    id: &str,
    note: &str,
) -> Result<Option<Value>> {
    match call_api(client, extractor, path, id, note, &[]) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{extractor}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(Error::Reported(message)) if message.contains("Failed to parse JSON") => Err(
            unsupported("optional API responses that are not JSON (upstream warns)"),
        ),
        Err(error) => Err(error),
    }
}

/// Python `str(value)` of a JSON scalar (`str_or_none` of a non-`None`).
fn py_str(value: &Value) -> Result<String> {
    Ok(match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(number) if number.is_f64() => {
            let float = number.as_f64().unwrap_or_default();
            if float.fract() == 0.0 && float.abs() < 1e16 {
                format!("{float:.1}")
            } else {
                float.to_string()
            }
        }
        Value::Number(number) => number.to_string(),
        _ => {
            return Err(unsupported(
                "list or object values where strings are expected",
            ));
        }
    })
}

/// `str_or_none(value)`.
fn str_or_none(value: Option<&Value>) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => py_str(value).map(Some),
    }
}

/// `int_or_none(value)`: Python `int()` of a number or numeric string.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float.trunc() as i64)),
        Value::String(text) => {
            let text = text.trim();
            let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            text.parse().ok()
        }
        _ => None,
    }
}

/// `url_or_none(value)`.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// A clock-seeded `random.randint(0, 99)`.
fn random_cdn_number() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos()),
    );
    hasher.finish() % 100
}

/// `NRKBaseIE._extract_nrk_formats(asset_url, video_id)`.
fn nrk_formats(
    client: &Client,
    asset_url: &str,
    video_id: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let asset = Url::parse(asset_url)?;
    // `update_url_query(asset_url, {'adap': [], 's': 0})`: `adap` dropped.
    let mut stripped = asset.clone();
    let kept: Vec<(String, String)> = asset
        .query_pairs()
        .filter(|(key, _)| key != "adap")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&kept)
        .finish();
    stripped.set_query((!query.is_empty()).then_some(query.as_str()));
    let asset_url = super::update_url_query(&stripped, &[("s", "0")]).to_string();
    if AKAMAI.is_match(&asset_url) {
        return Err(unsupported("Akamai HD (akamaihd.net/i/) assets"));
    }
    let asset_url = BANDWIDTH.replace_all(&asset_url, "").into_owned();
    let manifest = Url::parse(&asset_url)?;
    let mut formats =
        extract_m3u8_master_optional(client, &manifest, NAME, video_id, None, false, false)
            .map(|stream| stream.formats)
            .unwrap_or_default();
    if formats.is_empty() && CDN_REPL.is_match(&asset_url) {
        let replacement = format!("://nrk-od-{:02}.akamaized.net/no/", random_cdn_number());
        let retry = CDN_REPL
            .replace_all(&asset_url, replacement.as_str())
            .into_owned();
        let retry = Url::parse(&retry)?;
        formats = extract_m3u8_master_optional(client, &retry, NAME, video_id, None, false, false)
            .map(|stream| stream.formats)
            .unwrap_or_default();
    }
    Ok(formats)
}

/// `NRKBaseIE._raise_error(data)`.
fn raise_error(data: &Value, temp_id: &str) -> Error {
    let Some(data) = data.as_object() else {
        return unsupported("nonPlayable values that are not objects");
    };
    let message_type = match data.get("messageType") {
        None => "",
        Some(Value::String(text)) => text.as_str(),
        Some(_) => return unsupported("messageType values that are not strings"),
    };
    let geo_blocked = message_type.contains("IsGeoBlocked")
        || data
            .get("usageRights")
            .and_then(|rights| rights.get("isGeoBlocked"))
            == Some(&Value::Bool(true));
    if geo_blocked {
        // `raise_geo_restricted(msg, countries=['NO'])`.
        return crate::geo::restricted(
            &format!(
                "[{NAME}] {temp_id}: NRK har ikke rettigheter til å vise dette programmet utenfor Norge"
            ),
            &["NO"],
        );
    }
    let known = match message_type {
        "ProgramRightsAreNotReady" => Some("Du kan dessverre ikke se eller høre programmet"),
        "ProgramRightsHasExpired" => Some("Programmet har gått ut"),
        "NoProgramRights" => Some("Ikke tilgjengelig"),
        "ProgramIsGeoBlocked" => {
            Some("NRK har ikke rettigheter til å vise dette programmet utenfor Norge")
        }
        _ => None,
    };
    let message = match data.get("endUserMessage") {
        Some(value) if super::lnk::truthy(Some(value)) => match py_str(value) {
            Ok(text) => text,
            Err(error) => return error,
        },
        _ => known.unwrap_or(message_type).to_owned(),
    };
    Error::Reported(format!("[{NAME}] {temp_id}: {NAME} said: {message}"))
}

/// `traverse` of nested object keys (`try_get(x, lambda x: x[a][b]...)`).
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().try_fold(value, |value, key| value.get(*key))
}

/// A `url_result(target, ie, video_id, video_title)` playlist entry.
pub(crate) fn url_entry(
    target: &str,
    ie_key: &'static str,
    id: Option<&str>,
    temp_id: Option<String>,
    title: Option<String>,
) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // `get_temp_id(url)` without an id.
    if let Some(id) = id.map(str::to_owned).or(temp_id) {
        info.id = id;
    }
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ie_key.into());
    if let Some(id) = id {
        entry.insert("id".into(), id.into());
    }
    if let Some(title) = title {
        entry.insert("title".into(), title.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": ie_key,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `url_result(f'nrk:{nrk_id}', ie=NRKIE.ie_key(), video_id=nrk_id)`.
fn nrk_entry(nrk_id: &str) -> Result<VideoInfo> {
    url_entry(&format!("nrk:{nrk_id}"), NAME, Some(nrk_id), None, None)
}

/// A playlist result in `playlist_result`'s key order.
pub(crate) fn playlist_result(
    page: &Url,
    extractor: &'static str,
    id: String,
    title: Option<String>,
    description: Option<Value>,
    entries: Vec<VideoInfo>,
    generator: bool,
) -> ExtractResult {
    let mut extra = Map::new();
    let mut keys = vec!["id"];
    if title.is_some() {
        keys.push("title");
    }
    if let Some(description) = description {
        keys.push("description");
        extra.insert("description".into(), description);
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!(
            "playlist-dict;{}{}",
            keys.join(","),
            if generator {
                crate::info_order::GENERATOR
            } else {
                ""
            }
        )
        .into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title: title.unwrap_or_default(),
        webpage_url: page.clone(),
        extractor: extractor.into(),
        extractor_key: extractor.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

impl Extractor for Nrk {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_GEO_COUNTRIES = ['NO']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["NO"])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        let mut video_id = temp_id.rsplit('/').next().unwrap_or_default().to_owned();
        start(NAME, url);
        let playback = |item: &str, query: &[(&str, &str)], video_id: &str| -> Result<Value> {
            let note = format!("Downloading {item} JSON");
            match call_api(
                client,
                NAME,
                &format!("playback/{item}/program/{video_id}"),
                video_id,
                &note,
                query,
            ) {
                Err(error) if http_status(&error) == Some(400) => call_api(
                    client,
                    NAME,
                    &format!("playback/{item}/{video_id}"),
                    video_id,
                    &note,
                    query,
                ),
                result => result,
            }
        };
        // Known `preferredCdn` values: akamai, globalconnect and telenor.
        let manifest = playback("manifest", &[("preferredCdn", "akamai")], &video_id)?;
        if !manifest.is_object() {
            return Err(unsupported("playback manifests that are not objects"));
        }
        if let Some(Value::String(id)) = manifest.get("id")
            && !id.is_empty()
        {
            video_id = id.clone();
        }
        if manifest.get("playability").and_then(Value::as_str) == Some("nonPlayable") {
            let data = manifest
                .get("nonPlayable")
                .ok_or_else(|| key_error(&temp_id, "nonPlayable"))?;
            return Err(raise_error(data, &temp_id));
        }
        let playable = manifest
            .get("playable")
            .ok_or_else(|| key_error(&temp_id, "playable"))?;
        let assets = match playable.get("assets") {
            None => return Err(key_error(&temp_id, "assets")),
            Some(Value::Array(assets)) => assets,
            Some(_) => return Err(unsupported("asset lists that are not arrays")),
        };
        let mut formats = Vec::new();
        for asset in assets {
            if !asset.is_object() {
                continue;
            }
            if super::lnk::truthy(asset.get("encrypted")) {
                continue;
            }
            let Some(format_url) = url_or_none(asset.get("url")) else {
                continue;
            };
            let asset_format = match asset.get("format") {
                Some(Value::String(text)) => text.to_lowercase(),
                value if !super::lnk::truthy(value) => String::new(),
                _ => return Err(unsupported("asset formats that are not strings")),
            };
            if asset_format == "hls"
                || super::direct::determine_ext(&format_url).as_deref() == Some("m3u8")
            {
                formats.extend(nrk_formats(client, &format_url, &video_id)?);
            } else if asset_format == "mp3" {
                formats.push(dict_format(
                    vec![
                        ("url", format_url.into()),
                        ("format_id", asset_format.into()),
                        ("vcodec", "none".into()),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        let data = playback("metadata", &[], &video_id)?;
        let preplay = data
            .get("preplay")
            .ok_or_else(|| key_error(&temp_id, "preplay"))?;
        let titles = preplay
            .get("titles")
            .ok_or_else(|| key_error(&temp_id, "titles"))?;
        let mut title = match titles.get("title") {
            None => return Err(key_error(&temp_id, "title")),
            Some(Value::String(title)) => title.clone(),
            Some(_) => return Err(unsupported("titles that are not strings")),
        };
        let alt_title = match titles.get("subtitle") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => return Err(unsupported("subtitles (alt titles) that are not strings")),
        };
        let description = preplay
            .get("description")
            .and_then(Value::as_str)
            .map(|text| text.replace('\r', "\n"));
        let duration = [playable.get("duration"), data.get("duration")]
            .into_iter()
            .filter_map(|value| value.and_then(Value::as_str))
            .filter_map(crate::parse_duration)
            .find(|duration| duration.as_f64() != 0.0)
            .map_or(Value::Null, |duration| {
                serde_json::to_value(duration).unwrap_or_default()
            });
        let mut thumbnails = Vec::new();
        if let Some(Value::Array(images)) = path(preplay, &["poster", "images"]) {
            for image in images {
                if !image.is_object() {
                    continue;
                }
                let Some(image_url) = url_or_none(image.get("url")) else {
                    continue;
                };
                let mut thumbnail = Map::new();
                thumbnail.insert("url".into(), image_url.into());
                if let Some(width) = int_or_none(image.get("pixelWidth")) {
                    thumbnail.insert("width".into(), width.into());
                }
                if let Some(height) = int_or_none(image.get("pixelHeight")) {
                    thumbnail.insert("height".into(), height.into());
                }
                thumbnails.push(Value::Object(thumbnail));
            }
        }
        let mut subtitles: Map<String, Value> = Map::new();
        if let Some(Value::Array(items)) = playable.get("subtitles") {
            for sub in items {
                if !sub.is_object() {
                    continue;
                }
                let Some(sub_url) = url_or_none(sub.get("webVtt")) else {
                    continue;
                };
                let mut sub_key = str_or_none(sub.get("language"))?
                    .filter(|key| !key.is_empty())
                    .unwrap_or_else(|| "nb".into());
                if let Some(sub_type) = str_or_none(sub.get("type"))?.filter(|key| !key.is_empty())
                {
                    sub_key.push('-');
                    sub_key.push_str(&sub_type);
                }
                let ext = super::direct::determine_ext(&sub_url)
                    .unwrap_or_else(|| "unknown_video".into())
                    .to_lowercase();
                subtitles
                    .entry(sub_key)
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .expect("subtitle lists")
                    .push(json!({"url": sub_url, "ext": ext}));
            }
        }
        // https://en.wikipedia.org/wiki/Norwegian_Media_Authority
        let age_limit = match path(&data, &["legalAge", "body", "rating", "code"]) {
            Some(Value::String(code)) if code == "A" => Value::from(0),
            Some(Value::String(code))
                if !code.is_empty() && code.chars().all(|c| c.is_ascii_digit()) =>
            {
                int_or_none(Some(&Value::String(code.clone()))).map_or(Value::Null, Value::from)
            }
            _ => Value::Null,
        };
        let is_series =
            path(&data, &["_links", "series", "name"]).and_then(Value::as_str) == Some("series");
        let timestamp = path(&manifest, &["availability", "onDemand", "from"])
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);
        let mut series_fields = Vec::new();
        if is_series {
            let programs = call_api_optional(
                client,
                NAME,
                &format!("programs/{video_id}"),
                &video_id,
                "Downloading programs JSON",
            )?;
            let (mut series, mut season_id, mut season_number, mut episode, mut episode_number) =
                (None, None, None, None, None);
            if let Some(programs) = programs.filter(Value::is_object) {
                series = str_or_none(programs.get("seriesTitle"))?;
                season_id = str_or_none(programs.get("seasonId"))?;
                season_number = int_or_none(programs.get("seasonNumber"));
                episode = str_or_none(programs.get("episodeTitle"))?;
                episode_number = int_or_none(programs.get("episodeNumber"));
            }
            let series = series
                .filter(|series| !series.is_empty())
                .unwrap_or_else(|| title.clone());
            if let Some(alt_title) = alt_title.as_deref().filter(|text| !text.is_empty()) {
                title = format!("{title} - {alt_title}");
            }
            let regex_number = |pattern: &Regex, text: Option<&str>| {
                pattern
                    .captures(text.unwrap_or_default())
                    .and_then(|found| found[1].parse::<i64>().ok())
            };
            if season_number.is_none_or(|number| number == 0) {
                season_number = regex_number(&SEASON_NUMBER, description.as_deref());
            }
            if episode.as_deref().is_none_or(str::is_empty) {
                episode = alt_title.clone();
            }
            if episode_number.is_none_or(|number| number == 0) {
                episode_number = regex_number(&EPISODE_NUMBER, episode.as_deref());
            }
            if episode_number.is_none_or(|number| number == 0) {
                episode_number = regex_number(&DESCRIPTION_EPISODE, description.as_deref());
            }
            series_fields = vec![
                ("series", Value::from(series)),
                ("season_id", season_id.map_or(Value::Null, Value::from)),
                (
                    "season_number",
                    season_number.map_or(Value::Null, Value::from),
                ),
                ("episode", episode.map_or(Value::Null, Value::from)),
                (
                    "episode_number",
                    episode_number.map_or(Value::Null, Value::from),
                ),
            ];
        }
        let subtitles: crate::Subtitles = serde_json::from_value(Value::Object(subtitles))
            .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        let mut fields = vec![
            ("id", Value::from(video_id)),
            ("title", Value::from(title)),
            ("alt_title", alt_title.map_or(Value::Null, Value::from)),
            ("description", description.map_or(Value::Null, Value::from)),
            ("duration", duration),
            ("thumbnails", Value::Array(thumbnails)),
            ("age_limit", age_limit),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("timestamp", timestamp),
        ];
        fields.extend(series_fields);
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        // Each thumbnail dict sets `url`, `width` and `height` (even `None`).
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail!=url,width,height");
        }
        Ok(Some(info))
    }
}

/// `url_result(f'nrk:{video_id}', ie=NRKIE.ie_key(), video_id=video_id)`
/// of a delegating extractor.
fn delegate(pattern: &Regex, extractor: &'static str, url: &Url) -> Result<Option<ExtractResult>> {
    let Some(found) = pattern.captures(url.as_str()) else {
        return Ok(None);
    };
    let video_id = found["id"].to_owned();
    start(extractor, url);
    Ok(Some(super::kaltura::follow(url, format!("nrk:{video_id}"))))
}

impl Extractor for NrkTv {
    fn key(&self) -> &'static str {
        TV_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        delegate(&TV_URL, TV_NAME, url)
    }
}

impl Extractor for NrkTvDirekte {
    fn key(&self) -> &'static str {
        DIREKTE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        delegate(&DIREKTE_URL, DIREKTE_NAME, url)
    }
}

impl Extractor for NrkRadioPodkast {
    fn key(&self) -> &'static str {
        PODKAST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        delegate(&PODKAST_URL, PODKAST_NAME, url)
    }
}

impl Extractor for NrkTvEpisode {
    fn key(&self) -> &'static str {
        EPISODE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = EPISODE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(EPISODE_NAME, url);
        let (webpage, final_url) = download_webpage_handle(client, url, EPISODE_NAME, &display_id)?;
        let nrk_id = if let Some(found) = TV_URL.captures(final_url.as_str()) {
            found["id"].to_owned()
        } else {
            let data = match search_json(&PAGE_DATA, &webpage, false) {
                Some(Some(data)) => data,
                Some(None) => {
                    return Err(unsupported("page data that does not parse"));
                }
                None => {
                    return Err(Error::Reported(format!(
                        "[{EPISODE_NAME}] {display_id}: Unable to extract page data; {}",
                        crate::BUG_REPORTS_MESSAGE
                    )));
                }
            };
            let state = data
                .get("initialState")
                .ok_or_else(|| key_error(&display_id, "initialState"))?;
            let nrk_id = match state.get("selectedEpisodePrfId") {
                None => return Err(key_error(&display_id, "selectedEpisodePrfId")),
                Some(Value::String(id)) => id.clone(),
                Some(_) => return Err(unsupported("episode IDs that are not strings")),
            };
            if !EPISODE_ID.is_match(&nrk_id) {
                return Err(Error::Reported(format!(
                    "[{EPISODE_NAME}] {display_id}: Unable to extract NRK ID; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            nrk_id
        };
        // `url_result(..., season_number=..., episode_number=...)`: a `url`
        // result, whose extra fields upstream does not pass on.
        Ok(Some(super::kaltura::follow(url, format!("nrk:{nrk_id}"))))
    }
}

/// `NRKTVSerieBaseIE._extract_entries(entry_list)`.
fn extract_entries(entry_list: Option<&Value>) -> Result<Vec<VideoInfo>> {
    let Some(Value::Array(items)) = entry_list else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for episode in items {
        if !episode.is_object() {
            return Err(unsupported("catalog entries that are not objects"));
        }
        let nrk_id = [episode.get("prfId"), episode.get("episodeId")]
            .into_iter()
            .flatten()
            .find(|value| super::lnk::truthy(Some(value)));
        let Some(Value::String(nrk_id)) = nrk_id else {
            continue;
        };
        entries.push(nrk_entry(nrk_id)?);
    }
    Ok(entries)
}

/// `NRKTVSerieBaseIE._entries(data, display_id)`, read eagerly.
fn catalog_entries(
    client: &Client,
    extractor: &'static str,
    data: &Value,
    display_id: &str,
) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    let mut data = data.clone();
    for page_num in 1.. {
        if !data.is_object() {
            return Err(unsupported("catalog pages that are not objects"));
        }
        let embedded = match data.get("_embedded") {
            Some(embedded) if super::lnk::truthy(Some(embedded)) => embedded,
            _ => &data,
        };
        if !embedded.is_object() {
            break;
        }
        let Some(assets_key) = ["episodes", "instalments"]
            .into_iter()
            .find(|key| super::lnk::truthy(embedded.get(*key)))
        else {
            break;
        };
        let list = [
            path(embedded, &[assets_key, "_embedded", assets_key]),
            embedded.get(assets_key),
        ]
        .into_iter()
        .flatten()
        .find(|value| value.is_array());
        entries.extend(extract_entries(list)?);
        let next = [
            path(&data, &["_links", "next", "href"]),
            path(&data, &["_embedded", assets_key, "_links", "next", "href"]),
        ]
        .into_iter()
        .flatten()
        .find_map(Value::as_str)
        .filter(|next| !next.is_empty())
        .map(str::to_owned);
        let Some(next) = next else {
            break;
        };
        let note = format!("Downloading {assets_key} JSON page {page_num}");
        match call_api_optional(client, extractor, &next, display_id, &note)? {
            Some(page) if super::lnk::truthy(Some(&page)) => data = page,
            _ => break,
        }
    }
    Ok(entries)
}

/// `NRKTVSerieBaseIE._catalog_name(serie_kind)`.
fn catalog_name(serie_kind: &str) -> &'static str {
    if matches!(serie_kind, "podcast" | "podkast") {
        "podcast"
    } else {
        "series"
    }
}

impl Extractor for NrkTvSeason {
    fn key(&self) -> &'static str {
        SEASON_NAME
    }

    /// `_GEO_COUNTRIES = ['NO']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["NO"])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !season_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = SEASON_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let domain = &found["domain"];
        let serie = &found["serie"];
        let season_id = found
            .name("id")
            .or_else(|| found.name("id_2"))
            .map_or("", |id| id.as_str());
        let display_id = format!("{serie}/{season_id}");
        start(SEASON_NAME, url);
        let data = call_api(
            client,
            SEASON_NAME,
            &format!(
                "{domain}/catalog/{}/{serie}/seasons/{season_id}",
                catalog_name(&found["serie_kind"])
            ),
            &display_id,
            "Downloading season JSON",
            &[("pageSize", "50")],
        )?;
        let title = path(&data, &["titles", "title"])
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .map_or_else(|| display_id.clone(), str::to_owned);
        let entries = catalog_entries(client, SEASON_NAME, &data, &display_id)?;
        Ok(Some(playlist_result(
            url,
            SEASON_NAME,
            display_id,
            Some(title),
            None,
            entries,
            true,
        )))
    }
}

/// NRKTVSeasonIE `get_temp_id(url)`.
fn season_temp_id(url: &str) -> Option<String> {
    SEASON_URL
        .captures(url)
        .and_then(|found| found.name("id").map(|id| id.as_str().to_owned()))
}

impl Extractor for NrkTvSeries {
    fn key(&self) -> &'static str {
        SERIES_NAME
    }

    /// `_GEO_COUNTRIES = ['NO']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["NO"])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !series_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let site = &found["domain"];
        let series_id = found["id"].to_owned();
        let is_radio = site == "radio.nrk";
        let domain = if is_radio { "radio" } else { "tv" };
        start(SERIES_NAME, url);
        let size_key = if is_radio {
            "pageSize"
        } else {
            "embeddedInstalmentsPageSize"
        };
        let series = call_api(
            client,
            SERIES_NAME,
            &format!(
                "{domain}/catalog/{}/{series_id}",
                catalog_name(&found["serie_kind"])
            ),
            &series_id,
            "Downloading serie JSON",
            &[(size_key, "50")],
        )?;
        if !series.is_object() {
            return Err(unsupported("series catalogs that are not objects"));
        }
        // `try_get(series, [titles, series[type].titles,
        // series[seriesType].titles]) or {}`.
        let by_kind = |kind: &str| {
            series
                .get(kind)
                .and_then(Value::as_str)
                .and_then(|kind| path(&series, &[kind, "titles"]))
        };
        let titles = series
            .get("titles")
            .or_else(|| by_kind("type"))
            .or_else(|| by_kind("seriesType"))
            .filter(|titles| super::lnk::truthy(Some(titles)))
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !titles.is_object() {
            return Err(unsupported("series titles that are not objects"));
        }
        let mut entries = catalog_entries(client, SERIES_NAME, &series, &series_id)?;
        let empty = json!({});
        let embedded = series
            .get("_embedded")
            .filter(|embedded| super::lnk::truthy(Some(embedded)))
            .unwrap_or(&empty);
        let linked = match path(&series, &["_links", "seasons"]) {
            Some(Value::Array(seasons)) => seasons.clone(),
            Some(value) if super::lnk::truthy(Some(value)) => {
                return Err(unsupported("season links that are not arrays"));
            }
            _ => Vec::new(),
        };
        let embedded_seasons = match embedded.get("seasons") {
            Some(Value::Array(seasons)) => seasons.clone(),
            Some(value) if super::lnk::truthy(Some(value)) => {
                return Err(unsupported("embedded seasons that are not arrays"));
            }
            _ => Vec::new(),
        };
        if linked.len() > embedded_seasons.len() {
            for season in &linked {
                if !season.is_object() {
                    return Err(unsupported("season links that are not objects"));
                }
                let mut season_url = season
                    .get("href")
                    .and_then(Value::as_str)
                    .filter(|href| !href.is_empty())
                    .and_then(|href| url.join(href).ok())
                    .map(|season_url| season_url.to_string());
                if season_url.is_none()
                    && let Some(Value::String(name)) = season.get("name")
                    && !name.is_empty()
                {
                    season_url = Some(format!(
                        "https://{domain}.nrk.no/serie/{series_id}/sesong/{name}"
                    ));
                }
                let Some(season_url) = season_url else {
                    continue;
                };
                let title = str_or_none(season.get("title"))?;
                entries.push(url_entry(
                    &season_url,
                    SEASON_NAME,
                    None,
                    season_temp_id(&season_url),
                    title,
                )?);
            }
        } else {
            for season in &embedded_seasons {
                entries.extend(catalog_entries(client, SERIES_NAME, season, &series_id)?);
            }
        }
        let extra_material = embedded
            .get("extraMaterial")
            .filter(|value| super::lnk::truthy(Some(value)))
            .cloned()
            .unwrap_or_else(|| json!({}));
        entries.extend(catalog_entries(
            client,
            SERIES_NAME,
            &extra_material,
            &series_id,
        )?);
        let title = match titles.get("title") {
            Some(Value::String(title)) if !title.is_empty() => Some(title.clone()),
            None | Some(Value::Null) | Some(Value::String(_)) => None,
            Some(_) => return Err(unsupported("series titles that are not strings")),
        };
        let description = titles
            .get("subtitle")
            .filter(|value| !value.is_null())
            .cloned();
        Ok(Some(playlist_result(
            url,
            SERIES_NAME,
            series_id,
            title,
            description,
            entries,
            false,
        )))
    }
}

impl Extractor for NrkPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(id) = playlist_id(url.as_str()) else {
            return Ok(None);
        };
        start(PLAYLIST_NAME, url);
        let webpage = download_webpage(client, url, PLAYLIST_NAME, &id)?;
        let entries = PLAYLIST_ITEM
            .captures_iter(&webpage)
            .map(|found| {
                let target = format!("nrk:{}", &found[1]);
                let temp_id = VALID_URL
                    .captures(&target)
                    .map(|found| found["id"].to_owned());
                url_entry(&target, NAME, None, temp_id, None)
            })
            .collect::<Result<Vec<_>>>()?;
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            super::search_warning(PLAYLIST_NAME, "OpenGraph title");
        }
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            super::search_warning(PLAYLIST_NAME, "OpenGraph description");
        }
        Ok(Some(playlist_result(
            url,
            PLAYLIST_NAME,
            id,
            title.filter(|title| !title.is_empty()),
            description.map(Value::from),
            entries,
            false,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        for url in [
            "http://www.nrk.no/video/PS*150533",
            "nrk:ecc1b952-96dc-4a98-81b9-5296dc7a98d9",
            "nrk:clip/7707d5a3-ebe7-434a-87d5-a3ebe7a34a70",
            "https://v8-psapi.nrk.no/mediaelement/ecc1b952-96dc-4a98-81b9-5296dc7a98d9",
            "https://www.nrk.no/video/dompap-og-andre-fugler-i-piip-show_150533",
            "nrk:podcast/l_96f4f1b0-de54-4e6a-b4f1-b0de54fe6af8",
            "nrk:program/ENRK10100318",
            "nrk:channel/nrk1",
        ] {
            assert!(suitable(url), "{url}");
        }
        for url in [
            "https://tv.nrk.no/program/MDDP12000117",
            "https://tv.nrk.no/serie/20-spoersmaal-tv/MUHH48000314/23-05-2014",
            "https://radio.nrk.no/serie/dagsnytt/sesong/201507/NPUB21019315",
            "https://tv.nrk.no/serie/lindmo/2018/MUHU11006318/avspiller",
        ] {
            assert!(tv_suitable(url), "{url}");
            assert!(!season_suitable(url) && !series_suitable(url), "{url}");
        }
        assert!(direkte_suitable(
            "https://radio.nrk.no/direkte/p1_oslo_akershus"
        ));
        assert!(podkast_suitable(
            "https://radio.nrk.no/podkast/hele_historien/sesong/bortfoert-i-bergen/l_774d1a2c-7aa7-4965-8d1a-2c7aa7d9652c"
        ));
        let episode = "https://tv.nrk.no/serie/hellums-kro/sesong/1/episode/2";
        assert!(episode_suitable(episode) && !season_suitable(episode));
        for url in [
            "https://tv.nrk.no/serie/backstage/sesong/1",
            "https://tv.nrk.no/serie/lindmo/2016",
            "https://radio.nrk.no/podkast/hele_historien/sesong/diagnose-kverulant",
        ] {
            assert!(season_suitable(url) && !series_suitable(url), "{url}");
        }
        for url in [
            "https://tv.nrk.no/serie/groenn-glede",
            "https://tv.nrksuper.no/serie/labyrint",
            "https://nrksuper.no/serie/labyrint",
            "https://radio.nrk.no/podkast/ulrikkes_univers",
            "https://radio.nrk.no/podkast/ulrikkes_univers/nrkno-poddkast-26588-134079-05042018030000",
        ] {
            assert!(series_suitable(url), "{url}");
        }
        assert_eq!(
            playlist_id(
                "http://www.nrk.no/troms/gjenopplev-den-historiske-solformorkelsen-1.12270763"
            )
            .as_deref(),
            Some("gjenopplev-den-historiske-solformorkelsen-1.12270763")
        );
        assert!(!playlist_suitable("https://www.nrk.no/video/x/y"));
        assert!(!playlist_suitable("https://www.nrk.no/skole/x/y"));
        assert!(!playlist_suitable("https://www.nrk.no/single"));
    }

    #[test]
    fn python_int_and_str_conversions() {
        assert_eq!(int_or_none(Some(&json!("2014"))), Some(2014));
        assert_eq!(int_or_none(Some(&json!(" 7 "))), Some(7));
        assert_eq!(int_or_none(Some(&json!("1.5"))), None);
        assert_eq!(int_or_none(Some(&json!(2.7))), Some(2));
        assert_eq!(py_str(&json!(126936)).unwrap(), "126936");
        assert_eq!(py_str(&json!(true)).unwrap(), "True");
    }

    #[test]
    fn raised_errors_follow_upstream_messages() {
        let expired = json!({"messageType": "ProgramRightsHasExpired", "endUserMessage": null});
        assert_eq!(
            raise_error(&expired, "X").to_string(),
            "[NRK] X: NRK said: Programmet har gått ut"
        );
        let custom = json!({"messageType": "Other"});
        assert_eq!(
            raise_error(&custom, "X").to_string(),
            "[NRK] X: NRK said: Other"
        );
    }
}
