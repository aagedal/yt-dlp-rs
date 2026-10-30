//! Upstream `CuriosityStreamIE`, `CuriosityStreamCollectionsIE` and
//! `CuriosityStreamSeriesIE` (`yt_dlp/extractor/curiositystream.py`): a
//! video's HLS and DASH master playlists and direct encodings from the media
//! API, and a collection's or series' media as url results. Login and the
//! `auth_token` cookie are not used, so the anonymous API responses are the
//! only input; RTMP encodings are not supported natively.
use super::{Extractor, dict_format, dict_info, download_json, update_url_query};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct CuriosityStream;
pub struct CuriosityStreamCollections;
pub struct CuriosityStreamSeries;

/// `IE_NAME` of the video extractor, used in messages and `extractor`.
const NAME: &str = "curiositystream";
const COLLECTIONS_NAME: &str = "curiositystream:collections";
const SERIES_NAME: &str = "curiositystream:series";
const KEY: &str = "CuriosityStream";
const COLLECTIONS_KEY: &str = "CuriosityStreamCollections";
const SERIES_KEY: &str = "CuriosityStreamSeries";
const MEDIA_API: &str = "https://api.curiositystream.com/v1/media/";
const COLLECTIONS_API: &str = "https://api.curiositystream.com/v2/collections/";
const SERIES_API: &str = "https://api.curiositystream.com/v2/series/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:app\.)?curiositystream\.com/video/(?P<id>\d+)").unwrap()
});
static COLLECTIONS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:app\.)?curiositystream\.com/collections/(?P<id>\d+)").unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:app\.)?curiositystream\.com/(?:series|collection)/(?P<id>\d+)")
        .unwrap()
});
static RTMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^rtmpe?://[^/]+/.+/mp[34]:.+$").unwrap());

/// `_match_id` of CuriosityStreamIE.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id` of CuriosityStreamCollectionsIE.
pub fn collections_suitable(url: &str) -> bool {
    COLLECTIONS_URL.is_match(url)
}

/// `_match_id` of CuriosityStreamSeriesIE.
pub fn series_suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
}

/// `KeyError(key)` escaping `_real_extract`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("CuriosityStream {what} are not supported natively"))
}

/// Python's `str(value)` for the scalar IDs the API uses.
fn py_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".into(),
        Value::Bool(flag) => if *flag { "True" } else { "False" }.into(),
        other => other.to_string(),
    }
}

/// Python truthiness of a JSON value.
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

/// `CuriosityStreamBaseIE._handle_errors`: `error.message` (a dict's values
/// joined) raises `IE_NAME said: MESSAGE` (expected).
fn handle_errors(response: &Value, name: &'static str, id: &str) -> Result<()> {
    let message = match response.get("error").and_then(|error| error.get("message")) {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::Object(map)) => map.values().map(py_str).collect::<Vec<_>>().join(", "),
        Some(value) if truthy(Some(value)) => py_str(value),
        Some(_) => return Ok(()),
    };
    if message.is_empty() {
        return Ok(());
    }
    Err(Error::Reported(format!(
        "[{name}] {id}: {name} said: {message}"
    )))
}

/// `_call_api(path, video_id, query)`: the response's `data`.
fn call_api(client: &Client, url: &Url, name: &'static str, id: &str) -> Result<Value> {
    let response = download_json(client, url, name, id)?;
    handle_errors(&response, name, id)?;
    response
        .get("data")
        .cloned()
        .ok_or_else(|| key_error(id, "data"))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// The `url_result(target, ie=ie_key, video_id=id)` playlist entry.
fn url_entry(target: &str, ie_key: &str, extractor: &str, id: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = id.to_owned();
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ie_key.into());
    entry.insert("id".into(), id.into());
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

/// The formats of one `encodings` entry's direct `url` and `file_url`.
fn direct_formats(encoding: &Value, id: &str) -> Result<Vec<crate::VideoFormat>> {
    let text = |key: &str| {
        encoding
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let rate = |key: &str| super::streamable::int_or_none(encoding.get(key));
    let common = [
        (
            "width",
            super::streamable::int_or_none(encoding.get("width")),
        ),
        (
            "height",
            super::streamable::int_or_none(encoding.get("height")),
        ),
        ("vbr", rate("video_bitrate")),
        ("abr", rate("audio_bitrate")),
        ("filesize", rate("size_in_bytes")),
    ];
    let codec = |key: &str| encoding.get(key).cloned().unwrap_or(Value::Null);
    let mut formats = Vec::new();
    for f_url in [text("url"), text("file_url")] {
        if f_url.is_empty() {
            continue;
        }
        if RTMP.is_match(f_url) {
            return Err(unsupported("RTMP encodings"));
        }
        let mut fields: Vec<(&str, Value)> = common.to_vec();
        fields.extend([
            ("vcodec", codec("video_codec")),
            ("acodec", codec("audio_codec")),
            ("container", codec("container_type")),
            ("url", f_url.into()),
            ("format_id", "http".into()),
        ]);
        let mut format = dict_format(fields, NAME, id)?;
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    Ok(formats)
}

/// `subtitles` from `closed_captions`: `{lang: [{url}]}` in first-seen order.
fn subtitles(media: &Value) -> Result<Map<String, Value>> {
    let mut subtitles = Map::new();
    let Some(captions) = media.get("closed_captions").and_then(Value::as_array) else {
        return Ok(subtitles);
    };
    for caption in captions {
        let Some(sub_url) = caption.get("file").and_then(Value::as_str) else {
            continue;
        };
        if sub_url.is_empty() {
            continue;
        }
        let lang = [caption.get("code"), caption.get("language")]
            .into_iter()
            .flatten()
            .find(|value| truthy(Some(*value)))
            .map_or_else(|| "en".to_owned(), py_str);
        let entry = subtitles
            .entry(lang)
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(items) = entry {
            items.push(json!({"url": sub_url}));
        }
    }
    Ok(subtitles)
}

impl Extractor for CuriosityStream {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(NAME, url);
        let mut formats = Vec::new();
        // The last response's `title`, as upstream's loop variable.
        let mut media = Value::Null;
        for encoding_format in ["m3u8", "mpd"] {
            let api = update_url_query(
                &Url::parse(&format!("{MEDIA_API}{video_id}"))?,
                &[
                    ("encodingsNew", "true"),
                    ("encodingsFormat", encoding_format),
                ],
            );
            media = call_api(client, &api, NAME, &video_id)?;
            let encodings = match media.get("encodings") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(encodings)) => encodings.clone(),
                Some(_) => return Err(unsupported("encodings that are not lists")),
            };
            for encoding in &encodings {
                if !encoding.is_object() {
                    return Err(unsupported("encodings that are not objects"));
                }
                let playlist = encoding
                    .get("master_playlist_url")
                    .and_then(Value::as_str)
                    .filter(|playlist| !playlist.is_empty());
                if let Some(playlist) = playlist {
                    let manifest = Url::parse(playlist)
                        .map_err(|_| unsupported("manifest URLs that are not URLs"))?;
                    if encoding_format == "m3u8" {
                        if let Some(stream) = super::extract_m3u8_master_optional(
                            client,
                            &manifest,
                            NAME,
                            &video_id,
                            Some("hls"),
                            false,
                            false,
                        ) {
                            formats.extend(stream.formats);
                        }
                    } else if let Some(mut stream) = super::extract_mpd_optional_note(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some("dash"),
                        "Downloading MPD manifest",
                    ) {
                        // `_extract_mpd_formats` drops the manifest's subtitles.
                        super::ignore_subtitles(&mut stream, NAME, "DASH");
                        formats.extend(stream.formats);
                    }
                }
                formats.extend(direct_formats(encoding, &video_id)?);
            }
        }

        let title = media
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "title"))?;
        let get = |key: &str| media.get(key).cloned().unwrap_or(Value::Null);
        let first_truthy = |keys: &[&str]| {
            keys.iter()
                .map(|key| get(key))
                .find(|value| truthy(Some(value)))
                .unwrap_or(Value::Null)
        };
        let collection_id = match media.get("collection_id") {
            Some(value) if truthy(Some(value)) => Value::String(py_str(value)),
            _ => Value::Null,
        };
        let categories = Value::Array(vec![get("primary_category"), get("type")]);
        let subtitles = Value::Object(subtitles(&media)?);
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.clone().into()),
                ("formats", Value::Null),
                ("title", title),
                ("description", get("description")),
                (
                    "thumbnail",
                    first_truthy(&["image_large", "image_medium", "image_small"]),
                ),
                (
                    "duration",
                    super::streamable::int_or_none(media.get("duration")),
                ),
                ("tags", get("tags")),
                ("subtitles", subtitles),
                ("channel", get("producer")),
                ("categories", categories),
                ("average_rating", get("rating_percentage")),
                ("series_id", collection_id),
            ],
            formats,
        )))
    }
}

/// `CuriosityStreamCollectionBaseIE._real_extract` for a collection or series.
fn collection_result(
    client: &Client,
    url: &Url,
    (name, key, api): (&'static str, &'static str, &str),
    ids: &Regex,
) -> Result<Option<ExtractResult>> {
    let Some(found) = ids.captures(url.as_str()) else {
        return Ok(None);
    };
    let collection_id = found["id"].to_owned();
    announce(name, url);
    let collection = call_api(
        client,
        &Url::parse(&format!("{api}{collection_id}"))?,
        name,
        &collection_id,
    )?;
    let mut entries = Vec::new();
    if let Some(media_list) = collection.get("media").and_then(Value::as_array) {
        for media in media_list {
            let media_id = py_str(media.get("id").unwrap_or(&Value::Null));
            let (media_type, ie_key, ie_name) = if truthy(media.get("is_collection")) {
                ("series", SERIES_KEY, SERIES_NAME)
            } else {
                ("video", KEY, NAME)
            };
            let target = format!("https://curiositystream.com/{media_type}/{media_id}");
            entries.push(url_entry(&target, ie_key, ie_name, &media_id)?);
        }
    }
    // `playlist_result(entries, collection_id, title, description)`.
    let title = collection
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.is_empty())
        .map(str::to_owned);
    let description = collection
        .get("description")
        .filter(|value| !value.is_null());
    let mut keys = vec!["id"];
    if title.is_some() {
        keys.push("title");
    }
    let mut extra = Map::new();
    if let Some(description) = description {
        keys.push("description");
        extra.insert("description".into(), description.clone());
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    Ok(Some(ExtractResult::Playlist(PlaylistInfo {
        id: collection_id,
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })))
}

impl Extractor for CuriosityStreamCollections {
    fn key(&self) -> &'static str {
        COLLECTIONS_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        collection_result(
            client,
            url,
            (COLLECTIONS_NAME, COLLECTIONS_KEY, COLLECTIONS_API),
            &COLLECTIONS_URL,
        )
    }
}

impl Extractor for CuriosityStreamSeries {
    fn key(&self) -> &'static str {
        SERIES_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        collection_result(
            client,
            url,
            (SERIES_NAME, SERIES_KEY, SERIES_API),
            &SERIES_URL,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://app.curiositystream.com/video/2"));
        assert!(suitable("https://curiositystream.com/video/2"));
        assert!(!suitable("https://curiositystream.com/series/2"));
        assert!(collections_suitable(
            "https://curiositystream.com/collections/86"
        ));
        assert!(series_suitable("https://curiositystream.com/series/2"));
        assert!(series_suitable("https://curiositystream.com/collection/2"));
        assert!(!series_suitable(
            "https://curiositystream.com/collections/86"
        ));
    }
}
