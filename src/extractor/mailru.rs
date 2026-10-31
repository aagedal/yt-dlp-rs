//! Upstream `MailRuIE` and `MailRuMusicIE` (`yt_dlp/extractor/mailru.py`): a
//! `my.mail.ru` video's page config (its `metaUrl`) and meta JSON with the
//! direct MP4 qualities, and a music track's search entry (the track's
//! direct audio URL). The music search page (`MailRuMusicSearchIE`) pages
//! through every result and is not ported natively.
use super::{
    Extractor, dict_format, dict_info, download_json_note, download_json_note_headers,
    download_webpage,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct MailRu;
pub struct MailRuMusic;
pub struct MailRuMusicSearch;

const SEARCH_NAME: &str = "mailru:music:search";
const SEARCH_KEY: &str = "MailRuMusicSearch";
/// `MailRuMusicSearchBaseIE`'s `LIMIT`.
const SEARCH_LIMIT: i64 = 100;
static SEARCH_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://my\.mail\.ru/+music/+search/+(?P<id>[^/?#&]+)").unwrap()
});

const NAME: &str = "mailru";
const MUSIC_NAME: &str = "MailRuMusic";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(concat!(
        r"https?://(?:(?:www|m|videoapi)\.)?my\.mail\.ru/+(?:",
        r"video/.*#video=/?(?P<idv1>(?:[^/]+/){3}\d+)|",
        r"(?:videos/embed/)?(?:(?P<idv2prefix>(?:[^/]+/+){2})(?:video/(?:embed/)?)?(?P<idv2suffix>[^/]+/\d+))(?:\.html)?|",
        r"(?:video/embed|\+/video/meta)/(?P<metaid>\d+)",
        r")",
    ))
    .unwrap()
});
static PAGE_CONFIG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<script[^>]+class="sp-video__page-config"[^>]*>(.+?)</script>"#).unwrap()
});
static PAGE_VIDEO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)"video":\s*(\{.+?\}),"#).unwrap());
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)[pP]$").unwrap());
static MUSIC_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://my\.mail\.ru/+music/+songs/+[^/?#&]+-(?P<id>[\da-f]+)").unwrap()
});

/// `MailRuIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `MailRuMusicIE.suitable`.
pub fn music_suitable(url: &str) -> bool {
    MUSIC_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Mail.ru {what} are not supported natively"))
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `int_or_none`, dropping `None`.
fn int_field(value: Option<&Value>) -> Value {
    super::streamable::int_or_none(value)
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// Python's truthiness, for `x.get(a) or x.get(b)`.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

/// `value.get(key)` when truthy, else `fallback`'s value (Python's `or`).
fn or_get<'a>(value: &'a Value, key: &str, fallback: &str) -> Option<&'a Value> {
    let first = value.get(key);
    if truthy(first) {
        first
    } else {
        value.get(fallback)
    }
}

/// `_parse_json(_search_regex([...], default='{}'), fatal=False)`: the page
/// config's JSON, `None` when it does not parse.
fn page_config(webpage: &str) -> Option<Value> {
    let text = PAGE_CONFIG
        .captures(webpage)
        .or_else(|| PAGE_VIDEO.captures(webpage))
        .map(|found| found[1].to_owned())?;
    crate::json::from_str::<Value>(&text).ok()
}

impl Extractor for MailRu {
    fn key(&self) -> &'static str {
        "MailRu"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        start(NAME, url);
        if found.name("metaid").is_some() {
            return Err(unsupported("metadata-only URLs"));
        }
        let video_id = found
            .name("idv1")
            .map(|id| id.as_str().to_owned())
            .unwrap_or_else(|| format!("{}{}", &found["idv2prefix"], &found["idv2suffix"]));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let config = page_config(&webpage).filter(|config| truthy(Some(config)));
        let Some(config) = config else {
            return Err(unsupported("pages without a page config"));
        };
        // `page_config.get('metaUrl') or page_config.get('video', {}).get('metaUrl')
        // or page_config.get('metadataUrl')`.
        let meta_url = [
            config.get("metaUrl").cloned(),
            config
                .get("video")
                .and_then(|video| video.get("metaUrl"))
                .cloned(),
            config.get("metadataUrl").cloned(),
        ]
        .into_iter()
        .flatten()
        .find(|candidate| truthy(Some(candidate)))
        .as_ref()
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| unsupported("videos without a meta URL"))?;
        let meta_url = if meta_url.starts_with("/+/") {
            format!("https://my.mail.ru{meta_url}")
        } else {
            meta_url
        };
        let meta = Url::parse(&meta_url)?;
        let video_data = download_json_note(
            client,
            &meta,
            NAME,
            &video_id,
            "Downloading video meta JSON",
        )?;
        if !truthy(Some(&video_data)) {
            return Err(unsupported("videos with an empty meta JSON"));
        }

        let video_key = crate::cookies::get_cookie("https://my.mail.ru", "video_key");
        let mut formats = Vec::new();
        for entry in video_data
            .get("videos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(video_url) = entry.get("url").and_then(Value::as_str) else {
                continue;
            };
            if let Some(key) = &video_key
                && let Some(host) = Url::parse(video_url)
                    .ok()
                    .and_then(|parsed| parsed.host_str().map(str::to_owned))
            {
                crate::cookies::set_cookie(&host, "video_key", key);
            }
            let format_id = str_or_none(entry.get("key"));
            let height = format_id
                .as_deref()
                .and_then(|id| HEIGHT.captures(id))
                .and_then(|found| found[1].parse::<i64>().ok());
            let video_url = super::proto_relative_http(video_url);
            formats.push(dict_format(
                vec![
                    ("url", video_url.as_str().into()),
                    (
                        "format_id",
                        format_id.clone().map_or(Value::Null, Value::from),
                    ),
                    ("height", height.map_or(Value::Null, Value::from)),
                ],
                NAME,
                &video_id,
            )?);
        }

        let meta_data = video_data
            .get("meta")
            .ok_or_else(|| super::clyp::key_error(&video_id, "meta"))?;
        let title = meta_data
            .get("title")
            .and_then(Value::as_str)
            .map(|title| title.strip_suffix(".mp4").unwrap_or(title).to_owned())
            .ok_or_else(|| super::clyp::key_error(&video_id, "title"))?;
        let author = video_data
            .get("author")
            .ok_or_else(|| super::clyp::key_error(&video_id, "author"))?;
        let uploader = author.get("name").cloned().unwrap_or(Value::Null);
        let uploader_id = if truthy(author.get("id")) {
            author.get("id").cloned().unwrap_or(Value::Null)
        } else {
            author.get("email").cloned().unwrap_or(Value::Null)
        };
        let view_count = int_field(
            video_data
                .get("viewsCount")
                .filter(|value| truthy(Some(*value)))
                .or_else(|| video_data.get("views_count")),
        );
        let acc_id = str_or_none(meta_data.get("accId")).filter(|id| !id.is_empty());
        let item_id = str_or_none(meta_data.get("itemId")).filter(|id| !id.is_empty());
        let content_id = match (acc_id, item_id) {
            (Some(acc), Some(item)) => format!("{acc}_{item}"),
            _ => video_id.clone(),
        };
        let fields = vec![
            ("id", Value::from(content_id)),
            ("title", Value::from(title)),
            (
                "thumbnail",
                meta_data.get("poster").cloned().unwrap_or(Value::Null),
            ),
            ("timestamp", int_field(meta_data.get("timestamp"))),
            ("uploader", uploader),
            ("uploader_id", uploader_id),
            ("duration", int_field(meta_data.get("duration"))),
            ("view_count", view_count),
            ("formats", Value::Null),
        ];
        Ok(Some(dict_info(url, (NAME, "MailRu"), fields, formats)))
    }
}

/// `_search(query, url, audio_id)`: the first dict of the AJAX reply.
fn music_search(client: &Client, page: &Url, title: &str, audio_id: &str) -> Result<Value> {
    use std::io::Read;
    let search_params = crate::output::python_json_dumps(&serde_json::json!({
        "music": {"limit": 100, "offset": 0},
    }));
    let search = Url::parse_with_params(
        "https://my.mail.ru/cgi-bin/my/ajax",
        &[
            ("xemail", ""),
            ("ajax_call", "1"),
            ("func_name", "music.search"),
            ("mna", ""),
            ("mnb", ""),
            ("arg_query", title),
            ("arg_extended", "1"),
            ("arg_search_params", search_params.as_str()),
            ("arg_limit", "100"),
            ("arg_offset", "0"),
        ],
    )?;
    crate::screen::screen(&format!(
        "[{MUSIC_NAME}] {audio_id}: Downloading songs JSON page 1"
    ));
    let mut response = super::direct::request_with_headers(
        client,
        &search,
        Some(page),
        &[
            ("Referer", page.as_str()),
            ("X-Requested-With", "XMLHttpRequest"),
        ],
    )?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(audio_id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, response.headers());
    let reply: Value = crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{MUSIC_NAME}] {audio_id}: Failed to parse JSON  (caused by {error})"
        ))
    })?;
    reply
        .as_array()
        .and_then(|items| items.iter().find(|item| item.is_object()))
        .cloned()
        .ok_or_else(|| unsupported("music searches without a result dict"))
}

impl Extractor for MailRuMusic {
    fn key(&self) -> &'static str {
        MUSIC_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = MUSIC_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        start(MUSIC_NAME, url);
        let audio_id = found["id"].to_owned();
        let webpage = download_webpage(client, url, MUSIC_NAME, &audio_id)?;
        let Some(title) = crate::html::og_search_property(&webpage, "title") else {
            return Err(unsupported("tracks without an OpenGraph title"));
        };
        let search = music_search(client, url, &title, &audio_id)?;
        let music_data = search
            .get("MusicData")
            .and_then(Value::as_array)
            .ok_or_else(|| super::clyp::key_error(&audio_id, "MusicData"))?;
        let track = music_data
            .iter()
            .find(|track| track.get("File").and_then(Value::as_str) == Some(audio_id.as_str()))
            .ok_or_else(|| unsupported("tracks missing from the search results"))?;
        let audio_url = super::proto_relative_http(
            track
                .get("URL")
                .and_then(Value::as_str)
                .ok_or_else(|| super::clyp::key_error(&audio_id, "URL"))?,
        );
        let audio_url = audio_url.as_str();
        let file_id = track
            .get("File")
            .and_then(Value::as_str)
            .ok_or_else(|| super::clyp::key_error(&audio_id, "File"))?;

        let thumbnail = or_get(track, "AlbumCoverURL", "FiledAlbumCover")
            .cloned()
            .unwrap_or(Value::Null);
        let uploader = or_get(track, "OwnerName", "OwnerName_Text_HTML")
            .cloned()
            .unwrap_or(Value::Null);
        let uploader_id = track.get("UploaderID").cloned().unwrap_or(Value::Null);
        // `int_or_none(DurationInSeconds) or parse_duration(Duration or DurationStr)`.
        let seconds = int_field(track.get("DurationInSeconds"));
        let duration = if truthy(Some(&seconds)) {
            seconds
        } else {
            duration_value(track.get("Duration").or_else(|| track.get("DurationStr")))
        };
        let view_count = int_field(track.get("PlayCount").or_else(|| track.get("PlayCount_hr")));
        let track_name = or_get(track, "Name", "Name_Text_HTML")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let artist = or_get(track, "Author", "Author_Text_HTML")
            .and_then(Value::as_str)
            .map(str::to_owned);
        // `info['title'] = title`: the page's OpenGraph title wins over the track's.
        let mut fields = Map::new();
        fields.insert("extractor_key".into(), Value::from(MUSIC_NAME));
        fields.insert("id".into(), Value::from(file_id));
        fields.insert("title".into(), Value::from(title));
        fields.insert("thumbnail".into(), thumbnail);
        fields.insert("uploader".into(), uploader);
        fields.insert("uploader_id".into(), uploader_id);
        fields.insert("duration".into(), duration);
        fields.insert("view_count".into(), view_count);
        fields.insert("vcodec".into(), Value::from("none"));
        fields.insert("abr".into(), int_field(track.get("BitRate")));
        fields.insert("track".into(), track_name.map_or(Value::Null, Value::from));
        fields.insert("artist".into(), artist.map_or(Value::Null, Value::from));
        fields.insert(
            "album".into(),
            track.get("Album").cloned().unwrap_or(Value::Null),
        );
        fields.insert("url".into(), Value::from(audio_url));
        let keys = fields.keys().cloned().collect::<Vec<_>>().join(",");

        let media = Url::parse(audio_url)?;
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp3".into()),
            "",
        );
        // The audio format: its int `abr` is what processing derives `tbr` and
        // `filesize_approx` from.
        let mut format = dict_format(
            vec![
                ("url", Value::from(audio_url)),
                ("ext", Value::from("mp3")),
                ("format_id", Value::from("")),
                ("vcodec", Value::from("none")),
                ("abr", int_field(track.get("BitRate"))),
                // Upstream's `tbr` is `abr` (both ints) when no bitrate is given.
                ("tbr", int_field(track.get("BitRate"))),
            ],
            MUSIC_NAME,
            &audio_id,
        )?;
        crate::info_order::mark_int_rates(&mut format);
        info.formats = vec![format];
        info.format_id = None;
        info.id = fields["id"].as_str().unwrap_or_default().to_owned();
        info.title = fields["title"].as_str().unwrap_or_default().to_owned();
        for (key, value) in &fields {
            if !matches!(key.as_str(), "id" | "title" | "url" | "extractor_key") {
                crate::metadata_parser::set_field(&mut info, key, value.clone());
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = "mailru:music".into();
        info.extractor_key = MUSIC_NAME.into();
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("untyped-dict;{keys}"),
        )))
    }
}

/// `parse_duration(text)` as a JSON number (`None` stays `null`).
fn duration_value(text: Option<&Value>) -> Value {
    match text.and_then(Value::as_str).and_then(crate::parse_duration) {
        Some(crate::info::PyNumber::Int(seconds)) => Value::from(seconds),
        Some(crate::info::PyNumber::Float(seconds)) => Value::from(seconds),
        None => Value::Null,
    }
}

/// `MailRuMusicSearchIE._VALID_URL`.
pub fn search_suitable(url: &str) -> bool {
    SEARCH_URL.is_match(url)
}

/// `urllib.parse.unquote` of a path segment.
fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%' && i + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok())
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `MailRuMusicSearchBaseIE._search(query, url, audio_id, limit, offset)`:
/// the first object of the `music.search` ajax answer.
fn search_page(
    client: &Client,
    page: &Url,
    query: &str,
    audio_id: &str,
    offset: i64,
) -> Result<Value> {
    let ajax = Url::parse_with_params(
        "https://my.mail.ru/cgi-bin/my/ajax",
        &[
            ("xemail", ""),
            ("ajax_call", "1"),
            ("func_name", "music.search"),
            ("mna", ""),
            ("mnb", ""),
            ("arg_query", query),
            ("arg_extended", "1"),
            (
                "arg_search_params",
                &format!(r#"{{"music": {{"limit": {SEARCH_LIMIT}, "offset": {offset}}}}}"#),
            ),
            ("arg_limit", &SEARCH_LIMIT.to_string()),
            ("arg_offset", &offset.to_string()),
        ],
    )?;
    let answer = download_json_note_headers(
        client,
        &ajax,
        SEARCH_NAME,
        audio_id,
        &format!("Downloading songs JSON page {}", offset / SEARCH_LIMIT + 1),
        &[
            ("Referer", page.as_str()),
            ("X-Requested-With", "XMLHttpRequest"),
        ],
    )?;
    answer
        .as_array()
        .and_then(|items| items.iter().find(|item| item.is_object()).cloned())
        .ok_or_else(|| super::clyp::key_error(audio_id, "MusicData"))
}

/// `_extract_track(t, fatal=False)`: a search answer's track as a playlist
/// entry (`untyped-dict` with its direct audio URL).
fn search_track(track: &Value, page: &Url) -> Result<Option<VideoInfo>> {
    let Some(audio_url) = track
        .get("URL")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    else {
        return Ok(None);
    };
    let Some(audio_id) = track
        .get("File")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    else {
        return Ok(None);
    };
    let thumbnail = or_get(track, "AlbumCoverURL", "FiledAlbumCover")
        .cloned()
        .unwrap_or(Value::Null);
    let uploader = or_get(track, "OwnerName", "OwnerName_Text_HTML")
        .cloned()
        .unwrap_or(Value::Null);
    let uploader_id = track.get("UploaderID").cloned().unwrap_or(Value::Null);
    let seconds = int_field(track.get("DurationInSeconds"));
    let duration = if truthy(Some(&seconds)) {
        seconds
    } else {
        duration_value(track.get("Duration").or_else(|| track.get("DurationStr")))
    };
    let view_count = int_field(track.get("PlayCount").or_else(|| track.get("PlayCount_hr")));
    let track_name = or_get(track, "Name", "Name_Text_HTML")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let artist = or_get(track, "Author", "Author_Text_HTML")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let title = match (&track_name, &artist) {
        (Some(name), Some(artist)) => format!("{artist} - {name}"),
        (Some(name), None) => name.clone(),
        (None, _) => audio_id.to_owned(),
    };
    let bitrate = int_field(track.get("BitRate"));
    let audio = super::proto_relative_http(audio_url);

    let mut fields = Map::new();
    fields.insert("extractor_key".into(), Value::from(MUSIC_NAME));
    fields.insert("id".into(), Value::from(audio_id));
    fields.insert("title".into(), Value::from(title));
    fields.insert("thumbnail".into(), thumbnail);
    fields.insert("uploader".into(), uploader);
    fields.insert("uploader_id".into(), uploader_id);
    fields.insert("duration".into(), duration);
    fields.insert("view_count".into(), view_count);
    fields.insert("vcodec".into(), Value::from("none"));
    fields.insert("abr".into(), bitrate.clone());
    fields.insert("track".into(), track_name.map_or(Value::Null, Value::from));
    fields.insert("artist".into(), artist.map_or(Value::Null, Value::from));
    fields.insert(
        "album".into(),
        track.get("Album").cloned().unwrap_or(Value::Null),
    );
    fields.insert("url".into(), Value::from(audio.clone()));
    let keys = fields.keys().cloned().collect::<Vec<_>>().join(",");

    let media = Url::parse(&audio)?;
    let mut info = super::direct::with_format(
        super::direct::build_at(media.clone(), &media, "mp3".into()),
        "",
    );
    let mut format = dict_format(
        vec![
            ("url", Value::from(audio.clone())),
            ("ext", Value::from("mp3")),
            ("format_id", Value::from("")),
            ("vcodec", Value::from("none")),
            ("abr", bitrate.clone()),
            ("tbr", bitrate),
        ],
        MUSIC_NAME,
        audio_id,
    )?;
    crate::info_order::mark_int_rates(&mut format);
    info.formats = vec![format];
    info.format_id = None;
    info.id = audio_id.to_owned();
    info.title = fields["title"].as_str().unwrap_or_default().to_owned();
    for (key, value) in &fields {
        if !matches!(key.as_str(), "id" | "title" | "url" | "extractor_key") {
            crate::metadata_parser::set_field(&mut info, key, value.clone());
        }
    }
    info.direct = None;
    info.webpage_url = page.clone();
    info.extractor = SEARCH_NAME.into();
    info.extractor_key = MUSIC_NAME.into();
    Ok(Some(crate::info_order::mark_info(
        info,
        &format!("untyped-dict;{keys}"),
    )))
}

impl Extractor for MailRuMusicSearch {
    fn key(&self) -> &'static str {
        SEARCH_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SEARCH_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let query = unquote(&found["id"]);
        start(SEARCH_NAME, url);
        let mut entries = Vec::new();
        let mut offset = 0;
        loop {
            let search = search_page(client, url, &query, &query, offset)?;
            let Some(music_data) = search.get("MusicData").and_then(Value::as_array) else {
                break;
            };
            if music_data.is_empty() {
                break;
            }
            for track in music_data {
                if let Some(entry) = search_track(track, url)? {
                    entries.push(entry);
                }
            }
            // `try_get(search, lambda x: x['Results']['music']['Total'], int)`.
            let total = search
                .pointer("/Results/music/Total")
                .and_then(|value| match value {
                    Value::String(text) => text.trim().parse::<i64>().ok(),
                    Value::Number(number) => number.as_i64(),
                    _ => None,
                });
            if total.is_some_and(|total| offset > total) {
                break;
            }
            offset += SEARCH_LIMIT;
        }

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: query,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: SEARCH_NAME.into(),
            extractor_key: SEARCH_KEY.into(),
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
            "http://my.mail.ru/corp/hitech/video/news_hi-tech_mail_ru/1263.html"
        ));
        assert!(suitable(
            "http://my.mail.ru/video/top#video=/mail/sonypicturesrus/75/76"
        ));
        assert!(suitable(
            "https://my.mail.ru/video/embed/7949340477499637815"
        ));
        assert!(!suitable(
            "https://my.mail.ru/music/songs/x-4e31f7125d0dfaef505d947642366893"
        ));
        assert!(music_suitable(
            "https://my.mail.ru/music/songs/%D0%BC8-4e31f7125d0dfaef505d947642366893"
        ));
        assert!(!music_suitable(
            "https://my.mail.ru/music/search/black%20shadow"
        ));
    }

    #[test]
    fn search_urls_follow_upstream_valid_url() {
        assert!(search_suitable(
            "https://my.mail.ru/music/search/black%20shadow"
        ));
        assert!(!search_suitable(
            "https://my.mail.ru/music/songs/track-15732b7aa7e1f182e8d89646f98b079d"
        ));
    }
}
