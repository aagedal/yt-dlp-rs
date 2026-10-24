//! Upstream `ArteTVIE`, `ArteTVEmbedIE`, `ArteTVPlaylistIE` and
//! `ArteTVCategoryIE` (`yt_dlp/extractor/arte.py`): the player config API's
//! streams (HLS master playlists and HTTPS/RTMP URLs with upstream's version
//! code `language_preference`, accessible/forced subtitle locales and
//! duplicate removal), metadata, thumbnails and chapters; embeds followed to
//! their config URL; collections as playlists of shows or season items; and
//! category pages as playlists of their linked videos and collections.
//!
//! Not ported: upstream's fake `X-Forwarded-For` retry for geo-restricted
//! videos (this port sends no such header, so the restriction is reported
//! without the retry warning) and generic-page embed detection.
use super::clyp::key_error;
use super::{Extractor, dict_format, dict_info, download_json_headers};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct ArteTv;
pub struct ArteTvEmbed;
pub struct ArteTvPlaylist;
pub struct ArteTvCategory;

const NAME: &str = "ArteTV";
const EMBED: &str = "ArteTVEmbed";
const PLAYLIST: &str = "ArteTVPlaylist";
const CATEGORY: &str = "ArteTVCategory";

const API_BASE: &str = "https://api.arte.tv/api/player/v2";
const API_TOKEN: &str =
    "Nzc1Yjc1ZjJkYjk1NWFhN2I2MWEwMmRlMzAzNjI5NmU3NWU3ODg4ODJjOWMxNTMxYzEzZGRjYjg2ZGE4MmIwOA";
const VALIDATED_AGE: (&str, &str) = ("x-validated-age", "18");

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:https?://(?:(?:www\.)?arte\.tv/(?P<lang>fr|de|en|es|it|pl)/videos|api\.arte\.tv/api/player/v\d+/config/(?P<lang_2>fr|de|en|es|it|pl))|arte://program)/(?P<id>\d{6}-\d{3}-[AF]|LIVE)",
    )
    .unwrap()
});
static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?arte\.tv/player/v\d+/index\.php\?.*?\bjson_url=.+").unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?arte\.tv/(?P<lang>fr|de|en|es|it|pl)/videos/(?P<id>RC-\d{6})")
        .unwrap()
});
static CATEGORY_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?arte\.tv/(?P<lang>fr|de|en|es|it|pl)/videos/(?P<id>[\w-]+(?:/[\w-]+)*)/?\s*$",
    )
    .unwrap()
});
/// `_VERSION_CODE_RE`, matched at the start like `re.match`.
static VERSION_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^V(?P<original_voice>O?)(?P<vlang>[FA]|E\[[A-Z]+\]|EU)?(?P<audio_desc>AUD|)(?:(?P<has_sub>-ST)(?P<sdh_sub>M?)(?P<sub_lang>[FA]|E\[[A-Z]+\]|EU))?",
    )
    .unwrap()
});

/// `ArteTVIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `ArteTVEmbedIE.suitable`.
pub fn embed_suitable(url: &str) -> bool {
    EMBED_URL.is_match(url)
}

/// `ArteTVPlaylistIE.suitable`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `ArteTVCategoryIE.suitable`: not a video or collection URL.
pub fn category_suitable(url: &str) -> bool {
    !suitable(url) && !playlist_suitable(url) && CATEGORY_URL.is_match(url)
}

/// `_LANG_MAP` (ISO 639 to Arte's French abbreviations).
fn language_code(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "fr" => "F",
        "de" => "A",
        "en" => "E[ANG]",
        "es" => "E[ESP]",
        "it" => "E[ITA]",
        "pl" => "E[POL]",
        "mul" => "EU",
        _ => return None,
    })
}

fn start(extractor: &'static str, url: &Url) {
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(text: &str) -> Error {
    Error::UnsupportedHtml5(format!("ArteTV {text} are not supported natively"))
}

/// An `ExtractorError(..., expected=True)`.
/// `ArteTVIE._COUNTRIES_MAP`, "all obtained by exhaustive testing".
const DE_FR: [&str; 13] = [
    "BL", "DE", "FR", "GF", "GP", "MF", "MQ", "NC", "PF", "PM", "RE", "WF", "YT",
];
const EUR_DE_FR: [&str; 17] = [
    "AT", "BL", "CH", "DE", "FR", "GF", "GP", "LI", "MC", "MF", "MQ", "NC", "PF", "PM", "RE", "WF",
    "YT",
];
const SAT: [&str; 47] = [
    "AD", "AT", "AX", "BG", "BL", "CH", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GB", "GF",
    "GR", "HR", "HU", "IE", "IS", "IT", "KN", "LI", "LT", "LU", "LV", "MC", "MF", "MQ", "MT", "NC",
    "NL", "NO", "PF", "PL", "PM", "PT", "RE", "RO", "SE", "SI", "SK", "SM", "VA", "WF", "YT",
];

fn expected(extractor: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{extractor}] {id}: {message}"))
}

/// The `language_preference` of a stream's version code: six preference
/// bits joined as a decimal `int` (upstream's `int(''.join(...))`).
fn language_preference(code: &str, language_code: Option<&str>) -> i64 {
    let Some(found) = VERSION_CODE.captures(code) else {
        return -1;
    };
    let group = |name: &str| found.name(name).map(|value| value.as_str());
    let truthy = |name: &str| group(name).is_some_and(|value| !value.is_empty());
    let bits = [
        // Voice in the requested language,
        group("vlang") == language_code,
        // not the audio description version,
        !truthy("audio_desc"),
        // else at least the original voice,
        truthy("original_voice"),
        // subtitles in the requested language,
        group("sub_lang") == language_code,
        // else no subtitles,
        !truthy("has_sub"),
        // and not the hard-of-hearing ones.
        !truthy("sdh_sub"),
    ];
    bits.iter()
        .fold(0, |value, bit| value * 10 + i64::from(*bit))
}

/// `_fix_accessible_subs_locale`: `-MAL.m3u8` tracks are `LANG-acc`, tracks
/// without `_VO` are `LANG-forced`.
fn fix_accessible_subs_locale(subtitles: crate::Subtitles) -> crate::Subtitles {
    let mut updated = crate::Subtitles::default();
    for (language, tracks) in subtitles.entries {
        for track in tracks {
            let url = track.url.as_str();
            let suffix = if url.ends_with("-MAL.m3u8") {
                Some("acc")
            } else if !url.contains("_VO") {
                Some("forced")
            } else {
                None
            };
            let key = match (&language, suffix) {
                (Some(language), Some(suffix)) if !language.is_empty() => {
                    Some(format!("{language}-{suffix}"))
                }
                (_, Some(suffix)) => Some(suffix.to_owned()),
                (language, None) => language.clone(),
            };
            match updated.entries.iter_mut().find(|(lang, _)| *lang == key) {
                Some((_, existing)) => existing.push(track),
                None => updated.entries.push((key, vec![track])),
            }
        }
    }
    updated
}

/// `_merge_subtitles(subs, target=target)`: tracks with a URL already in the
/// language are dropped.
fn merge_subtitles(target: &mut crate::Subtitles, source: crate::Subtitles) {
    for (language, tracks) in source.entries {
        let index = match target.entries.iter().position(|(key, _)| *key == language) {
            Some(index) => index,
            None => {
                target.entries.push((language, Vec::new()));
                target.entries.len() - 1
            }
        };
        let existing = &mut target.entries[index].1;
        let seen: Vec<Url> = existing.iter().map(|track| track.url.clone()).collect();
        existing.extend(
            tracks
                .into_iter()
                .filter(|track| !seen.contains(&track.url)),
        );
    }
}

/// `_remove_duplicate_formats` for formats without fragments: the first
/// format of each URL.
fn remove_duplicate_formats(formats: &mut Vec<crate::VideoFormat>) {
    let mut seen: Vec<Url> = Vec::new();
    formats.retain(|format| {
        if seen.contains(&format.url) {
            return false;
        }
        seen.push(format.url.clone());
        true
    });
}

/// `traverse_obj(value, (KEY, ...))` through objects only.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .try_fold(value, |value, key| value.as_object()?.get(*key))
        .filter(|value| !value.is_null())
}

/// `obj[KEY]` (an upstream `KeyError` without it).
fn index<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    match value {
        Value::Object(map) => map.get(key).ok_or_else(|| key_error(id, key)),
        _ => Err(unsupported(&format!(
            "{key} containers that are not objects"
        ))),
    }
}

/// `f'{value}'` of a JSON string or `None`.
fn py_format(value: Option<&Value>, default: &str, what: &str) -> Result<String> {
    match value {
        None => Ok(default.to_owned()),
        Some(Value::Null) => Ok("None".to_owned()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(unsupported(&format!("{what} values that are not strings"))),
    }
}

/// The formats (secondary `cc`/`OGsub` ones apart) and merged subtitles of the config streams.
struct Streams {
    formats: Vec<crate::VideoFormat>,
    secondary: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
}

fn stream_formats(
    client: &Client,
    stream: &Value,
    video_id: &str,
    language_code: Option<&str>,
    out: &mut Streams,
) -> Result<()> {
    // The official player reads `e.get("versions")[0].eStat.ml5`.
    let versions = index(stream, "versions", video_id)?;
    let version = match versions {
        Value::Array(versions) => versions.first().ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: An extractor error has occurred. (caused by IndexError('list index out of range')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?,
        _ => return Err(unsupported("stream versions that are not lists")),
    };
    let code = index(index(version, "eStat", video_id)?, "ml5", video_id)?
        .as_str()
        .ok_or_else(|| unsupported("version codes that are not strings"))?
        .to_owned();
    let lang_pref = language_preference(&code, language_code);
    let short_label = version
        .get("shortLabel")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let note = format!(
        "{} [{short_label}]",
        py_format(version.get("label"), "unknown", "version labels")?
    );
    let protocol = index(stream, "protocol", video_id)?
        .as_str()
        .ok_or_else(|| unsupported("stream protocols that are not strings"))?;
    if protocol.contains("HLS") {
        let manifest = index(stream, "url", video_id)?
            .as_str()
            .ok_or_else(|| unsupported("stream URLs that are not strings"))?;
        let manifest = Url::parse(manifest)
            .map_err(|_| unsupported(&format!("relative stream URLs ({manifest})")))?;
        let Some(stream) = super::extract_m3u8_master_optional(
            client,
            &manifest,
            NAME,
            video_id,
            Some(&code),
            true,
            false,
        ) else {
            return Ok(());
        };
        let mut formats = stream.formats;
        for format in &mut formats {
            format.format_note = Some(note.clone());
            format
                .extra
                .insert("language_preference".into(), lang_pref.into());
            crate::info_order::mark_format_update(format, "format_note");
            crate::info_order::mark_format_update(format, "language_preference");
        }
        if short_label.starts_with("cc") || short_label.starts_with("OGsub") {
            out.secondary.extend(formats);
        } else {
            out.formats.extend(formats);
        }
        merge_subtitles(
            &mut out.subtitles,
            fix_accessible_subs_locale(stream.subtitles),
        );
    } else if matches!(protocol, "HTTPS" | "RTMP") {
        let url = index(stream, "url", video_id)?.clone();
        let format = dict_format(
            vec![
                ("format_id", format!("{protocol}-{code}").into()),
                ("url", url),
                ("format_note", note.into()),
                ("language_preference", lang_pref.into()),
            ],
            NAME,
            video_id,
        )?;
        out.formats.push(format);
    } else {
        crate::screen::warning(&format!(
            "[{NAME}] Skipping stream with unknown protocol {protocol}"
        ));
    }
    Ok(())
}

impl Extractor for ArteTv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let lang = found
            .name("lang")
            .or_else(|| found.name("lang_2"))
            .map(|lang| lang.as_str().to_owned());
        let language_code = lang.as_deref().and_then(language_code);
        start(NAME, url);
        // An `arte://program` URL has no language (`None` upstream).
        let api = Url::parse(&format!(
            "{API_BASE}/config/{}/{video_id}",
            lang.as_deref().unwrap_or("None")
        ))?;
        let config = download_json_headers(client, &api, NAME, &video_id, &[VALIDATED_AGE])?;

        let geoblocking = path(
            &config,
            &["data", "attributes", "restriction", "geoblocking"],
        );
        if let Some(geoblocking) = geoblocking.filter(|value| super::lnk::truthy(Some(value))) {
            let Some(geoblocking) = geoblocking.as_object() else {
                return Err(unsupported("geoblocking values that are not objects"));
            };
            if super::lnk::truthy(geoblocking.get("restrictedArea")) {
                let code = geoblocking
                    .get("code")
                    .ok_or_else(|| key_error(&video_id, "code"))?
                    .as_str()
                    .ok_or_else(|| unsupported("geoblocking codes that are not strings"))?;
                // `GeoRestrictedError(..., countries=_COUNTRIES_MAP.get(code,
                // ('DE', 'FR')))`.
                let countries: &[&str] = match code {
                    "DE_FR" => &DE_FR,
                    "EUR_DE_FR" => &EUR_DE_FR,
                    "SAT" => &SAT,
                    _ => &["DE", "FR"],
                };
                return Err(crate::geo::restricted(
                    &format!(
                        "[{NAME}] {video_id}: Video restricted to {}",
                        crate::batch::py_repr(code)
                    ),
                    countries,
                ));
            }
        }
        if !super::lnk::truthy(path(&config, &["data", "attributes", "rights"])) {
            return Err(expected(
                NAME,
                &video_id,
                "Video is not available in this language edition of Arte or broadcast rights expired",
            ));
        }

        let attributes = &config["data"]["attributes"];
        let streams = match index(attributes, "streams", &video_id)? {
            Value::Array(streams) => streams,
            _ => return Err(unsupported("stream lists that are not lists")),
        };
        let mut out = Streams {
            formats: Vec::new(),
            secondary: Vec::new(),
            subtitles: crate::Subtitles::default(),
        };
        for stream in streams {
            stream_formats(client, stream, &video_id, language_code, &mut out)?;
        }
        let mut formats = out.formats;
        formats.extend(out.secondary);
        remove_duplicate_formats(&mut formats);

        let metadata = index(attributes, "metadata", &video_id)?;
        let id = index(metadata, "providerId", &video_id)?
            .as_str()
            .ok_or_else(|| unsupported("provider IDs that are not strings"))?
            .to_owned();
        let get = |key: &str| metadata.get(key).cloned().unwrap_or(Value::Null);
        let webpage_url = path(metadata, &["link", "url"])
            .and_then(Value::as_str)
            .ok_or_else(|| unsupported("videos without a link URL"))?;
        let webpage_url = Url::parse(webpage_url)
            .map_err(|_| unsupported(&format!("relative link URLs ({webpage_url})")))?;
        let title = path(metadata, &["subtitle"])
            .or_else(|| path(metadata, &["title"]))
            .cloned()
            .unwrap_or(Value::Null);
        let alt_title = if super::lnk::truthy(metadata.get("subtitle")) {
            get("title")
        } else {
            get("subtitle")
        };
        let timestamp = path(attributes, &["rights", "begin"])
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);
        let images = match metadata.get("images") {
            Some(Value::Array(images)) => images.as_slice(),
            Some(images) if super::lnk::truthy(Some(images)) => {
                return Err(unsupported("image lists that are not lists"));
            }
            _ => &[],
        };
        let mut thumbnails = Vec::new();
        for image in images {
            let Some(image) = image.as_object() else {
                return Err(unsupported("images that are not objects"));
            };
            let Some(url) = image.get("url") else {
                continue;
            };
            if url.as_str().and_then(super::json_ld::url_or_none).is_none() {
                continue;
            }
            thumbnails.push(json!({
                "url": url,
                "id": image.get("caption").cloned().unwrap_or(Value::Null),
            }));
        }
        // `('chapters', 'elements', ..., {'start_time': 'startTime', 'title':
        // 'title'})`: `None` keys and empty dicts are dropped.
        let elements: Vec<&Value> = match path(attributes, &["chapters", "elements"]) {
            Some(Value::Array(items)) => items.iter().collect(),
            Some(Value::Object(items)) => items.values().collect(),
            _ => Vec::new(),
        };
        let chapters: Vec<Value> = elements
            .into_iter()
            .filter_map(Value::as_object)
            .filter_map(|element| {
                let mut chapter = Map::new();
                for (key, source) in [("start_time", "startTime"), ("title", "title")] {
                    if let Some(value) = element.get(source).filter(|value| !value.is_null()) {
                        chapter.insert(key.into(), value.clone());
                    }
                }
                (!chapter.is_empty()).then_some(Value::Object(chapter))
            })
            .collect();
        let mut fields = vec![
            ("id", Value::from(id.clone())),
            ("webpage_url", webpage_url.as_str().into()),
            ("title", title),
            ("alt_title", alt_title),
            ("description", get("description")),
            (
                "duration",
                path(metadata, &["duration", "seconds"])
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            ("language", get("language")),
            ("timestamp", timestamp),
            (
                "is_live",
                attributes
                    .get("live")
                    .cloned()
                    .unwrap_or(Value::Bool(false)),
            ),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("thumbnails", Value::Array(thumbnails)),
            (
                "chapters",
                if chapters.is_empty() {
                    Value::Null
                } else {
                    Value::Array(chapters)
                },
            ),
        ];
        // Processing drops a `None` duration.
        fields.retain(|(key, value)| *key != "duration" || !value.is_null());
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.webpage_url = webpage_url;
        info.subtitles = out.subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";chapter=start_time,title;thumbnail=url,id");
        }
        Ok(Some(info))
    }
}

/// A followed `url_result(url, ...)`.
fn follow(page: &Url, target: &Url) -> ExtractResult {
    let mut follow = super::direct::build_at(page.clone(), page, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow
        .extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    ExtractResult::Video(follow)
}

impl Extractor for ArteTvEmbed {
    fn key(&self) -> &'static str {
        EMBED
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        if !EMBED_URL.is_match(page.as_str()) {
            return Ok(None);
        }
        start(EMBED, page);
        // `parse_qs(url)['json_url'][0]` (blank values dropped).
        let Some(json_url) = page
            .query_pairs()
            .find(|(key, value)| key == "json_url" && !value.is_empty())
            .map(|(_, value)| value.into_owned())
        else {
            // Upstream's `KeyError('json_url')`.
            return Err(unsupported("embeds without a json_url value"));
        };
        // `ArteTVIE._match_id(json_url)`.
        if !suitable(&json_url) {
            return Err(unsupported(&format!(
                "embed config URLs that ArteTVIE does not match ({json_url})"
            )));
        }
        let target = Url::parse(&json_url)?;
        Ok(Some(follow(page, &target)))
    }
}

/// A playlist entry: upstream's `url_result` dict (`raw`), with the fields a
/// `url_transparent` entry overlays on the extracted video.
fn entry(raw: Map<String, Value>) -> Result<VideoInfo> {
    let target = raw
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let transparent = raw.get("_type").and_then(Value::as_str) == Some("url_transparent");
    let url = Url::parse(&target)
        .map_err(|_| unsupported(&format!("relative playlist entry URLs ({target})")))?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = raw
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    info.title = raw
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let fields: Map<String, Value> = if transparent {
        raw.iter()
            .filter(|(key, _)| !matches!(key.as_str(), "_type" | "url" | "ie_key" | "id"))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    } else {
        Map::new()
    };
    let ie_key = raw.get("ie_key").cloned().unwrap_or(Value::Null);
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": fields,
            "ie_key": ie_key,
            "extractor": ie_key,
            "entry": Value::Object(raw),
        }),
    );
    Ok(info)
}

/// `playlist_result(entries, playlist_id, ...)` with its key order.
fn playlist(
    page: &Url,
    extractor: &'static str,
    keys: &[&str],
    fields: Map<String, Value>,
    entries: Vec<VideoInfo>,
    generator: bool,
) -> ExtractResult {
    let mut extra = fields;
    let id = extra
        .shift_remove("id")
        .and_then(|id| id.as_str().map(str::to_owned));
    let title = extra
        .shift_remove("title")
        .and_then(|title| title.as_str().map(str::to_owned));
    if title.is_none() {
        extra.insert("title".into(), Value::Null);
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
        id: id.unwrap_or_default(),
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

impl Extractor for ArteTvPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let lang = found["lang"].to_owned();
        let playlist_id = found["id"].to_owned();
        start(PLAYLIST, page);
        let api = Url::parse(&format!(
            "https://api.arte.tv/api/opa/v3/programs/{lang}/{playlist_id}"
        ))?;
        let authorization = format!("Bearer {API_TOKEN}");
        let programs = download_json_headers(
            client,
            &api,
            PLAYLIST,
            &playlist_id,
            &[("Authorization", &authorization)],
        )?;
        // `('programs', ..., {dict}, any)`.
        let empty = Map::new();
        let info = match programs.get("programs") {
            Some(Value::Array(items)) => items.iter().find_map(Value::as_object),
            Some(Value::Object(items)) => items.values().find_map(Value::as_object),
            _ => None,
        }
        .unwrap_or(&empty);
        let mut fields = Map::new();
        let mut keys = Vec::new();
        for (key, source) in [("title", "title"), ("description", "shortDescription")] {
            if let Some(value) = info.get(source).filter(|value| value.is_string()) {
                fields.insert(key.into(), value.clone());
                keys.push(key);
            }
        }
        fields.insert("id".into(), playlist_id.clone().into());
        keys.push("id");

        // Seasons first.
        let children: &[Value] = info
            .get("children")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice);
        let mut season_ids = Vec::new();
        for child in children {
            if child.get("catalogType").and_then(Value::as_str) != Some("SEASON") {
                continue;
            }
            match child.get("programId") {
                None | Some(Value::Null) => {}
                Some(Value::String(id)) => season_ids.push(id.clone()),
                Some(_) => return Err(unsupported("season IDs that are not strings")),
            }
        }
        if !season_ids.is_empty() {
            let mut entries = Vec::new();
            for season_id in season_ids {
                let api = Url::parse(&format!("{API_BASE}/playlist/{lang}/{season_id}"))?;
                let season =
                    download_json_headers(client, &api, PLAYLIST, &season_id, &[VALIDATED_AGE])?;
                let items = path(&season, &["data", "attributes", "items"]);
                let items: Vec<&Value> = match items {
                    Some(Value::Array(items)) => items.iter().collect(),
                    Some(Value::Object(items)) => items.values().collect(),
                    _ => Vec::new(),
                };
                for video in items {
                    // `lambda _, v: v['providerId'] and v['link']['url']`.
                    let link = path(video, &["link", "url"]);
                    if !super::lnk::truthy(video.get("providerId")) || !super::lnk::truthy(link) {
                        continue;
                    }
                    let Some(link) = link.filter(|link| link.is_string()) else {
                        return Err(unsupported("season item links that are not strings"));
                    };
                    let mut raw = Map::new();
                    raw.insert("_type".into(), "url_transparent".into());
                    raw.insert("ie_key".into(), NAME.into());
                    raw.insert("url".into(), link.clone());
                    for (key, value) in [
                        (
                            "id",
                            video.get("providerId").filter(|v| v.is_string()).cloned(),
                        ),
                        (
                            "title",
                            video.get("title").filter(|v| v.is_string()).cloned(),
                        ),
                        (
                            "alt_title",
                            video.get("subtitle").filter(|v| v.is_string()).cloned(),
                        ),
                        (
                            "duration",
                            Some(super::streamable::int_or_none(path(
                                video,
                                &["duration", "seconds"],
                            ))),
                        ),
                        (
                            "age_limit",
                            Some(super::streamable::int_or_none(video.get("ageRating"))),
                        ),
                    ] {
                        if let Some(value) = value.filter(|value| !value.is_null()) {
                            raw.insert(key.into(), value);
                        }
                    }
                    entries.push(entry(raw)?);
                }
            }
            return Ok(Some(playlist(page, PLAYLIST, &keys, fields, entries, true)));
        }

        // Else a mini series of shows.
        let videos: &[Value] = info
            .get("videos")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice);
        let mut entries = Vec::new();
        for video in videos {
            if video.get("kind").and_then(Value::as_str) != Some("SHOW") {
                continue;
            }
            let Some(url) = video
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| super::json_ld::url_or_none(url).is_some())
            else {
                continue;
            };
            let mut raw = Map::new();
            raw.insert("ie_key".into(), NAME.into());
            raw.insert("_type".into(), "url".into());
            raw.insert("url".into(), url.into());
            entries.push(entry(raw)?);
        }
        Ok(Some(playlist(
            page, PLAYLIST, &keys, fields, entries, false,
        )))
    }
}

/// The category page's `<a href=...>` links to this language's videos;
/// upstream's `(?P<q>"|\'|\b)...(?P=q)` backreference as alternatives.
fn category_links(webpage: &str, lang: &str) -> Vec<String> {
    let url = format!(r"https?://www\.arte\.tv/{lang}/videos/[\w/-]+");
    let pattern = Regex::new(&format!(
        r#"<a\b[^>]*?href\s*=\s*(?:"(?P<double>{url})"|'(?P<single>{url})'|\b(?P<bare>{url}))"#
    ))
    .expect("static pattern");
    pattern
        .captures_iter(webpage)
        .filter_map(|found| {
            found
                .name("double")
                .or_else(|| found.name("single"))
                .or_else(|| found.name("bare"))
                .map(|url| url.as_str().to_owned())
        })
        .collect()
}

impl Extractor for ArteTvCategory {
    fn key(&self) -> &'static str {
        CATEGORY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        if !category_suitable(page.as_str()) {
            return Ok(None);
        }
        let Some(found) = CATEGORY_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let lang = found["lang"].to_owned();
        let playlist_id = found["id"].to_owned();
        start(CATEGORY, page);
        let webpage = super::download_webpage(client, page, CATEGORY, &playlist_id)?;
        let mut items: Vec<String> = Vec::new();
        for video in category_links(&webpage, &lang) {
            if video == page.as_str() {
                continue;
            }
            // `orderedSet`.
            if (suitable(&video) || playlist_suitable(&video)) && !items.contains(&video) {
                items.push(video);
            }
        }
        // `_generic_title('', webpage)`: OpenGraph title, else `<title>`.
        let generic = crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
            .or_else(|| crate::html::html_extract_title(&webpage).filter(|t| !t.is_empty()))
            .unwrap_or_default();
        let title = generic
            .rsplit_once('|')
            .map_or(generic.as_str(), |(head, _)| head)
            .trim()
            .to_owned();
        let description = crate::html::og_search_property(&webpage, "description");
        let mut fields = Map::new();
        fields.insert(
            "description".into(),
            description.map_or(Value::Null, Value::from),
        );
        fields.insert("id".into(), playlist_id.into());
        let mut keys = vec!["description", "id"];
        if !title.is_empty() {
            fields.insert("title".into(), title.into());
            keys.push("title");
        }
        let entries = items
            .into_iter()
            .map(|item| {
                let mut raw = Map::new();
                raw.insert("_type".into(), "url".into());
                raw.insert("url".into(), item.into());
                entry(raw)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(playlist(page, CATEGORY, &keys, fields, entries, true)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert!(suitable(
            "https://www.arte.tv/en/videos/088501-000-A/mexico-stealing-petrol-to-survive/"
        ));
        assert!(suitable("https://api.arte.tv/api/player/v2/config/de/LIVE"));
        assert!(suitable("arte://program/100605-013-A"));
        assert!(!suitable(
            "https://www.arte.tv/en/videos/RC-016954/earn-a-living/"
        ));
        assert!(embed_suitable(
            "https://www.arte.tv/player/v3/index.php?json_url=https://api.arte.tv/api/player/v2/config/de/100605-013-A"
        ));
        assert!(playlist_suitable(
            "https://www.arte.tv/en/videos/RC-016954/earn-a-living/"
        ));
        assert!(category_suitable(
            "https://www.arte.tv/en/videos/politics-and-society/"
        ));
        assert!(!category_suitable(
            "https://www.arte.tv/en/videos/RC-016954/earn-a-living/"
        ));
        assert!(!category_suitable(
            "https://www.arte.tv/de/videos/110203-006-A/zaz/"
        ));
    }

    #[test]
    fn version_codes_rank_like_upstream() {
        assert_eq!(language_preference("VF", Some("F")), 110011);
        assert_eq!(language_preference("VA-STF", Some("F")), 10101);
        assert_eq!(language_preference("VOF-STE[ANG]", Some("E[ANG]")), 11101);
        assert_eq!(language_preference("VFAUD", Some("F")), 100011);
        assert_eq!(language_preference("XQ", Some("F")), -1);
        // No language (`arte://program`): `None == None`.
        assert_eq!(language_preference("VO", None), 111111);
    }
}
