//! Upstream `SVTPlayIE`, `SVTSeriesIE` and `SVTPageIE`
//! (`yt_dlp/extractor/svt.py`): SVT Play / Öppet arkiv videos through the
//! page's `URQL_DATA` store (or a `modalId`/`id` query, or a Barnplay URL's
//! ID) and the videoplayer API (`SVTBaseIE._extract_video`: HLS, DASH and
//! plain `videoReferences`, subtitle references with the `-forced` fixup,
//! metadata and the Sweden-only geo error), a series' GraphQL seasons as
//! `svt:ID` `url` results, and an svt.se article's `urqlState` videos as a
//! playlist of API results.
//!
//! Not supported natively: `f4m` video references (an explicit error).
use super::{
    Extractor, dict_format, dict_info, download_json, download_json_note, download_webpage,
    extract_m3u8_master_optional, extract_mpd_optional_note, search_json, search_warning,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, Subtitles, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct SvtPlay;
pub struct SvtSeries;
pub struct SvtPage;

const PLAY_NAME: &str = "svt:play";
const PLAY_KEY: &str = "SVTPlay";
const SERIES_NAME: &str = "svt:play:series";
const SERIES_KEY: &str = "SVTSeries";
const PAGE_NAME: &str = "svt:page";
const PAGE_KEY: &str = "SVTPage";

static PLAY_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:(?:svt:|https?://(?:www\.)?svt\.se/barnkanalen/barnplay/[^/]+/)(?P<svt_id>[^/?#&]+)|https?://(?:www\.)?(?:svtplay|oppetarkiv)\.se/(?:video|klipp|kanaler)/(?P<id>[^/?#&]+)(?:.*?(?:modalId|id)=(?P<modal_id>[\da-zA-Z-]+))?)",
    )
    .unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?svtplay\.se/(?P<id>[^/?&#]+)(?:.+?\btab=(?P<season_slug>[^&#]+))?",
    )
    .unwrap()
});
static PAGE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?svt\.se/(?:[^/?#]+/)*(?P<id>[^/?&#]+)").unwrap()
});
/// `(?<=[\s.])URQL_DATA\s*=` without look-behind (the match end is the same).
static URQL_DATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\s.]URQL_DATA\s*=").unwrap());
static URQL_STATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"urqlState\s*[=:]").unwrap());
static TITLE_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\|\s*.+?$").unwrap());

/// `SVTPlayIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    PLAY_URL.is_match(url)
}

/// `SVTSeriesIE.suitable`: not an `SVTPlayIE` URL.
pub fn series_suitable(url: &str) -> bool {
    !suitable(url) && SERIES_URL.is_match(url)
}

/// `SVTPageIE.suitable`: not an `SVTPlayIE` URL.
pub fn page_suitable(url: &str) -> bool {
    !suitable(url) && PAGE_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("SVT {what} are not supported natively"))
}

/// `[NAME] ID: ` (just `[NAME] ` without an ID, like `ExtractorError`).
fn prefix(name: &str, id: Option<&str>) -> String {
    match id {
        Some(id) => format!("[{name}] {id}: "),
        None => format!("[{name}] "),
    }
}

/// A fatal `_search_regex`/`require` miss.
fn bug(name: &str, id: Option<&str>, message: &str) -> Error {
    Error::Reported(format!(
        "{}{message}; {}",
        prefix(name, id),
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: Option<&str>, key: &str) -> Error {
    Error::Reported(format!(
        "{}An extractor error has occurred. (caused by KeyError('{key}')); {}",
        id.map(|id| format!("{id}: ")).unwrap_or_default(),
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// `dict_get(d, keys, skip_false_values=SKIP)`: the first present, non-`None`
/// (and, when skipping, truthy) value.
fn dict_get<'a>(value: &'a Value, keys: &[&str], skip_false: bool) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find(|found| !found.is_null() && (!skip_false || truthy(found)))
}

/// `int_or_none`: Python's `int()` of numbers, booleans and integer text.
fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Bool(flag)) => Value::from(i64::from(*flag)),
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text
            .trim()
            .replace('_', "")
            .parse::<i64>()
            .map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

/// A string-or-`None` metadata value.
fn text_field(value: Option<&Value>, what: &str) -> Result<Value> {
    match value {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::String(text)) => Ok(text.clone().into()),
        Some(_) => Err(unsupported(&format!("{what} values that are not strings"))),
    }
}

/// `_merge_subtitles(subs, target=subtitles)`: tracks with an already listed
/// URL (and data) are dropped.
fn merge_subtitles(target: &mut Subtitles, source: Subtitles) {
    for (language, tracks) in source.entries {
        let existing = match target.entries.iter_mut().find(|(key, _)| *key == language) {
            Some((_, existing)) => existing,
            None => {
                target.entries.push((language, Vec::new()));
                &mut target.entries.last_mut().expect("just pushed").1
            }
        };
        let known: Vec<(Url, Option<Value>)> = existing
            .iter()
            .map(|track| (track.url.clone(), track.extra.get("data").cloned()))
            .collect();
        existing.extend(tracks.into_iter().filter(|track| {
            !known.contains(&(track.url.clone(), track.extra.get("data").cloned()))
        }));
    }
}

/// `SVTBaseIE._fixup_subtitles`: tracks of a `text-open` URL move to
/// `LANG-forced`.
fn fixup_subtitles(subtitles: Subtitles) -> Subtitles {
    let mut fixed = Subtitles::default();
    for (language, tracks) in subtitles.entries {
        for track in tracks {
            let key = if track.url.as_str().contains("text-open") {
                Some(format!("{}-forced", language.as_deref().unwrap_or("None")))
            } else {
                language.clone()
            };
            match fixed.entries.iter_mut().find(|(known, _)| *known == key) {
                Some((_, existing)) => existing.push(track),
                None => fixed.entries.push((key, vec![track])),
            }
        }
    }
    fixed
}

/// An extracted video: the info dict's fields (`formats` and `subtitles` mark
/// their positions), formats and subtitles.
type Video = (Vec<(&'static str, Value)>, Vec<VideoFormat>, Subtitles);

/// `SVTBaseIE._extract_video(video_info, video_id)`. `temp_id` is the URL's
/// ID that upstream's errors carry.
fn extract_video(
    client: &Client,
    video_info: &Value,
    video_id: &str,
    name: &'static str,
    temp_id: Option<&str>,
) -> Result<Video> {
    let is_live = dict_get(video_info, &["live", "simulcast"], true)
        .cloned()
        .unwrap_or(Value::Bool(false));
    if !is_live.is_boolean() {
        return Err(unsupported("live flags that are not booleans"));
    }
    let live = is_live == Value::Bool(true);
    let references = match video_info.get("videoReferences") {
        None => return Err(key_error(temp_id, "videoReferences")),
        Some(Value::Array(references)) => references,
        Some(_) => return Err(unsupported("videoReferences that are not lists")),
    };
    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    for reference in references {
        if !reference.is_object() {
            return Err(unsupported("video references that are not objects"));
        }
        // `vr.get('playerType') or vr.get('format')`.
        let player_type = match reference
            .get("playerType")
            .filter(|value| truthy(value))
            .or_else(|| reference.get("format"))
        {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => return Err(unsupported("player types that are not strings")),
        };
        let video_url = match reference.get("url") {
            None => return Err(key_error(temp_id, "url")),
            Some(Value::String(text)) => text.clone(),
            Some(_) => return Err(unsupported("video reference URLs that are not strings")),
        };
        let ext = super::direct::determine_ext(&video_url).unwrap_or_default();
        match ext.as_str() {
            "m3u8" => {
                let manifest = Url::parse(&video_url)?;
                if let Some(stream) = extract_m3u8_master_optional(
                    client,
                    &manifest,
                    name,
                    video_id,
                    player_type.as_deref(),
                    true,
                    false,
                ) {
                    // `entry_protocol='m3u8' if is_live else 'm3u8_native'`.
                    formats.extend(stream.formats.into_iter().map(|mut format| {
                        if live {
                            format.protocol = Some("m3u8".into());
                        }
                        format
                    }));
                    merge_subtitles(&mut subtitles, stream.subtitles);
                }
            }
            "f4m" => return Err(unsupported("f4m (HDS) video references")),
            "mpd" => {
                let manifest = Url::parse(&video_url)?;
                if let Some(stream) = extract_mpd_optional_note(
                    client,
                    &manifest,
                    name,
                    video_id,
                    player_type.as_deref(),
                    "Downloading MPD manifest",
                ) {
                    formats.extend(stream.formats);
                    merge_subtitles(&mut subtitles, stream.subtitles);
                }
            }
            _ => formats.push(dict_format(
                vec![
                    ("format_id", player_type.map_or(Value::Null, Value::from)),
                    ("url", video_url.into()),
                ],
                name,
                video_id,
            )?),
        }
    }
    let empty = Value::Object(Map::new());
    let rights = video_info
        .get("rights")
        .filter(|rights| rights.is_object())
        .unwrap_or(&empty);
    if formats.is_empty() && rights.get("geoBlockedSweden").is_some_and(truthy) {
        // `raise_geo_restricted(msg, countries=['SE'], metadata_available=True)`.
        return Err(Error::Reported(format!(
            "{}This video is only available in Sweden\nThis video is available in Sweden.\nYou might want to use a VPN or a proxy server (with --proxy) to workaround.",
            prefix(name, temp_id)
        )));
    }

    if let Some(Value::Array(references)) =
        dict_get(video_info, &["subtitles", "subtitleReferences"], true)
    {
        for reference in references {
            if !reference.is_object() {
                return Err(unsupported("subtitle references that are not objects"));
            }
            // `sr.get('language', 'sv')`.
            let language = match reference.get("language") {
                None => Some("sv".to_owned()),
                Some(Value::Null) => None,
                Some(Value::String(text)) => Some(text.clone()),
                Some(_) => return Err(unsupported("subtitle languages that are not strings")),
            };
            let Some(subtitle_url) = reference.get("url").filter(|url| truthy(url)) else {
                continue;
            };
            let Value::String(subtitle_url) = subtitle_url else {
                return Err(unsupported("subtitle URLs that are not strings"));
            };
            let parsed = Url::parse(subtitle_url)
                .map_err(|_| unsupported("subtitle URLs that are not absolute"))?;
            let ext = super::direct::determine_ext(subtitle_url);
            // Processing's `determine_ext(url).lower()` unless set to `vtt`.
            let ext = match ext.as_deref() {
                Some("m3u8") => "vtt".to_owned(),
                Some(ext) => ext.to_lowercase(),
                None => "unknown_video".to_owned(),
            };
            let track = crate::SubtitleFormat {
                name: None,
                url: parsed,
                ext,
                protocol: None,
                fragments: Vec::new(),
                extra: Map::new(),
            };
            match subtitles
                .entries
                .iter_mut()
                .find(|(key, _)| *key == language)
            {
                Some((_, existing)) => existing.push(track),
                None => subtitles.entries.push((language, vec![track])),
            }
        }
    }

    let timestamp = rights
        .get("validFrom")
        .and_then(Value::as_str)
        .and_then(|date| crate::dates::unified_timestamp(date, false))
        .map_or(Value::Null, Value::from);
    let age_limit = dict_get(
        video_info,
        &["inappropriateForChildren", "blockedForChildren"],
        false,
    )
    .map_or(Value::Null, |adult| {
        Value::from(if truthy(adult) { 18 } else { 0 })
    });
    let fields = vec![
        ("id", Value::from(video_id)),
        ("title", text_field(video_info.get("title"), "titles")?),
        ("formats", Value::Null),
        ("subtitles", Value::Null),
        (
            "duration",
            int_or_none(dict_get(
                video_info,
                &["materialLength", "contentDuration"],
                true,
            )),
        ),
        ("timestamp", timestamp),
        ("age_limit", age_limit),
        (
            "series",
            text_field(video_info.get("programTitle"), "program titles")?,
        ),
        ("season_number", int_or_none(video_info.get("season"))),
        (
            "episode",
            text_field(video_info.get("episodeTitle"), "episode titles")?,
        ),
        (
            "episode_number",
            int_or_none(video_info.get("episodeNumber")),
        ),
        ("is_live", is_live),
    ];
    Ok((fields, formats, fixup_subtitles(subtitles)))
}

/// `str_or_none` of an `URQL_DATA` `videoSvtId`.
fn svt_id_text(value: &Value) -> Result<Option<String>> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        Value::Number(number) => Ok(Some(number.to_string())),
        Value::Bool(flag) => Ok(Some(if *flag { "True" } else { "False" }.into())),
        _ => Err(unsupported("SVT IDs that are not strings")),
    }
}

/// `traverse_obj(data, ({dict.values}, ..., 'data', {json.loads},
/// 'detailsPageByPath', 'smartStart', 'videoSvtId', {str_or_none}, any))`.
fn urql_svt_id(data: &Value) -> Result<Option<String>> {
    let Value::Object(entries) = data else {
        return Ok(None);
    };
    for entry in entries.values() {
        let Some(Value::String(text)) = entry.get("data") else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        let found = parsed
            .get("detailsPageByPath")
            .and_then(|page| page.get("smartStart"))
            .and_then(|start| start.get("videoSvtId"));
        if let Some(found) = found
            && let Some(id) = svt_id_text(found)?
        {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

impl Extractor for SvtPlay {
    fn key(&self) -> &'static str {
        PLAY_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = PLAY_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found.name("id").map(|id| id.as_str().to_owned());
        let mut svt_id = found
            .name("svt_id")
            .or_else(|| found.name("modal_id"))
            .map(|id| id.as_str().to_owned());
        announce(PLAY_NAME, url);
        // `_download_webpage(url, video_id, fatal=not svt_id) or ''`: the
        // request of an `svt:ID` pseudo-URL fails in upstream's networking.
        if url.scheme() == "svt" {
            return Err(Error::Reported(
                "Unable to handle request: Unsupported url scheme: \"svt\" (urllib)".into(),
            ));
        }
        let label = video_id.as_deref().unwrap_or("None");
        let webpage = match download_webpage(client, url, PLAY_NAME, label) {
            Ok(webpage) => webpage,
            Err(Error::ExtractorRequest { source, .. }) if svt_id.is_some() => {
                crate::screen::warning(&format!(
                    "{}Unable to download webpage: {}",
                    prefix(PLAY_NAME, video_id.as_deref()),
                    crate::upstream_error_text(&source)
                ));
                String::new()
            }
            Err(error) => return Err(error),
        };
        if svt_id.is_none() {
            let data = match search_json(&URQL_DATA, &webpage, true) {
                Some(Some(data)) => data,
                Some(None) => {
                    return Err(Error::Reported(format!(
                        "[{PLAY_NAME}] {label}: Unable to extract json data - Failed to parse JSON"
                    )));
                }
                None => {
                    return Err(bug(PLAY_NAME, Some(label), "Unable to extract json data"));
                }
            };
            svt_id = Some(
                urql_svt_id(&data)?
                    .ok_or_else(|| bug(PLAY_NAME, Some(label), "Unable to extract SVT ID"))?,
            );
        }
        let svt_id = svt_id.expect("set above");

        // `_extract_by_video_id(svt_id)`.
        let api = Url::parse(&format!(
            "https://api.svt.se/videoplayer-api/video/{svt_id}"
        ))?;
        let data = download_json(client, &api, PLAY_NAME, &svt_id)?;
        let (mut fields, formats, subtitles) =
            extract_video(client, &data, &svt_id, PLAY_NAME, video_id.as_deref())?;
        let field = |fields: &[(&str, Value)], key: &str| {
            fields
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
                .unwrap_or(Value::Null)
        };
        let mut title = field(&fields, "title");
        if !truthy(&title) {
            // `traverse_obj(info_dict, 'episode', 'series')`.
            let episode = field(&fields, "episode");
            title = if episode.is_null() {
                field(&fields, "series")
            } else {
                episode
            };
        }
        if !truthy(&title) {
            let Some(og_title) = crate::html::og_search_property(&webpage, "title") else {
                search_warning(PLAY_NAME, "OpenGraph title");
                // `re.sub(..., None)`.
                return Err(Error::Reported(
                    "expected string or bytes-like object, got 'NoneType'".into(),
                ));
            };
            title = TITLE_SUFFIX.replace_all(&og_title, "").into_owned().into();
        }
        fields[1].1 = title;
        // `_og_search_description(webpage, default=None)`: the API's videos
        // carry no description.
        let description = crate::html::og_search_property(&webpage, "description");
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            search_warning(PLAY_NAME, "thumbnail URL");
        }
        fields.push(("description", description.map_or(Value::Null, Value::from)));
        fields.push(("thumbnail", thumbnail.map_or(Value::Null, Value::from)));
        let mut info = dict_info(url, (PLAY_NAME, PLAY_KEY), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

/// `url_result('svt:' + content_id, SVTPlayIE.ie_key(), content_id)`.
fn play_entry(content_id: &str) -> Result<VideoInfo> {
    let target = format!("svt:{content_id}");
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), PLAY_KEY.into());
    entry.insert("id".into(), content_id.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.clone().into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": PLAY_KEY,
            "extractor": PLAY_NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// Python's `urlencode({'query': query})` (`quote_plus`).
fn graphql_url(query: &str) -> Result<Url> {
    const PLUS: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'_')
        .remove(b'.')
        .remove(b'-')
        .remove(b'~')
        .remove(b' ');
    let encoded = percent_encoding::utf8_percent_encode(query, PLUS)
        .to_string()
        .replace(' ', "+");
    Ok(Url::parse(&format!(
        "https://api.svt.se/contento/graphql?query={encoded}"
    ))?)
}

fn series_query(slug: &str) -> String {
    format!(
        "{{\n  listablesBySlug(slugs: [\"{slug}\"]) {{\n    associatedContent(include: [productionPeriod, season]) {{\n      items {{\n        item {{\n          ... on Episode {{\n            videoSvtId\n          }}\n        }}\n      }}\n      id\n      name\n    }}\n    id\n    longDescription\n    name\n    shortDescription\n  }}\n}}"
    )
}

impl Extractor for SvtSeries {
    fn key(&self) -> &'static str {
        SERIES_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let series_slug = found["id"].to_owned();
        let season_id = found.name("season_slug").map(|id| id.as_str().to_owned());
        announce(SERIES_NAME, url);
        let api = graphql_url(&series_query(&series_slug))?;
        let response = download_json_note(
            client,
            &api,
            SERIES_NAME,
            &series_slug,
            "Downloading series page",
        )?;
        // `[...]['data']['listablesBySlug'][0]`.
        let temp_id = Some(series_slug.as_str());
        let data = response
            .get("data")
            .ok_or_else(|| key_error(temp_id, "data"))?;
        let listables = data
            .get("listablesBySlug")
            .ok_or_else(|| key_error(temp_id, "listablesBySlug"))?;
        let Some(series) = listables.as_array().and_then(|items| items.first()) else {
            return Err(unsupported("series responses without a listable"));
        };
        if !series.is_object() {
            return Err(unsupported("series listables that are not objects"));
        }

        let mut season_name = Value::Null;
        let mut entries = Vec::new();
        let seasons = match series.get("associatedContent") {
            None => return Err(key_error(temp_id, "associatedContent")),
            Some(Value::Array(seasons)) => seasons,
            Some(_) => return Err(unsupported("series contents that are not lists")),
        };
        for season in seasons {
            if !season.is_object() {
                continue;
            }
            if let Some(season_id) = &season_id {
                if season.get("id").and_then(Value::as_str) != Some(season_id) {
                    continue;
                }
                season_name = season.get("name").cloned().unwrap_or(Value::Null);
            }
            let Some(Value::Array(items)) = season.get("items") else {
                continue;
            };
            for item in items {
                if !item.is_object() {
                    return Err(unsupported("series items that are not objects"));
                }
                // `item.get('item') or {}`.
                let video = match item.get("item") {
                    Some(video) if truthy(video) => video,
                    _ => continue,
                };
                if !video.is_object() {
                    return Err(unsupported("series items that are not objects"));
                }
                let Some(Value::String(content_id)) = video.get("videoSvtId") else {
                    continue;
                };
                if content_id.is_empty() {
                    continue;
                }
                entries.push(play_entry(content_id)?);
            }
        }

        let text = |value: Option<&Value>, what: &str| -> Result<Option<String>> {
            match value {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) => Ok(Some(text.clone())),
                Some(_) => Err(unsupported(&format!("series {what} that are not strings"))),
            }
        };
        let name = text(series.get("name"), "names")?;
        let season_name = text(Some(&season_name), "season names")?
            .filter(|name| !name.is_empty())
            .or_else(|| season_id.clone());
        let title = match (&name, &season_name) {
            (Some(name), Some(season)) if !name.is_empty() => Some(format!("{name} - {season}")),
            _ if season_id.is_some() => season_id.clone(),
            _ => name,
        };
        let id = match &season_id {
            Some(id) => Some(id.clone()),
            None => text(series.get("id"), "IDs")?,
        };
        let description = text(
            dict_get(series, &["longDescription", "shortDescription"], true),
            "descriptions",
        )?;

        // `playlist_result(entries, id, title, description)`: falsy IDs and
        // titles and a `None` description are left out.
        let id = id.filter(|id| !id.is_empty());
        let title = title.filter(|title| !title.is_empty());
        let mut keys = Vec::new();
        if id.is_some() {
            keys.push("id");
        }
        if title.is_some() {
            keys.push("title");
        }
        let mut extra = Map::new();
        if let Some(description) = description {
            keys.push("description");
            extra.insert("description".into(), description.into());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: id.unwrap_or_default(),
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: SERIES_NAME.into(),
            extractor_key: SERIES_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// `traverse_obj(urql_state, (..., 'data', {str}, {json.loads}),
/// get_all=False)`.
fn urql_page_data(state: &Value) -> Option<Value> {
    let values: Vec<&Value> = match state {
        Value::Object(map) => map.values().collect(),
        Value::Array(items) => items.iter().collect(),
        _ => Vec::new(),
    };
    values.into_iter().find_map(|value| {
        let Some(Value::String(text)) = value.get("data") else {
            return None;
        };
        serde_json::from_str::<Value>(text)
            .ok()
            .filter(|parsed| !parsed.is_null() && *parsed != Value::Object(Map::new()))
    })
}

/// `traverse_obj(data, ('page', (('topMedia', 'svtId'), ('body', ...,
/// 'video', 'svtId')), {str}))`, without duplicates.
fn page_video_ids(data: &Value) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let Some(page) = data.get("page") else {
        return ids;
    };
    let mut found = Vec::new();
    if let Some(id) = page.get("topMedia").and_then(|media| media.get("svtId")) {
        found.push(id);
    }
    let body: Vec<&Value> = match page.get("body") {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(map)) => map.values().collect(),
        _ => Vec::new(),
    };
    for block in body {
        if let Some(id) = block.get("video").and_then(|video| video.get("svtId")) {
            found.push(id);
        }
    }
    for id in found {
        if let Value::String(id) = id
            && !ids.contains(id)
        {
            ids.push(id.clone());
        }
    }
    ids
}

impl Extractor for SvtPage {
    fn key(&self) -> &'static str {
        PAGE_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = PAGE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        announce(PAGE_NAME, url);
        let webpage = download_webpage(client, url, PAGE_NAME, &display_id)?;
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            search_warning(PAGE_NAME, "OpenGraph title");
        }
        let urql_state = match search_json(&URQL_STATE, &webpage, false) {
            Some(Some(state)) => state,
            Some(None) => {
                return Err(Error::Reported(format!(
                    "[{PAGE_NAME}] {display_id}: Unable to extract json data - Failed to parse JSON"
                )));
            }
            None => {
                return Err(bug(
                    PAGE_NAME,
                    Some(&display_id),
                    "Unable to extract json data",
                ));
            }
        };
        // `... or {}`.
        let data = urql_page_data(&urql_state)
            .filter(truthy)
            .unwrap_or_else(|| Value::Object(Map::new()));

        // Upstream iterates a `set` of the IDs (in hash order); these keep
        // their page order.
        let mut entries = Vec::new();
        for video_id in page_video_ids(&data) {
            let api = Url::parse(&format!("https://api.svt.se/video/{video_id}"))?;
            let video = download_json(client, &api, PAGE_NAME, &video_id)?;
            let (mut fields, formats, subtitles) =
                extract_video(client, &video, &video_id, PAGE_NAME, Some(&display_id))?;
            fields[1].1 = title.clone().map_or(Value::Null, Value::from);
            let mut info = dict_info(url, (PAGE_NAME, PAGE_KEY), fields, formats);
            info.subtitles = subtitles;
            entries.push(info);
        }

        let title = title.filter(|title| !title.is_empty());
        let keys = if title.is_some() { "id,title" } else { "id" };
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}{}", crate::info_order::GENERATOR).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: PAGE_NAME.into(),
            extractor_key: PAGE_KEY.into(),
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
        for url in [
            "https://www.svtplay.se/video/eXYgwZb/sverige-och-kriget/1-utbrottet",
            "https://www.svtplay.se/video/30479064",
            "https://www.svtplay.se/video/30684086/rapport/rapport-24-apr-18-00-7?id=e72gVpa",
            "http://www.oppetarkiv.se/video/5219710/trollflojten",
            "http://www.svtplay.se/klipp/9023742/stopptid-om-bjorn-borg",
            "https://www.svtplay.se/kanaler/svt1",
            "svt:1376446-003A",
            "svt:14278044",
            "https://www.svt.se/barnkanalen/barnplay/kar/eWv5MLX/",
        ] {
            assert!(suitable(url), "{url}");
            assert!(!series_suitable(url), "{url}");
            assert!(!page_suitable(url), "{url}");
        }
        let modal = PLAY_URL
            .captures("https://www.svtplay.se/video/30479064/husdrommar/x?modalId=8zVbDPA")
            .unwrap();
        assert_eq!(&modal["id"], "30479064");
        assert_eq!(&modal["modal_id"], "8zVbDPA");
        assert!(series_suitable("https://www.svtplay.se/rederiet"));
        let season = SERIES_URL
            .captures("https://www.svtplay.se/rederiet?tab=season-2-jpmQYgn")
            .unwrap();
        assert_eq!(&season["season_slug"], "season-2-jpmQYgn");
        assert!(page_suitable(
            "https://www.svt.se/vader/manadskronikor/maj2018"
        ));
        assert!(!page_suitable("https://www.svtplay.se/rederiet"));
    }

    #[test]
    fn subtitles_merge_without_duplicates_and_mark_forced_tracks() {
        let track = |url: &str| crate::SubtitleFormat {
            name: None,
            url: Url::parse(url).unwrap(),
            ext: "vtt".into(),
            protocol: None,
            fragments: Vec::new(),
            extra: Map::new(),
        };
        let mut target = Subtitles {
            entries: vec![(
                Some("sv".into()),
                vec![track("https://a.test/text-open.m3u8")],
            )],
        };
        merge_subtitles(
            &mut target,
            Subtitles {
                entries: vec![(
                    Some("sv".into()),
                    vec![
                        track("https://a.test/text-open.m3u8"),
                        track("https://a.test/closed.vtt"),
                    ],
                )],
            },
        );
        let fixed = fixup_subtitles(target);
        let languages: Vec<_> = fixed
            .entries
            .iter()
            .map(|(language, tracks)| (language.clone().unwrap(), tracks.len()))
            .collect();
        assert_eq!(
            languages,
            [("sv-forced".to_owned(), 1), ("sv".to_owned(), 1)]
        );
    }

    #[test]
    fn graphql_queries_encode_like_quote_plus() {
        let url = graphql_url("{ a: [\"b~c\"] }").unwrap();
        assert_eq!(
            url.as_str(),
            "https://api.svt.se/contento/graphql?query=%7B+a%3A+%5B%22b~c%22%5D+%7D"
        );
    }
}
