//! Upstream `BandcampIE` (`yt_dlp/extractor/bandcamp.py`): a track page's
//! `data-tralbum`/`data-embed` JSON, its streaming files and, for free
//! downloads, the download page's formats through their stat URLs.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Bandcamp;

const NAME: &str = "Bandcamp";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?P<uploader>[^/]+)\.bandcamp\.com/track/(?P<id>[^/?#&]+)",
    )
    .unwrap()
});
static ALBUM_ARTIST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<h3 class="albumTitle">[\S\s]*?by\s*<span>\s*<a href="[^>]+">\s*([^>]+?)\s*</a>"#)
        .unwrap()
});

/// `BandcampIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    fail_in(NAME, id, message)
}

/// [`fail`] for one of the module's classes (`[Bandcamp:album] ID: ...`).
fn fail_in(extractor: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{extractor}] {id}: {message}"))
}

/// `_extract_data_attr(webpage, id, attr)`: `data-ATTR="{...}"` JSON.
fn data_attr(webpage: &str, id: &str, attr: &str, fatal: bool) -> Result<Value> {
    data_attr_in(NAME, webpage, id, attr, fatal)
}

fn data_attr_in(
    extractor: &str,
    webpage: &str,
    id: &str,
    attr: &str,
    fatal: bool,
) -> Result<Value> {
    let pattern = Regex::new(&format!(r#"data-{attr}=(?:"(\{{.+?\}})"|'(\{{.+?\}})')"#))
        .map_err(|error| Error::Reported(error.to_string()))?;
    // The search is fatal; `fatal` only covers the JSON parse.
    let Some(found) = pattern.captures(webpage) else {
        return Err(fail_in(
            extractor,
            id,
            &format!(
                "Unable to extract {attr} data; {}",
                crate::BUG_REPORTS_MESSAGE
            ),
        ));
    };
    let raw = found.get(1).or(found.get(2)).map_or("", |raw| raw.as_str());
    let text = crate::html::unescape_html(raw);
    serde_json::from_str(&text).or_else(|error| {
        if fatal {
            Err(fail_in(
                extractor,
                id,
                &format!("Failed to parse JSON  (caused by {error})"),
            ))
        } else {
            crate::screen::warning(&format!(
                "[{extractor}] {id}: Failed to parse JSON  (caused by {error})"
            ));
            Ok(json!({}))
        }
    })
}

/// `BandcampBaseIE._download_webpage`: a page without a title warns
/// (`_html_extract_title`), and a client challenge needs impersonation.
fn fetch_page(client: &Client, url: &Url, id: &str, note: &str) -> Result<String> {
    fetch_page_in(NAME, client, url, id, note)
}

fn fetch_page_in(
    extractor: &'static str,
    client: &Client,
    url: &Url,
    id: &str,
    note: &str,
) -> Result<String> {
    let page = super::download_webpage_note(client, url, extractor, id, note)?;
    match crate::html::html_extract_title(&page).as_deref() {
        Some("Client Challenge") => Err(Error::UnsupportedHtml5(
            "Bandcamp's client challenge needs browser impersonation, which is unavailable".into(),
        )),
        Some(_) => Ok(page),
        None => {
            super::search_warning(extractor, "title");
            Ok(page)
        }
    }
}

fn float(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn int(value: Option<&Value>) -> Option<i64> {
    float(value).map(|number| number as i64)
}

/// `_proto_relative_url(url, 'http:')`.
fn proto_relative(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("http://{rest}"),
        None => url.to_owned(),
    }
}

/// `parse_filesize('NUMBER UNIT')` for the download page's sizes.
fn parse_filesize(text: &str) -> Option<i64> {
    static SIZE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\s*([\d.,]+)\s*([kmgt]?i?b)\s*$").unwrap());
    let found = SIZE.captures(text)?;
    let number: f64 = found[1].replace(',', ".").parse().ok()?;
    let unit = found[2].to_lowercase();
    let factor = match unit.as_str() {
        "b" => 1.0,
        "kb" => 1000.0,
        "kib" => 1024.0,
        "mb" => 1000f64.powi(2),
        "mib" => 1024f64.powi(2),
        "gb" => 1000f64.powi(3),
        "gib" => 1024f64.powi(3),
        "tb" => 1000f64.powi(4),
        "tib" => 1024f64.powi(4),
        _ => return None,
    };
    Some((number * factor) as i64)
}

fn format_from(mut format: Map<String, Value>, id: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = format.keys().cloned().collect();
    for key in ["abr", "filesize", "format_note"] {
        if format.get(key).is_some_and(Value::is_null) {
            format.shift_remove(key);
        }
    }
    if format.get("ext").is_none_or(Value::is_null) {
        let ext = format
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext)
            .unwrap_or_else(|| "unknown_video".into());
        format.insert("ext".into(), ext.into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_rates(&mut format);
    Ok(format)
}

impl Extractor for Bandcamp {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (title_id, uploader) = (found["id"].to_owned(), found["uploader"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = fetch_page(client, url, &title_id, "Downloading webpage")?;
        let tralbum = data_attr(&webpage, &title_id, "tralbum", true)?;
        let mut thumbnail = crate::html::og_search_property(&webpage, "image");
        let mut formats = Vec::new();
        let mut track: Option<String> = None;
        let mut track_id: Option<String> = None;
        let mut track_number = None;
        let mut duration = None;
        if let Some(info) = tralbum
            .pointer("/trackinfo/0")
            .filter(|info| info.is_object())
        {
            if let Some(files) = info.get("file").and_then(Value::as_object) {
                for (format_id, format_url) in files {
                    let Some(format_url) =
                        format_url.as_str().and_then(super::json_ld::url_or_none)
                    else {
                        continue;
                    };
                    let (ext, abr) = format_id.split_once('-').unwrap_or((format_id, ""));
                    let mut format = Map::new();
                    format.insert("format_id".into(), format_id.clone().into());
                    format.insert("url".into(), proto_relative(&format_url).into());
                    format.insert("ext".into(), ext.into());
                    format.insert("vcodec".into(), "none".into());
                    format.insert("acodec".into(), ext.into());
                    format.insert("abr".into(), abr.parse::<i64>().ok().into());
                    formats.push(format_from(format, &title_id)?);
                }
            }
            track = info.get("title").and_then(Value::as_str).map(str::to_owned);
            track_id = ["track_id", "id"]
                .iter()
                .find_map(|key| match info.get(*key) {
                    Some(Value::Number(number)) if number.as_f64() != Some(0.0) => {
                        Some(number.to_string())
                    }
                    Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
                    _ => None,
                });
            track_number = int(info.get("track_num"));
            duration = float(info.get("duration"));
        }
        let embed = data_attr(&webpage, &title_id, "embed", false)?;
        let current = tralbum.get("current").cloned().unwrap_or_else(|| json!({}));
        let text_of = |value: Option<&Value>| {
            value
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let mut artist = text_of(embed.get("artist"))
            .or_else(|| text_of(current.get("artist")))
            .or_else(|| text_of(tralbum.get("artist")));
        let album_artist = ALBUM_ARTIST
            .captures(&webpage)
            .map(|found| found[1].to_owned());
        if album_artist.is_none() {
            super::search_warning(NAME, "album artist");
        }
        let timestamp = text_of(current.get("publish_date"))
            .or_else(|| text_of(tralbum.get("album_publish_date")))
            .and_then(|date| crate::dates::unified_timestamp(&date, false));
        if let Some(download_link) = text_of(tralbum.get("freeDownloadPage")) {
            let id = match tralbum.get("id") {
                Some(Value::Number(number)) => number.to_string(),
                Some(Value::String(text)) => text.clone(),
                _ => {
                    return Err(fail(
                        &title_id,
                        "An extractor error has occurred. (caused by KeyError('id'))",
                    ));
                }
            };
            track_id = Some(id.clone());
            let download_url = Url::parse(&download_link)?;
            let download_page = fetch_page(
                client,
                &download_url,
                &id,
                "Downloading free downloads page",
            )?;
            let blob = data_attr(&download_page, &id, "blob", true)?;
            if let Some(info) = blob
                .pointer("/digital_items/0")
                .or_else(|| blob.pointer("/download_items/0"))
                .filter(|info| info.is_object())
                && let Some(downloads) = info.get("downloads").and_then(Value::as_object)
            {
                if track.is_none() {
                    track = text_of(info.get("title"));
                }
                if artist.is_none() {
                    artist = text_of(info.get("artist"));
                }
                if thumbnail.is_none() {
                    thumbnail = text_of(info.get("thumb_url"));
                }
                let mut extensions: Vec<(String, String)> = Vec::new();
                for entry in blob
                    .get("download_formats")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let (Some(name), Some(ext)) = (
                        entry.get("name").and_then(Value::as_str),
                        entry.get("file_extension").and_then(Value::as_str),
                    ) {
                        extensions.push((name.to_owned(), ext.trim_matches('.').to_owned()));
                    }
                }
                for (key, entry) in downloads {
                    let Some(format_url) = entry
                        .get("url")
                        .and_then(Value::as_str)
                        .filter(|u| !u.is_empty())
                    else {
                        continue;
                    };
                    let rand = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |time| time.as_millis() as u64 % 1_000_000_007);
                    let Ok(stat_base) = Url::parse(&proto_relative(
                        &format_url.replace("/download/", "/statdownload/"),
                    )) else {
                        continue;
                    };
                    let stat_url =
                        super::update_url_query(&stat_base, &[(".rand", &rand.to_string())]);
                    let format_id =
                        text_of(entry.get("encoding_name")).unwrap_or_else(|| key.clone());
                    let stat = super::download_json_transformed(
                        client,
                        &stat_url,
                        NAME,
                        &id,
                        &format!("Downloading {format_id} JSON"),
                        &|text| {
                            let start = text.find('{').unwrap_or(0);
                            let end = text.rfind('}').map_or(text.len(), |end| end + 1);
                            Ok(text[start..end].to_owned())
                        },
                    );
                    let stat = match stat {
                        Ok(stat) => stat,
                        Err(error) => {
                            crate::screen::warning(&error.to_string());
                            continue;
                        }
                    };
                    let Some(retry) = stat
                        .get("retry_url")
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                    else {
                        continue;
                    };
                    let mut format = Map::new();
                    format.insert("url".into(), proto_relative(&retry).into());
                    format.insert(
                        "ext".into(),
                        extensions
                            .iter()
                            .find(|(name, _)| *name == format_id)
                            .map(|(_, ext)| ext.clone())
                            .into(),
                    );
                    format.insert("format_id".into(), format_id.clone().into());
                    format.insert(
                        "format_note".into(),
                        entry.get("description").cloned().unwrap_or(Value::Null),
                    );
                    format.insert(
                        "filesize".into(),
                        entry
                            .get("size_mb")
                            .and_then(Value::as_str)
                            .and_then(parse_filesize)
                            .into(),
                    );
                    format.insert("vcodec".into(), "none".into());
                    format.insert(
                        "acodec".into(),
                        format_id.split('-').next().unwrap_or_default().into(),
                    );
                    formats.push(format_from(format, &id)?);
                }
            }
        }
        let title = match &artist {
            Some(artist) => format!("{artist} - {}", track.as_deref().unwrap_or("None")),
            None => track.clone().unwrap_or_default(),
        };
        if duration.is_none_or(|duration| duration == 0.0) {
            duration = crate::html::html_search_meta(&webpage, "duration")
                .and_then(|text| text.trim().parse().ok());
        }
        let tags: Vec<Value> =
            crate::html_elements::elements_text_and_html_by_class("tag", &webpage)
                .unwrap_or_default()
                .into_iter()
                .map(|(content, _)| Value::from(crate::html::clean_html(&content)))
                .collect();
        let mut dict = Map::new();
        dict.insert("id".into(), track_id.clone().into());
        dict.insert("title".into(), title.into());
        dict.insert("thumbnail".into(), thumbnail.into());
        dict.insert("uploader".into(), artist.clone().into());
        dict.insert("uploader_id".into(), uploader.clone().into());
        dict.insert(
            "uploader_url".into(),
            format!("https://{uploader}.bandcamp.com").into(),
        );
        dict.insert("timestamp".into(), timestamp.into());
        dict.insert(
            "release_timestamp".into(),
            text_of(tralbum.get("album_release_date"))
                .and_then(|date| crate::dates::unified_timestamp(&date, false))
                .into(),
        );
        dict.insert(
            "duration".into(),
            duration.map_or(Value::Null, |d| json!(d)),
        );
        dict.insert("track".into(), track.into());
        dict.insert("track_number".into(), track_number.into());
        dict.insert("track_id".into(), track_id.into());
        dict.insert("artist".into(), artist.into());
        dict.insert(
            "album".into(),
            embed.get("album_title").cloned().unwrap_or(Value::Null),
        );
        dict.insert("album_artist".into(), album_artist.into());
        dict.insert("formats".into(), json!([]));
        dict.insert("tags".into(), Value::Array(tags));
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        if formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.formats = formats;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let mut keys = Vec::new();
        for (key, value) in dict {
            keys.push(key.clone());
            match key.as_str() {
                "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
                "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                "formats" => {}
                _ if value.is_null() => {
                    info.extra.insert(key, Value::Null);
                }
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("dict;{}", keys.join(",")),
        )))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

// Upstream `BandcampAlbumIE`, `BandcampWeeklyIE` and `BandcampUserIE`.

const ALBUM_NAME: &str = "Bandcamp:album";
const ALBUM_KEY: &str = "BandcampAlbum";
const WEEKLY_NAME: &str = "Bandcamp:weekly";
const WEEKLY_KEY: &str = "BandcampWeekly";
const USER_NAME: &str = "Bandcamp:user";
const USER_KEY: &str = "BandcampUser";

static ALBUM_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?P<subdomain>[^.]+)\.)?bandcamp\.com/album/(?P<id>[^/?#&]+)",
    )
    .unwrap()
});
static WEEKLY_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?bandcamp\.com/radio/?\?(?:[^#]+&)?show=(?P<id>\d+)",
    )
    .unwrap()
});
/// The `(?!www\.)` lookahead is applied by [`user_suitable`].
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?P<id>[^.]+)\.bandcamp\.com(?:/music)?/?(?:[#?]|$)")
        .unwrap()
});
/// `_yield_items`: `<li data-item-id=...><a href=...>`, minus merch links
/// (the `(?!...)` lookahead is checked in [`is_merch`]).
static ITEM_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<li data-item-id=["'][^>]+>\s*<a href=["']([^"']+)"#).unwrap());
static TRACK_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div[^>]+trackTitle["'][^"']+["']([^"']+)"#).unwrap());
static MUSIC_GRID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<[^>]+id=["']music-grid["'][^>]*>"#).unwrap());

/// `BandcampAlbumIE.suitable`: never a track or weekly show URL.
pub fn album_suitable(url: &str) -> bool {
    ALBUM_URL.is_match(url) && !suitable(url) && !WEEKLY_URL.is_match(url)
}

/// `BandcampWeeklyIE.suitable`.
pub fn weekly_suitable(url: &str) -> bool {
    WEEKLY_URL.is_match(url)
}

/// `BandcampUserIE.suitable`: `(?!www\.)`.
pub fn user_suitable(url: &str) -> bool {
    USER_URL
        .captures(url)
        .is_some_and(|found| &found["id"] != "www")
}

/// Python's truthiness for an info-dict value.
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

/// `str_or_none` for an ID that may be a JSON number.
fn id_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `url_result(url, ie, id=..., video_title=...)` as a playlist entry; with
/// no `ie` the entry only carries its URL (`_type: url`).
fn url_entry(
    target: &str,
    ie: Option<&str>,
    id: Option<String>,
    title: Option<Value>,
) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    if let Some(ie) = ie {
        entry.insert("ie_key".into(), ie.into());
    }
    if let Some(id) = id {
        entry.insert("id".into(), id.into());
    }
    if let Some(title) = title {
        entry.insert("title".into(), title);
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie,
            "extractor": ie.map(str::to_lowercase),
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `KeyError` from a missing field: no `[IE]` prefix.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `BandcampUserIE._yield_items`: `(?!/merch)` links, then the track titles
/// when there are none, then the `music-grid` element's `page_url`s.
fn user_items(webpage: &str) -> Vec<String> {
    let is_merch = |href: &str| {
        href.find('/')
            .is_some_and(|index| href[index..].starts_with("/merch"))
    };
    let mut items: Vec<String> = ITEM_LINK
        .captures_iter(webpage)
        .map(|found| found[1].to_owned())
        .filter(|href| !is_merch(href))
        .collect();
    if items.is_empty() {
        items = TRACK_TITLE
            .captures_iter(webpage)
            .map(|found| found[1].to_owned())
            .collect();
    }
    if let Some(grid) = MUSIC_GRID.find(webpage) {
        let attributes = crate::html_elements::extract_attributes(grid.as_str());
        if let Some(Value::Array(list)) = attributes
            .get("data-client-items")
            .and_then(Value::as_str)
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        {
            items.extend(
                list.iter()
                    .filter_map(|item| item.get("page_url")?.as_str().map(str::to_owned)),
            );
        }
    }
    items
}

pub struct BandcampAlbum;

impl Extractor for BandcampAlbum {
    fn key(&self) -> &'static str {
        ALBUM_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ALBUM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let uploader_id = found.name("subdomain").map(|name| name.as_str().to_owned());
        let album_id = found["id"].to_owned();
        crate::screen::set_extractor(ALBUM_NAME);
        crate::screen::screen(&format!(
            "[{ALBUM_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = fetch_page_in(ALBUM_NAME, client, url, &album_id, "Downloading webpage")?;
        let tralbum = data_attr_in(ALBUM_NAME, &webpage, &album_id, "tralbum", true)?;
        let track_info = tralbum
            .get("trackinfo")
            .and_then(Value::as_array)
            .filter(|tracks| !tracks.is_empty())
            .ok_or_else(|| {
                fail_in(
                    ALBUM_NAME,
                    &album_id,
                    &format!(
                        "The page doesn't contain any tracks; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ),
                )
            })?;
        // Only tracks with duration info have songs.
        let mut entries = Vec::new();
        for track in track_info
            .iter()
            .filter(|track| truthy(track.get("duration")))
        {
            let link = track
                .get("title_link")
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(&album_id, "title_link"))?;
            let target = url.join(link)?;
            let id = if truthy(track.get("track_id")) {
                id_text(track.get("track_id"))
            } else {
                id_text(track.get("id"))
            };
            entries.push(url_entry(
                target.as_str(),
                Some("Bandcamp"),
                id,
                track.get("title").cloned(),
            )?);
        }

        let current = tralbum
            .get("current")
            .filter(|current| truthy(Some(current)))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let mut extra = Map::new();
        extra.insert(
            "uploader_id".into(),
            uploader_id.map_or(Value::Null, Value::from),
        );
        extra.insert(
            "description".into(),
            current.get("about").cloned().unwrap_or(Value::Null),
        );
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;_type,uploader_id,id,title,description".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: album_id,
            title: current
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            webpage_url: url.clone(),
            extractor: ALBUM_NAME.into(),
            extractor_key: ALBUM_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

pub struct BandcampWeekly;

impl Extractor for BandcampWeekly {
    fn key(&self) -> &'static str {
        WEEKLY_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = WEEKLY_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let show_id = found["id"].to_owned();
        crate::screen::set_extractor(WEEKLY_NAME);
        crate::screen::screen(&format!(
            "[{WEEKLY_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let item_id: i64 = show_id.parse().unwrap_or_default();
        let api = Url::parse("https://bandcamp.com/api/player/2/player_data_web")?;
        let response = super::post_json_note_headers(
            client,
            &api,
            WEEKLY_NAME,
            &show_id,
            "Downloading radio show JSON",
            &[("Content-Type", "application/json")],
            json!({"item_id": item_id, "item_type": "radio"}).to_string(),
        )?;
        let show_data = response
            .get("tracklist")
            .ok_or_else(|| key_error(&show_id, "tracklist"))?;
        let audio_data = show_data
            .get("compiledTrack")
            .ok_or_else(|| key_error(&show_id, "compiledTrack"))?;
        let stream_url = audio_data
            .get("streamUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&show_id, "streamUrl"))?
            .to_owned();
        // `traverse_obj(stream_url, ({parse_qs}, 'enc', -1))`
        let format_id = Url::parse(&stream_url).ok().and_then(|parsed| {
            parsed
                .query_pairs()
                .filter(|(name, _)| name == "enc")
                .last()
                .map(|(_, value)| value.into_owned())
        });
        let format_text = format_id.clone().unwrap_or_default();
        let (encoding, bitrate_str) = format_text.split_once('-').unwrap_or((&format_text, ""));

        let series_title = show_data
            .get("subtitle")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let release_timestamp = show_data
            .get("date")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, false));
        let release_date = release_timestamp.and_then(|timestamp| {
            crate::dates::strftime_or_none(
                crate::dates::StrftimeInput::Integer(timestamp),
                "%Y-%m-%d",
            )
            .ok()
            .flatten()
        });
        let title = [series_title.clone(), release_date]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let thumbnail = match show_data.get("imageId") {
            value if truthy(value) => {
                id_text(value).map(|id| format!("https://f4.bcbits.com/img/{id}_0.jpg"))
            }
            _ => None,
        };

        let mut format = Map::new();
        format.insert("url".into(), stream_url.clone().into());
        format.insert(
            "format_id".into(),
            format_id.clone().map_or(Value::Null, Value::from),
        );
        format.insert(
            "ext".into(),
            if encoding.is_empty() { "mp3" } else { encoding }.into(),
        );
        format.insert(
            "acodec".into(),
            if encoding.is_empty() {
                Value::Null
            } else {
                encoding.into()
            },
        );
        format.insert("vcodec".into(), "none".into());
        format.insert("abr".into(), bitrate_str.parse::<i64>().ok().into());
        let formats = vec![format_from(format, &show_id)?];

        let mut dict = Map::new();
        dict.insert("id".into(), show_id.clone().into());
        dict.insert("episode_id".into(), show_id.clone().into());
        dict.insert(
            "episode".into(),
            show_data.get("title").cloned().unwrap_or(Value::Null),
        );
        dict.insert("title".into(), title.into());
        dict.insert(
            "series".into(),
            series_title.map_or(Value::Null, Value::from),
        );
        dict.insert(
            "thumbnail".into(),
            thumbnail.map_or(Value::Null, Value::from),
        );
        dict.insert(
            "description".into(),
            show_data.get("description").cloned().unwrap_or(Value::Null),
        );
        dict.insert(
            "duration".into(),
            float(audio_data.get("duration")).map_or(Value::Null, |duration| json!(duration)),
        );
        dict.insert(
            "release_timestamp".into(),
            release_timestamp.map_or(Value::Null, Value::from),
        );
        dict.insert("formats".into(), json!([]));

        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        info.formats = formats;
        info.webpage_url = url.clone();
        info.extractor = WEEKLY_NAME.into();
        info.extractor_key = WEEKLY_KEY.into();
        let mut keys = Vec::new();
        for (key, value) in dict {
            keys.push(key.clone());
            match key.as_str() {
                "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
                "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                "formats" => {}
                _ if value.is_null() => {
                    info.extra.insert(key, Value::Null);
                }
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("dict;{}", keys.join(",")),
        )))
    }
}

pub struct BandcampUser;

impl Extractor for BandcampUser {
    fn key(&self) -> &'static str {
        USER_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !user_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = USER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let uploader = found["id"].to_owned();
        crate::screen::set_extractor(USER_NAME);
        crate::screen::screen(&format!(
            "[{USER_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = fetch_page_in(USER_NAME, client, url, &uploader, "Downloading webpage")?;
        // `playlist_from_matches`: `orderedSet` of the joined URLs.
        let mut targets: Vec<String> = Vec::new();
        for item in user_items(&webpage) {
            let target = url.join(&item)?.to_string();
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        let mut entries = Vec::new();
        for target in &targets {
            entries.push(url_entry(target, None, None, None)?);
        }
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: uploader.clone(),
            title: format!("Discography of {uploader}"),
            webpage_url: url.clone(),
            extractor: USER_NAME.into(),
            extractor_key: USER_KEY.into(),
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
            "http://youtube-dl.bandcamp.com/track/youtube-dl-test-song"
        ));
        assert!(!suitable("https://benprunty.bandcamp.com/album/lanius"));
    }

    #[test]
    fn download_sizes_follow_parse_filesize() {
        assert_eq!(parse_filesize("7.2MB"), Some(7_200_000));
        assert_eq!(parse_filesize("1.5 MiB"), Some(1_572_864));
    }

    #[test]
    fn album_weekly_and_user_urls_follow_upstream_valid_url() {
        assert!(album_suitable(
            "http://blazo.bandcamp.com/album/jazz-format-mixtape-vol-1"
        ));
        assert!(!album_suitable(
            "http://youtube-dl.bandcamp.com/track/youtube-dl-test-song"
        ));
        assert!(weekly_suitable("https://bandcamp.com/radio?show=224"));
        assert!(weekly_suitable(
            "https://bandcamp.com/radio/?foo=bar&show=224"
        ));
        assert!(!weekly_suitable("https://bandcamp.com/radio"));
        assert!(user_suitable("http://dotscale.bandcamp.com"));
        assert!(user_suitable("https://steviasphere.bandcamp.com/music"));
        assert!(!user_suitable("https://www.bandcamp.com"));
        assert!(!user_suitable("http://blazo.bandcamp.com/album/jazz"));
    }

    #[test]
    fn user_items_skip_merch_and_read_the_music_grid() {
        let page = r#"<li data-item-id="a"><a href="/album/one">x</a></li><li data-item-id="b"><a href="/merch/shirt">m</a></li>"#;
        assert_eq!(user_items(page), vec!["/album/one".to_owned()]);
        let grid = r#"<div id="music-grid" data-client-items="[{&quot;page_url&quot;: &quot;/album/two&quot;}]"></div>"#;
        assert_eq!(user_items(grid), vec!["/album/two".to_owned()]);
    }
}
