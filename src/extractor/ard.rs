//! Upstream `ARDBetaMediathekIE` (`ARDMediathek`), `ARDMediathekCollectionIE`,
//! `ARDAudiothekIE` and `ARDAudiothekPlaylistIE` (`yt_dlp/extractor/ard.py`):
//! ARD Mediathek videos through the page-gateway `item` JSON (the old
//! `contentId` as ID, the `player_ondemand`/`player_live` widget's streams as
//! HLS and progressive formats with stream-kind preferences and audio
//! languages, subtitle sources, jump-mark chapters, metadata, episode numbers
//! parsed from the title, the FSK login error and the optional `ams`-cookie
//! age-verification token), Mediathek shows, series seasons and compilations
//! as paged playlists of `url` results, and ARD Audiothek episodes and shows
//! through its GraphQL API.
//!
//! Not supported natively: `ARDMediathekBaseIE` (only used by other sites'
//! extractors). Collection pages are fetched eagerly (upstream's
//! `OnDemandPagedList` fetches them while processing), and values of
//! unexpected types are explicit native errors.
use super::{Extractor, dict_format, dict_info, download_json_headers, download_json_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, SubtitleFormat, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct ArdMediathek;
pub struct ArdMediathekCollection;
pub struct ArdAudiothek;
pub struct ArdAudiothekPlaylist;

const NAME: &str = "ARDMediathek";
const KEY: &str = "ARDBetaMediathek";
const COLLECTION: &str = "ARDMediathekCollection";
const AUDIO: &str = "ARDAudiothek";
const AUDIO_PLAYLIST: &str = "ARDAudiothekPlaylist";
const TOKEN_URL: &str = "https://sso.ardmediathek.de/sso/token";
const PAGE_SIZE: usize = 100;
const GRAPHQL: &str = "https://api.ardaudiothek.de/graphql";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:beta|www)\.)?ardmediathek\.de/(?:[^/]+/)?(?:player|live|video)/(?:[^?#]+/)?(?P<id>[a-zA-Z0-9]+)/?(?:[?#]|$)",
    )
    .unwrap()
});
static COLLECTION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:beta|www)\.)?ardmediathek\.de/(?:[^/?#]+/)?(?P<playlist>sendung|serie|sammlung)/(?:(?P<display_id>[^?#]+?)/)?(?P<id>[a-zA-Z0-9]+)(?:/(?P<season>\d+)(?:/(?P<version>OV|AD))?)?/?(?:[?#]|$)",
    )
    .unwrap()
});
static AUDIO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https:?//(?:www\.)?ard(?:audiothek|sounds)\.de/episode/(?P<id>urn:ard:(?:episode|section|extra):[a-f0-9]{16})",
    )
    .unwrap()
});
static AUDIO_PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https:?//(?:www\.)?ard(?:audiothek|sounds)\.de/sendung/(?P<playlist>[\w-]+)/(?P<id>urn:ard:show:[a-f0-9]{16})",
    )
    .unwrap()
});
/// `ARDBetaMediathekIE._extract_episode_info`'s patterns, tried in order.
static EPISODE_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"^.*(?P<ep_info> \(S(?P<season_number>\d+)/E(?P<episode_number>\d+)\)).*",
        r"^.*(?P<ep_info> \((?:Folge |Teil )?(?P<episode_number>\d+)(?:/\d+)?\)).*",
        r#"^.*(?P<ep_info>Folge (?P<episode_number>\d+)(?:\:| -|) )"(?P<episode>.+)".*"#,
        r"^.*(?P<ep_info>Folge (?P<episode_number>\d+)(?:/\d+)?(?:\:| -|) ).*",
        r"^(?P<title>.*)",
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});

const QUERY_ITEM: &str = "    query($id: ID!) {\n        item(id: $id) {\n            audioList {\n                href\n                distributionType\n                audioBitrate\n                audioCodec\n            }\n            show {\n              title\n            }\n            image {\n              url1X1\n            }\n            programSet {\n              publicationService {\n                organizationName\n              }\n            }\n            description\n            title\n            duration\n            startDate\n            episodeNumber\n        }\n    }";
const QUERY_PLAYLIST: &str = "\n    query($id: ID!) {\n        show(id: $id) {\n            title\n            description\n            items(filter: { isPublished: { equalTo: true } }) {\n                nodes {\n                    url\n                }\n            }\n        }\n    }";

/// `ARDBetaMediathekIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `ARDMediathekCollectionIE._match_valid_url`.
pub fn collection_suitable(url: &str) -> bool {
    COLLECTION_URL.is_match(url)
}

/// `ARDAudiothekIE._match_id`.
pub fn audiothek_suitable(url: &str) -> bool {
    AUDIO_URL.is_match(url)
}

/// `ARDAudiothekPlaylistIE._match_valid_url`.
pub fn audiothek_playlist_suitable(url: &str) -> bool {
    AUDIO_PLAYLIST_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("ARD {what} are not supported natively"))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
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

/// A `{str}` traversal: the value when it is a string.
fn str_value(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

/// `url_or_none` of a string value.
fn url_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `parse_iso8601` of a string value.
fn timestamp_value(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map_or(Value::Null, Value::from)
}

/// A text field kept untyped by upstream's traversal: strings (or `None`).
fn text_field(value: Option<&Value>, what: &str) -> Result<Option<Value>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone().into())),
        Some(_) => Err(unsupported(&format!("{what} that are not strings"))),
    }
}

/// Python's `str()` of a value in an f-string.
fn py_str(value: Option<&Value>, what: &str) -> Result<String> {
    match value {
        None | Some(Value::Null) => Ok("None".into()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Bool(flag)) => Ok(if *flag { "True" } else { "False" }.into()),
        Some(Value::Number(number)) if number.is_i64() || number.is_u64() => Ok(number.to_string()),
        Some(_) => Err(unsupported(&format!("{what} that are not strings"))),
    }
}

/// The values of a `(..., KEY, ...)` traversal step: a list's items or a
/// dict's values.
fn branch(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(map)) => map.values().collect(),
        _ => Vec::new(),
    }
}

fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().try_fold(value, |value, key| value.get(*key))
}

/// `_merge_subtitles(subs, target=subtitles)`.
fn merge_subtitles(target: &mut Subtitles, source: Subtitles) {
    super::svt::merge_subtitles(target, source);
}

fn add_subtitle(subtitles: &mut Subtitles, language: Option<String>, track: SubtitleFormat) {
    match subtitles
        .entries
        .iter_mut()
        .find(|(key, _)| *key == language)
    {
        Some((_, tracks)) => tracks.push(track),
        None => subtitles.entries.push((language, vec![track])),
    }
}

/// `ARDBetaMediathekIE._extract_episode_info(title)`: the first matching
/// pattern's season/episode numbers and episode (in that key order).
fn episode_info(title: Option<&Value>) -> Vec<(&'static str, Value)> {
    // `re.match` of a non-string raises; `traverse_obj` drops it.
    let Some(title) = title.and_then(Value::as_str) else {
        return Vec::new();
    };
    for pattern in EPISODE_PATTERNS.iter() {
        let Some(found) = pattern.captures(title) else {
            continue;
        };
        let number = |name: &str| {
            found
                .name(name)
                .map_or(Value::Null, |text| int_or_none(Some(&text.as_str().into())))
        };
        let episode = found
            .name("episode")
            .map(|episode| episode.as_str().to_owned())
            .or_else(|| {
                found
                    .name("ep_info")
                    .map(|info| title.replace(info.as_str(), ""))
            })
            .or_else(|| found.name("title").map(|title| title.as_str().to_owned()))
            .map(|episode| episode.trim().to_owned());
        let mut fields = Vec::new();
        for (key, value) in [
            ("season_number", number("season_number")),
            ("episode_number", number("episode_number")),
            ("episode", episode.map_or(Value::Null, Value::from)),
        ] {
            if !value.is_null() {
                fields.push((key, value));
            }
        }
        return fields;
    }
    Vec::new()
}

/// `jwt_decode_hs256(token)` as a dict, else `None`.
fn decode_jwt(token: &str) -> Option<Map<String, Value>> {
    match super::podbayfm::jwt_payload(token)? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// The age-verification token for an `ams` cookie: the `x-authorization`
/// header value and the `userId` query.
fn age_token(client: &Client, display_id: &str) -> Result<Option<(String, String)>> {
    if crate::cookies::get_cookie(TOKEN_URL, "ams").is_none_or(|value| value.is_empty()) {
        return Ok(None);
    }
    let token = match download_json_note(
        client,
        &Url::parse(TOKEN_URL)?,
        NAME,
        display_id,
        "Fetching token for age verification",
    ) {
        Ok(token) => Some(token),
        // `fatal=False`.
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {display_id}: Unable to fetch age verification token: {}",
                crate::upstream_error_text(&source)
            ));
            None
        }
        Err(Error::Reported(text)) => {
            crate::screen::warning(&text);
            None
        }
        Err(error) => return Err(error),
    };
    let id_token = token
        .as_ref()
        .and_then(|token| str_value(token.get("idToken")));
    let decoded = id_token.as_deref().and_then(decode_jwt);
    let user_id = decoded.as_ref().and_then(|decoded| {
        ["user_id", "sub"]
            .iter()
            .find_map(|key| str_value(decoded.get(*key)))
    });
    let (Some(id_token), Some(decoded), Some(user_id)) = (id_token, decoded, user_id) else {
        crate::screen::warning(&format!(
            "[{NAME}] Unable to extract token, continuing without authentication"
        ));
        return Ok(None);
    };
    if decoded.get("age_rating").and_then(Value::as_f64) != Some(18.0) {
        crate::screen::warning(&format!(
            "[{NAME}] Account is not verified as 18+; video may be unavailable"
        ));
    }
    Ok(Some((format!("Bearer {id_token}"), user_id)))
}

impl Extractor for ArdMediathek {
    fn key(&self) -> &'static str {
        KEY
    }

    /// `_GEO_COUNTRIES = ['DE']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["DE"])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        announce(NAME, url);
        let mut query = vec![("embedded", "false".to_owned()), ("mcV6", "true".into())];
        let mut headers = Vec::new();
        if let Some((authorization, user_id)) = age_token(client, &display_id)? {
            headers.push(("x-authorization", authorization));
            query.push(("userId", user_id));
        }
        let api = Url::parse(&format!(
            "https://api.ardmediathek.de/page-gateway/pages/ard/item/{display_id}"
        ))?;
        let pairs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let api = super::update_url_query(&api, &pairs);
        let header_pairs: Vec<(&str, &str)> =
            headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let page_data = download_json_headers(client, &api, NAME, &display_id, &header_pairs)?;

        // The old `contentId` instead of the longer crid.
        let old_id = match path(&page_data, &["tracking", "atiCustomVars", "contentId"]) {
            Some(Value::Number(number)) if number.is_i64() || number.is_u64() => {
                Some(number.to_string())
            }
            Some(Value::Bool(flag)) => Some(if *flag { "True" } else { "False" }.into()),
            _ => None,
        };
        let (video_id, archive_ids) = match old_id {
            Some(id) => (id, json!([format!("{} {display_id}", KEY.to_lowercase())])),
            None => {
                crate::screen::warning(&format!(
                    "[{NAME}] Could not extract contentId; {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
                (display_id.clone(), Value::Null)
            }
        };

        let player = branch(page_data.get("widgets")).into_iter().find(|widget| {
            widget.is_object()
                && matches!(
                    widget.get("type").and_then(Value::as_str),
                    Some("player_ondemand" | "player_live")
                )
        });
        let Some(player) = player else {
            // `player_data.get('type')` of `None`.
            return Err(Error::Reported(
                "'NoneType' object has no attribute 'get'".into(),
            ));
        };
        let is_live = player.get("type").and_then(Value::as_str) == Some("player_live");
        let empty = Value::Object(Map::new());
        let media_data = path(player, &["mediaCollection", "embedded"])
            .filter(|media| media.is_object())
            .unwrap_or(&empty);
        if player.get("blockedByFsk").is_some_and(super::nuxt::truthy) {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: {}",
                super::login::login_required_message(
                    "This video is only available for age verified users or after 22:00",
                    None,
                    None,
                )
            )));
        }

        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        for stream in branch(media_data.get("streams")) {
            if !stream.is_object() {
                continue;
            }
            let kind = stream.get("kind");
            let kind_text = py_str(kind, "stream kinds")?;
            // The main stream over sign language and others.
            let preference = if kind.and_then(Value::as_str) == Some("main") {
                Value::from(1)
            } else {
                Value::Null
            };
            for media in branch(stream.get("media")) {
                let Some(media_url) = media.get("url").and_then(Value::as_str) else {
                    continue;
                };
                if super::json_ld::url_or_none(media_url).is_none() {
                    continue;
                }
                let audio = media
                    .get("audios")
                    .and_then(Value::as_array)
                    .and_then(|audios| audios.first());
                let audio_kind = audio
                    .and_then(|audio| str_value(audio.get("kind")))
                    .unwrap_or_default()
                    .replace("standard", "");
                let lang_code = audio
                    .and_then(|audio| str_value(audio.get("languageCode")))
                    .filter(|code| !code.is_empty())
                    .unwrap_or_else(|| "deu".into());
                let lang = if audio_kind.is_empty() {
                    lang_code
                } else {
                    format!("{lang_code}-{audio_kind}")
                };
                let language_preference = if lang == "deu" { 10 } else { -10 };

                if super::direct::determine_ext(media_url).as_deref() == Some("m3u8") {
                    let manifest =
                        Url::parse(media_url).map_err(|_| unsupported("relative stream URLs"))?;
                    let Some(stream) = super::extract_m3u8_master_optional(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some(&format!("hls-{kind_text}")),
                        true,
                        is_live,
                    ) else {
                        continue;
                    };
                    for mut format in stream.formats {
                        if !preference.is_null() {
                            format.extra.insert("preference".into(), preference.clone());
                        }
                        format.language = Some(lang.clone());
                        format
                            .extra
                            .insert("language_preference".into(), language_preference.into());
                        crate::info_order::mark_format_update(&mut format, "language");
                        crate::info_order::mark_format_update(&mut format, "language_preference");
                        formats.push(format);
                    }
                    merge_subtitles(&mut subtitles, stream.subtitles);
                } else {
                    let mut fields = vec![
                        ("url", Value::from(media_url)),
                        ("format_id", format!("http-{kind_text}").into()),
                        ("preference", preference.clone()),
                        ("language", lang.clone().into()),
                        ("language_preference", language_preference.into()),
                    ];
                    for (key, value) in [
                        (
                            "format_note",
                            str_value(media.get("forcedLabel")).map_or(Value::Null, Value::from),
                        ),
                        ("width", int_or_none(media.get("maxHResolutionPx"))),
                        ("height", int_or_none(media.get("maxVResolutionPx"))),
                        (
                            "vcodec",
                            str_value(media.get("videoCodec")).map_or(Value::Null, Value::from),
                        ),
                    ] {
                        if !value.is_null() {
                            fields.push((key, value));
                        }
                    }
                    formats.push(dict_format(fields, NAME, &video_id)?);
                }
            }
        }

        for sub in branch(media_data.get("subtitles")) {
            if !sub.is_object() {
                continue;
            }
            let language = match sub.get("languageCode") {
                Some(Value::String(code)) if !code.is_empty() => code.clone(),
                None | Some(Value::Null) | Some(Value::String(_)) => "deu".into(),
                Some(_) => return Err(unsupported("subtitle languages that are not strings")),
            };
            for source in branch(sub.get("sources")) {
                let Some(source_url) = source.get("url").and_then(Value::as_str) else {
                    continue;
                };
                if super::json_ld::url_or_none(source_url).is_none() {
                    continue;
                }
                let ext = match source.get("kind").and_then(Value::as_str) {
                    Some("webvtt") => "vtt".to_owned(),
                    Some("ebutt") => "ttml".to_owned(),
                    // Processing's `determine_ext(url).lower()`.
                    _ => super::direct::determine_ext(source_url)
                        .unwrap_or_else(|| "unknown_video".into())
                        .to_lowercase(),
                };
                let parsed = Url::parse(source_url)
                    .map_err(|_| unsupported("subtitle URLs that are not absolute"))?;
                add_subtitle(
                    &mut subtitles,
                    Some(language.clone()),
                    SubtitleFormat {
                        name: None,
                        url: parsed,
                        ext,
                        protocol: None,
                        fragments: Vec::new(),
                        extra: Map::new(),
                    },
                );
            }
        }

        // `remove_start(fskRating, 'FSK')` as an int.
        let age_limit = page_data
            .get("fskRating")
            .and_then(Value::as_str)
            .map_or(Value::Null, |rating| {
                int_or_none(Some(&rating.strip_prefix("FSK").unwrap_or(rating).into()))
            });

        // `('pluginData', 'jumpmarks@all', 'chapterArray', lambda _, v:
        // int_or_none(v['chapterTime']), {...})`: truthy chapter times only.
        let mut chapters = Vec::new();
        for chapter in branch(path(
            media_data,
            &["pluginData", "jumpmarks@all", "chapterArray"],
        )) {
            let start = int_or_none(chapter.get("chapterTime"));
            if !super::nuxt::truthy(&start) {
                continue;
            }
            let mut entry = Map::new();
            entry.insert("start_time".into(), start);
            if let Some(title) = str_value(chapter.get("chapterTitle")) {
                entry.insert("title".into(), title.into());
            }
            chapters.push(Value::Object(entry));
        }

        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("display_id", display_id.clone().into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("is_live", is_live.into()),
            ("age_limit", age_limit),
        ];
        if !chapters.is_empty() {
            fields.push(("chapters", Value::Array(chapters)));
        }
        let meta = media_data.get("meta").unwrap_or(&Value::Null);
        let thumbnail = meta
            .get("images")
            .and_then(Value::as_array)
            .and_then(|images| images.first())
            .and_then(|image| url_value(image.get("url")));
        for (key, value) in [
            ("title", text_field(meta.get("title"), "titles")?),
            ("description", text_field(meta.get("synopsis"), "synopses")?),
            (
                "timestamp",
                Some(timestamp_value(meta.get("broadcastedOnDateTime"))),
            ),
            (
                "series",
                text_field(meta.get("seriesTitle"), "series titles")?,
            ),
            ("thumbnail", thumbnail.map(Value::from)),
            ("duration", Some(int_or_none(meta.get("durationSeconds")))),
            (
                "channel",
                text_field(meta.get("clipSourceName"), "clip source names")?,
            ),
        ] {
            if let Some(value) = value.filter(|value| !value.is_null()) {
                fields.push((key, value));
            }
        }
        fields.extend(episode_info(page_data.get("title")));
        fields.push(("_old_archive_ids", archive_ids));
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        info.subtitles = subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";chapter=start_time,title");
        }
        Ok(Some(info))
    }
}

/// A `url_result(target, ie, **fields)` playlist entry.
fn url_entry(
    target: &str,
    (ie_key, extractor): (&'static str, &'static str),
    fields: Vec<(&str, Value)>,
) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    for (key, value) in fields {
        if key == "id"
            && let Some(id) = value.as_str()
        {
            info.id = id.to_owned();
        }
        entry.insert(key.into(), value);
    }
    entry.insert("ie_key".into(), ie_key.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
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

/// `playlist_result(entries, id, display_id=..., **kwargs)`: the keyword
/// fields first (`None` values kept), then the ID.
fn playlist(
    page: &Url,
    extractor: &'static str,
    id: String,
    fields: Vec<(&str, Value)>,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
    keys.push("id");
    let mut extra = Map::new();
    let mut title = None;
    for (key, value) in fields {
        if key == "title"
            && let Some(text) = value.as_str()
        {
            title = Some(text.to_owned());
            continue;
        }
        extra.insert(key.into(), value);
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
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

impl Extractor for ArdMediathekCollection {
    fn key(&self) -> &'static str {
        COLLECTION
    }

    /// `_GEO_COUNTRIES = ['DE']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["DE"])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = COLLECTION_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        let display_id = found.name("display_id").map(|id| id.as_str().to_owned());
        let playlist_type = found["playlist"].to_owned();
        let season = found
            .name("season")
            .map(|season| season.as_str().to_owned());
        let version = found
            .name("version")
            .map(|version| version.as_str().to_owned());
        announce(COLLECTION, url);

        let api_path = if playlist_type == "sammlung" {
            "compilations/ard"
        } else {
            "widgets/ard/asset"
        };
        let call_api = |page: usize| -> Result<Value> {
            let base = Url::parse(&format!(
                "https://api.ardmediathek.de/page-gateway/{api_path}/{playlist_id}"
            ))?;
            let page_text = page.to_string();
            let size = PAGE_SIZE.to_string();
            let mut query = vec![
                ("pageNumber", page_text.as_str()),
                ("pageSize", size.as_str()),
            ];
            if let Some(season) = &season {
                query.extend([
                    ("seasoned", "true"),
                    ("seasonNumber", season.as_str()),
                    (
                        "withOriginalversion",
                        if version.as_deref() == Some("OV") {
                            "true"
                        } else {
                            "false"
                        },
                    ),
                    (
                        "withAudiodescription",
                        if version.as_deref() == Some("AD") {
                            "true"
                        } else {
                            "false"
                        },
                    ),
                ]);
            }
            let api = super::update_url_query(&base, &query);
            download_json_note(
                client,
                &api,
                COLLECTION,
                &playlist_id,
                &format!("Downloading playlist page {page}"),
            )
        };

        let page_data = call_api(0)?;
        let mut full_id = playlist_id.clone();
        for part in [&season, &version].into_iter().flatten() {
            full_id.push('_');
            full_id.push_str(part);
        }

        // `OnDemandPagedList(fetch_page, 100)`: pages until one yields fewer
        // entries than the page size.
        let mut entries = Vec::new();
        for page in 0.. {
            let data = call_api(page)?;
            let mut yielded = 0;
            for item in branch(data.get("teasers")) {
                if !item.is_object() {
                    continue;
                }
                let target = item.get("links").and_then(|links| links.get("target"));
                let item_id = target
                    .and_then(|target| {
                        ["urlId", "id"]
                            .iter()
                            .filter_map(|key| target.get(*key))
                            .find(|value| !value.is_null())
                    })
                    .or_else(|| item.get("id").filter(|value| !value.is_null()));
                let Some(item_id) = item_id else {
                    continue;
                };
                if !super::nuxt::truthy(item_id) || item_id.as_str() == Some(playlist_id.as_str()) {
                    continue;
                }
                let item_id = py_str(Some(item_id), "teaser IDs")?;
                let compilation = item.get("type").and_then(Value::as_str) == Some("compilation");
                let (mode, ie) = if compilation {
                    ("sammlung", (COLLECTION, COLLECTION))
                } else {
                    ("video", (KEY, NAME))
                };
                let mut fields = Vec::new();
                for (key, value) in [
                    (
                        "id",
                        str_value(item.get("id")).map_or(Value::Null, Value::from),
                    ),
                    (
                        "title",
                        str_value(item.get("longTitle")).map_or(Value::Null, Value::from),
                    ),
                    ("duration", int_or_none(item.get("duration"))),
                    ("timestamp", timestamp_value(item.get("broadcastedOn"))),
                ] {
                    if !value.is_null() {
                        fields.push((key, value));
                    }
                }
                entries.push(url_entry(
                    &format!("https://www.ardmediathek.de/{mode}/{item_id}"),
                    ie,
                    fields,
                )?);
                yielded += 1;
            }
            if yielded < PAGE_SIZE {
                break;
            }
        }

        let title = text_field(page_data.get("title"), "playlist titles")?.unwrap_or(Value::Null);
        let description =
            text_field(page_data.get("synopsis"), "playlist synopses")?.unwrap_or(Value::Null);
        Ok(Some(playlist(
            url,
            COLLECTION,
            full_id,
            vec![
                ("display_id", display_id.map_or(Value::Null, Value::from)),
                ("title", title),
                ("description", description),
            ],
            entries,
        )))
    }
}

/// `ARDAudiothekBaseIE._graphql_query(urn, query)`: the POSTed query's
/// `data`.
fn graphql_query(client: &Client, name: &'static str, urn: &str, query: &str) -> Result<Value> {
    use std::io::Read;
    let body = format!(
        "{{\"query\": {}, \"variables\": {{\"id\": {}}}}}",
        serde_json::to_string(query).expect("string"),
        serde_json::to_string(urn).expect("string")
    );
    crate::screen::screen(&format!("[{name}] {urn}: Downloading JSON metadata"));
    let url = Url::parse(GRAPHQL)?;
    let mut response =
        super::direct::post(client, &url, &[("Content-Type", "application/json")], body).map_err(
            |error| match error {
                Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                    extractor: name,
                    id: urn.to_owned(),
                    what: "JSON metadata",
                    source: Box::new(error),
                },
                error => error,
            },
        )?;
    let page_url = response.url().clone();
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    let bytes = super::decode_content(response.headers(), bytes).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(urn, None, None);
    crate::screen::page_read(&page_url, &bytes)?;
    let text = super::decode_webpage(&bytes, response.headers());
    let data: Value = crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{name}] {urn}: Failed to parse JSON  (caused by {error})"
        ))
    })?;
    match data {
        Value::Object(mut map) => map
            .shift_remove("data")
            .ok_or_else(|| key_error(urn, "data")),
        _ => Err(unsupported("GraphQL responses that are not objects")),
    }
}

/// `data[KEY]` of a GraphQL response's data.
fn graphql_field(data: Value, urn: &str, key: &str) -> Result<Value> {
    match data {
        Value::Object(mut map) => map.shift_remove(key).ok_or_else(|| key_error(urn, key)),
        Value::Null => Err(Error::Reported(
            "'NoneType' object is not subscriptable".into(),
        )),
        _ => Err(unsupported("GraphQL data that are not objects")),
    }
}

/// `update_url(url, query=None)`: the URL without its query.
fn without_query(url: &str) -> String {
    let Some((base, rest)) = url.split_once('?') else {
        return url.to_owned();
    };
    match rest.split_once('#') {
        Some((_, fragment)) => format!("{base}#{fragment}"),
        None => base.to_owned(),
    }
}

impl Extractor for ArdAudiothek {
    fn key(&self) -> &'static str {
        AUDIO
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = AUDIO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let urn = found["id"].to_owned();
        announce(AUDIO, url);
        let data = graphql_query(client, AUDIO, &urn, QUERY_ITEM)?;
        let item = graphql_field(data, &urn, "item")?;
        let empty = Value::Object(Map::new());
        let item = if item.is_object() { &item } else { &empty };

        let mut formats = Vec::new();
        for audio in branch(item.get("audioList")) {
            let Some(href) = audio.get("href").and_then(Value::as_str) else {
                continue;
            };
            if super::json_ld::url_or_none(href).is_none() {
                continue;
            }
            let mut fields = vec![("url", Value::from(href))];
            for (key, value) in [
                (
                    "format_id",
                    str_value(audio.get("distributionType")).map_or(Value::Null, Value::from),
                ),
                ("abr", int_or_none(audio.get("audioBitrate"))),
                (
                    "acodec",
                    str_value(audio.get("audioCodec")).map_or(Value::Null, Value::from),
                ),
            ] {
                if !value.is_null() {
                    fields.push((key, value));
                }
            }
            fields.push(("vcodec", "none".into()));
            let mut format = dict_format(fields, AUDIO, &urn)?;
            if format.abr.is_some() {
                crate::info_order::mark_int_rates(&mut format);
            }
            formats.push(format);
        }

        let mut fields = vec![("id", Value::from(urn.clone()))];
        if !formats.is_empty() {
            fields.push(("formats", Value::Null));
        }
        let thumbnail = path(item, &["image", "url1X1"])
            .and_then(|url| url_value(Some(url)))
            .map(|url| without_query(&url));
        for (key, value) in [
            (
                "channel",
                str_value(path(
                    item,
                    &["programSet", "publicationService", "organizationName"],
                ))
                .map_or(Value::Null, Value::from),
            ),
            (
                "description",
                str_value(item.get("description")).map_or(Value::Null, Value::from),
            ),
            ("duration", int_or_none(item.get("duration"))),
            (
                "series",
                str_value(path(item, &["show", "title"])).map_or(Value::Null, Value::from),
            ),
            ("episode_number", int_or_none(item.get("episodeNumber"))),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            ("timestamp", timestamp_value(item.get("startDate"))),
            (
                "title",
                str_value(item.get("title")).map_or(Value::Null, Value::from),
            ),
        ] {
            if !value.is_null() {
                fields.push((key, value));
            }
        }
        Ok(Some(dict_info(url, (AUDIO, AUDIO), fields, formats)))
    }
}

impl Extractor for ArdAudiothekPlaylist {
    fn key(&self) -> &'static str {
        AUDIO_PLAYLIST
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = AUDIO_PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let urn = found["id"].to_owned();
        let display_id = found["playlist"].to_owned();
        announce(AUDIO_PLAYLIST, url);
        let data = graphql_query(client, AUDIO_PLAYLIST, &urn, QUERY_PLAYLIST)?;
        let show = graphql_field(data, &urn, "show")?;
        let mut entries = Vec::new();
        for node in branch(path(&show, &["items", "nodes"])) {
            if let Some(target) = url_value(node.get("url")) {
                entries.push(url_entry(&target, (AUDIO, AUDIO), Vec::new())?);
            }
        }
        let mut fields = vec![("display_id", Value::from(display_id))];
        for (key, value) in [
            ("title", str_value(show.get("title"))),
            ("description", str_value(show.get("description"))),
        ] {
            if let Some(value) = value {
                fields.push((key, value.into()));
            }
        }
        Ok(Some(playlist(url, AUDIO_PLAYLIST, urn, fields, entries)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "https://www.ardmediathek.de/video/filme-im-mdr/liebe-auf-vier-pfoten/mdr-fernsehen/Y3JpZDovL21kci5kZS9zZW5kdW5nLzI4MjA0MC80MjIwOTEtNDAyNTM0",
            "https://beta.ardmediathek.de/ard/video/Y3JpZDovL2Rhc2Vyc3RlLmRlL3RhdG9ydC9mYmM4NGM1NC0xNzU4LTRmZGYtYWFhZS0wYzcyZTIxNGEyMDE",
            "https://ardmediathek.de/ard/video/saartalk/saartalk-gesellschaftsgift-haltung-gegen-hass/sr-fernsehen/Y3JpZDovL3NyLW9ubGluZS5kZS9TVF84MTY4MA/",
            "https://www.ardmediathek.de/ard/player/Y3JpZDovL3N3ci5kZS9hZXgvbzEwNzE5MTU/",
            "https://www.ardmediathek.de/swr/live/Y3JpZDovL3N3ci5kZS8xMzQ4MTA0Mg",
        ] {
            assert!(suitable(url), "{url}");
            assert!(!collection_suitable(url), "{url}");
        }
        for (url, id, display, season, version) in [
            (
                "https://www.ardmediathek.de/serie/quiz/staffel-1-originalversion/Y3JpZDovL3dkci5kZS9vbmUvcXVpeg/1/OV",
                "Y3JpZDovL3dkci5kZS9vbmUvcXVpeg",
                Some("quiz/staffel-1-originalversion"),
                Some("1"),
                Some("OV"),
            ),
            (
                "https://www.ardmediathek.de/sendung/tatort/Y3JpZDovL2Rhc2Vyc3RlLmRlL3RhdG9ydA",
                "Y3JpZDovL2Rhc2Vyc3RlLmRlL3RhdG9ydA",
                Some("tatort"),
                None,
                None,
            ),
            (
                "https://www.ardmediathek.de/ard/sammlung/team-muenster/5JpTzLSbWUAK8184IOvEir/",
                "5JpTzLSbWUAK8184IOvEir",
                Some("team-muenster"),
                None,
                None,
            ),
        ] {
            let found = COLLECTION_URL.captures(url).unwrap();
            assert_eq!(&found["id"], id);
            assert_eq!(found.name("display_id").map(|m| m.as_str()), display);
            assert_eq!(found.name("season").map(|m| m.as_str()), season);
            assert_eq!(found.name("version").map(|m| m.as_str()), version);
            assert!(!suitable(url));
        }
        assert!(audiothek_suitable(
            "https://www.ardsounds.de/episode/urn:ard:extra:d2fe7303d2dcbf5d/"
        ));
        assert!(audiothek_playlist_suitable(
            "https://www.ardsounds.de/sendung/100-berlin/urn:ard:show:4d248e0806ce37bc/"
        ));
    }

    #[test]
    fn episode_info_follows_upstream_patterns() {
        let info = |title: &str| episode_info(Some(&title.into()));
        assert_eq!(
            info("Homo sapiens (S06/E07) - Originalversion"),
            vec![
                ("season_number", Value::from(6)),
                ("episode_number", Value::from(7)),
                ("episode", "Homo sapiens - Originalversion".into()),
            ]
        );
        assert_eq!(
            info("a (Folge 12) b"),
            vec![
                ("episode_number", Value::from(12)),
                ("episode", "a b".into())
            ]
        );
        assert_eq!(
            info("Folge 3 \"x\""),
            vec![("episode_number", Value::from(3)), ("episode", "x".into())]
        );
        assert_eq!(info(""), vec![("episode", "".into())]);
        assert!(episode_info(None).is_empty());
        assert!(episode_info(Some(&Value::from(5))).is_empty());
    }

    #[test]
    fn thumbnails_drop_their_query() {
        assert_eq!(
            without_query("https://a.test/i?w={width}&ch=1"),
            "https://a.test/i"
        );
        assert_eq!(
            without_query("https://a.test/i?w=1#f"),
            "https://a.test/i#f"
        );
        assert_eq!(without_query("https://a.test/i"), "https://a.test/i");
    }
}
