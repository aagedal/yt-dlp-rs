//! Upstream `NprIE` (`yt_dlp/extractor/npr.py`): the story API's `audio`
//! and `multimedia` items as a playlist, each with its known `format`
//! files (HLS, SMIL with the RTMP base rewritten to HTTPS, direct files),
//! its `hlsUrl` stream and, without formats, the page's JSON-LD
//! `subjectOf` HLS URL.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Npr;

const NAME: &str = "Npr";

/// `KNOWN_FORMATS`, also `qualities(KNOWN_FORMATS)`'s order.
const KNOWN_FORMATS: [&str; 5] = ["threegp", "m3u8", "smil", "mp4", "mp3"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?npr\.org/(?:sections/[^/]+/)?\d{4}/\d{2}/\d{2}/(?P<id>\d+)")
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("NPR {what} are not supported natively"))
}

/// `KeyError(key)` raised while extracting.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `obj.get(key, {})` on a dict.
fn get_dict<'a>(
    object: &'a Value,
    key: &str,
    what: &str,
) -> Result<Option<&'a Map<String, Value>>> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(_) => Err(unsupported(what)),
    }
}

/// `obj.get(key, {}).get('$text')`.
fn text<'a>(object: &'a Value, key: &str) -> Result<Option<&'a Value>> {
    Ok(get_dict(object, key, "story fields that are not objects")?
        .and_then(|map| map.get("$text"))
        .filter(|value| !value.is_null()))
}

/// `$text` values used as strings (titles, URLs).
fn text_str(value: Option<&Value>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(unsupported("non-string story texts")),
    }
}

/// `_extract_smil_formats(url, media_id, transform_source=..., fatal=False)`
/// with `_check_formats`.
fn smil_formats(client: &Client, url: &str, id: &str) -> Result<Vec<VideoFormat>> {
    use std::io::Read;
    let smil_url = Url::parse(url).map_err(|_| unsupported("relative SMIL URLs"))?;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading SMIL file"));
    let mut response = match super::direct::request(client, &smil_url, None) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download SMIL file: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
    };
    let final_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers()).replace(
        "rtmp://flash.npr.org/ondemand/",
        "https://ondemand.npr.org/",
    );
    let smil =
        super::smil::parse(&text).ok_or_else(|| unsupported("SMIL files that are not SMIL"))?;
    let (mut formats, subtitles) =
        super::html5::smil_formats_and_subtitles(client, &smil, &final_url, id, NAME)?;
    if !subtitles.is_empty() {
        crate::screen::warning_once(&format!(
            "[{NAME}] Ignoring subtitle tracks found in the SMIL manifest; if any subtitle tracks are missing, {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    super::check_formats(client, &mut formats, NAME, id)?;
    Ok(formats)
}

/// `_extract_m3u8_formats(url, id, 'mp4', ..., m3u8_id='hls', fatal=False)`.
fn m3u8_formats(client: &Client, url: &str, id: &str) -> Result<Vec<VideoFormat>> {
    let manifest = Url::parse(url).map_err(|_| unsupported("relative HLS URLs"))?;
    let Some(stream) =
        super::extract_m3u8_master_optional(client, &manifest, NAME, id, Some("hls"), false, false)
    else {
        return Ok(Vec::new());
    };
    let mut formats = stream.formats;
    for format in &mut formats {
        format.ext = "mp4".into();
    }
    Ok(formats)
}

/// The media's formats from its `format` and `stream` dicts.
fn media_formats(client: &Client, media: &Value, media_id: &str) -> Result<Vec<VideoFormat>> {
    let mut formats = Vec::new();
    let entries = get_dict(media, "format", "media formats that are not objects")?;
    for (format_id, entry) in entries.into_iter().flatten() {
        if !super::nuxt::truthy(entry) {
            continue;
        }
        let entry = match entry {
            Value::Array(items) => &items[0],
            entry => entry,
        };
        let Value::Object(entry) = entry else {
            return Err(unsupported("media format entries that are not objects"));
        };
        let Some(format_url) = text_str(entry.get("$text").filter(|value| !value.is_null()))?
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let Some(quality) = KNOWN_FORMATS.iter().position(|known| known == format_id) else {
            continue;
        };
        match format_id.as_str() {
            "m3u8" => formats.extend(m3u8_formats(client, &format_url, media_id)?),
            "smil" => formats.extend(smil_formats(client, &format_url, media_id)?),
            _ => formats.push(dict_format(
                vec![
                    ("url", format_url.into()),
                    ("format_id", format_id.as_str().into()),
                    ("quality", Value::from(quality)),
                ],
                NAME,
                media_id,
            )?),
        }
    }
    let streams = get_dict(media, "stream", "media streams that are not objects")?;
    for (stream_id, entry) in streams.into_iter().flatten() {
        if !entry.is_object() || stream_id != "hlsUrl" {
            continue;
        }
        let Some(stream_url) = entry
            .get("$text")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        // Upstream passes the stream's key as the video ID.
        formats.extend(m3u8_formats(client, &stream_url, stream_id)?);
    }
    Ok(formats)
}

/// Without formats: `traverse_obj(json_ld, (..., 'subjectOf', ...,
/// 'embedUrl'), get_all=False)` of the page and its HLS formats.
fn json_ld_formats(
    client: &Client,
    url: &Url,
    playlist_id: &str,
    media_id: &str,
) -> Result<Vec<VideoFormat>> {
    let webpage = download_webpage(client, url, NAME, playlist_id)?;
    let embed = super::json_ld::yield_json_ld(&webpage)
        .iter()
        .flat_map(|item| match item.get("subjectOf") {
            Some(Value::Array(items)) => items.iter().collect::<Vec<_>>(),
            Some(Value::Object(map)) => map.values().collect(),
            _ => Vec::new(),
        })
        .find_map(|subject| subject.get("embedUrl").filter(|value| !value.is_null()))
        .cloned();
    let m3u8_url = match embed {
        None => {
            crate::screen::warning(&format!(
                "[{NAME}] Failed to obtain m3u8 URL; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
            return Ok(Vec::new());
        }
        Some(Value::String(text)) if text.is_empty() => {
            crate::screen::warning(&format!(
                "[{NAME}] Failed to obtain m3u8 URL; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
            return Ok(Vec::new());
        }
        Some(Value::String(text)) => text,
        Some(_) => return Err(unsupported("non-string JSON-LD embed URLs")),
    };
    m3u8_formats(client, &m3u8_url, media_id)
}

impl Extractor for Npr {
    fn key(&self) -> &'static str {
        NAME
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
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("id", &playlist_id)
            .append_pair("fields", "audio,multimedia,title")
            .append_pair("format", "json")
            .append_pair("apiKey", "MDAzMzQ2MjAyMDEyMzk4MTU1MDg3ZmM3MQ010")
            .finish();
        let api = Url::parse(&format!("http://api.npr.org/query?{query}"))?;
        let response = download_json(client, &api, NAME, &playlist_id)?;
        let story = response
            .get("list")
            .ok_or_else(|| key_error(&playlist_id, "list"))?
            .get("story")
            .ok_or_else(|| key_error(&playlist_id, "story"))?
            .get(0)
            .ok_or_else(|| unsupported("stories without items"))?;
        let playlist_title = text_str(text(story, "title")?)?;

        let list = |key: &str| -> Result<Vec<Value>> {
            match story.get(key) {
                None => Ok(Vec::new()),
                Some(Value::Array(items)) => Ok(items.clone()),
                Some(_) => Err(unsupported("story media that are not lists")),
            }
        };
        let mut entries = Vec::new();
        for media in list("audio")?.into_iter().chain(list("multimedia")?) {
            let media_id = match media.get("id") {
                None => return Err(key_error(&playlist_id, "id")),
                Some(Value::String(id)) => id.clone(),
                Some(_) => return Err(unsupported("non-string media IDs")),
            };
            let mut formats = media_formats(client, &media, &media_id)?;
            if formats.is_empty() {
                formats = json_ld_formats(client, url, &playlist_id, &media_id)?;
            }
            let title = text_str(text(&media, "title")?)?
                .filter(|title| !title.is_empty())
                .or_else(|| playlist_title.clone());
            let thumbnail = text(&media, "altImageUrl")?.cloned();
            let duration = super::streamable::int_or_none(text(&media, "duration")?);
            entries.push(dict_info(
                url,
                (NAME, NAME),
                vec![
                    ("id", media_id.into()),
                    ("title", title.map_or(Value::Null, Value::from)),
                    ("thumbnail", thumbnail.unwrap_or(Value::Null)),
                    ("duration", duration),
                    ("formats", Value::Null),
                ],
                formats,
            ));
        }

        // `playlist_result(entries, playlist_id, playlist_title)`.
        let mut extra = Map::new();
        let title = playlist_title.filter(|title| !title.is_empty());
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            if title.is_some() {
                "playlist-dict;id,title"
            } else {
                "playlist-dict;id"
            }
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
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
            "https://www.npr.org/sections/allsongs/2015/10/21/449974205/new-music-from-beach-house"
        ));
        assert!(suitable(
            "https://www.npr.org/2017/06/19/533198237/tigers-jaw-tiny-desk-concert"
        ));
        assert!(!suitable(
            "https://www.npr.org/podcasts/510289/planet-money"
        ));
    }

    #[test]
    fn smil_sources_join_like_pythons_urljoin() {
        use super::super::html5::py_urljoin;
        // `urljoin('https://ondemand.npr.org//', 'anon/x.mp4')`.
        assert_eq!(
            py_urljoin("https://ondemand.npr.org//", "anon/x.mp4").as_deref(),
            Some("https://ondemand.npr.org/anon/x.mp4")
        );
        assert_eq!(
            py_urljoin("https://a.test/p//q/", "../x?a=b").as_deref(),
            Some("https://a.test/p/x?a=b")
        );
        // Absolute paths and URLs are kept.
        assert_eq!(
            py_urljoin("https://a.test/b/", "//c.test//d").as_deref(),
            Some("https://c.test//d")
        );
        assert_eq!(
            py_urljoin("https://a.test/b/", "/c//d").as_deref(),
            Some("https://a.test/c//d")
        );
    }
}
