//! Upstream `LRTStreamIE`, `LRTVODIE` and `LRTRadioIE`
//! (`yt_dlp/extractor/lrt.py`): LRT live channels (the page's escaped
//! `get_streams_url` and its live HLS `content*` URLs), mediateka videos
//! (the page's content or canonical URL, the VOD `media_info` API's JW
//! Player item merged with the API metadata) and radioteka audios (the
//! `rest-api/media` item's HLS playlist and metadata).
use super::{Extractor, dict_info, download_json, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct LrtStream;
pub struct LrtVod;
pub struct LrtRadio;

const STREAM_NAME: &str = "LRTStream";
const VOD_NAME: &str = "LRTVOD";
const RADIO_NAME: &str = "LRTRadio";

static STREAM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?lrt\.lt/mediateka/tiesiogiai/(?P<id>[\w-]+)").unwrap()
});
static VOD_URL: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"^https?://(?:(?:www|archyvai)\.)?lrt\.lt/mediateka/irasas/(?P<id>[0-9]+)")
            .unwrap(),
        Regex::new(r"^https?://(?:(?:www|archyvai)\.)?lrt\.lt/mediateka/video/[^?#]+\?(?:[^#]*&)?episode=(?P<id>[0-9]+)").unwrap(),
    ]
});
static RADIO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?lrt\.lt/radioteka/irasas/(?P<id>\d+)/(?P<path>[^?#/]+)")
        .unwrap()
});
static STREAMS_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\\"get_streams_url\\":\\"([^"]+)\\""#).unwrap());
static CONTENT_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\\"(?:article|data)\\":\{[^}]*\\"url\\":\\"(/[^"]+)\\""#).unwrap()
});
static CANONICAL_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<link\s+rel="canonical"\s*href="(/[^"]+)""#).unwrap());

/// `LRTStreamIE._match_valid_url`.
pub fn stream_suitable(url: &str) -> bool {
    STREAM_URL.is_match(url)
}

/// `LRTVODIE._match_valid_url`.
pub fn vod_suitable(url: &str) -> bool {
    VOD_URL.iter().any(|pattern| pattern.is_match(url))
}

/// `LRTRadioIE._match_valid_url`.
pub fn radio_suitable(url: &str) -> bool {
    RADIO_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// A fatal `_search_regex` miss.
fn bug(name: &str, id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{name}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn with_query(url: &str, key: &str, value: &str) -> Result<Url> {
    let mut url = Url::parse(url)?;
    url.query_pairs_mut().append_pair(key, value);
    Ok(url)
}

/// The shared `traverse_obj(media, {...})` fields: `('tags', ..., 'name',
/// {str})` (dropped when empty) and `('date', {lambda x: x.replace('.',
/// '/')}, {unified_timestamp})`.
fn tags(media: &Value) -> Option<Value> {
    let tags: Vec<Value> = match media.get("tags") {
        Some(Value::Array(tags)) => tags.iter().collect::<Vec<_>>(),
        Some(Value::Object(tags)) => tags.values().collect(),
        _ => Vec::new(),
    }
    .into_iter()
    .filter_map(|tag| tag.get("name").filter(|name| name.is_string()).cloned())
    .collect();
    (!tags.is_empty()).then_some(Value::Array(tags))
}

fn timestamp(media: &Value) -> Option<Value> {
    media
        .get("date")
        .and_then(Value::as_str)
        .and_then(|date| crate::dates::unified_timestamp(&date.replace('.', "/"), false))
        .map(Value::from)
}

fn clean_text(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .map(|text| Value::from(crate::html::clean_html(text)))
}

impl Extractor for LrtStream {
    fn key(&self) -> &'static str {
        STREAM_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = STREAM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(STREAM_NAME, url);
        let webpage = download_webpage(client, url, STREAM_NAME, &id)?;
        let streams_url = STREAMS_URL
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| bug(STREAM_NAME, &id, "stream URL"))?;
        let streams = download_json(client, &Url::parse(&streams_url)?, STREAM_NAME, &id)?;
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        // `('response', 'data', lambda k, _: k.startswith('content'),
        // {url_or_none})`.
        if let Some(Value::Object(data)) = streams
            .get("response")
            .and_then(|response| response.get("data"))
        {
            for (key, value) in data {
                let Some(stream_url) = key
                    .starts_with("content")
                    .then(|| value.as_str().and_then(super::json_ld::url_or_none))
                    .flatten()
                else {
                    continue;
                };
                let manifest = Url::parse(&stream_url)?;
                let stream = super::extract_m3u8_fatal_live(
                    client,
                    &manifest,
                    STREAM_NAME,
                    &id,
                    Some("hls"),
                    true,
                )?;
                formats.extend(stream.formats);
                super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        }
        // `_og_search_title(webpage)` (`fatal=False`).
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            search_warning(STREAM_NAME, "OpenGraph title");
        }
        let mut info = dict_info(
            url,
            (STREAM_NAME, STREAM_NAME),
            vec![
                ("id", Value::from(id.as_str())),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("is_live", Value::Bool(true)),
                ("title", title.map_or(Value::Null, Value::from)),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

impl Extractor for LrtVod {
    fn key(&self) -> &'static str {
        VOD_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VOD_URL
            .iter()
            .find_map(|pattern| pattern.captures(url.as_str()))
        else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(VOD_NAME, url);
        let webpage = download_webpage(client, url, VOD_NAME, &id)?;
        let canonical = match CONTENT_URL.captures(&webpage) {
            Some(found) => found[1].to_owned(),
            None => {
                search_warning(VOD_NAME, "content URL");
                CANONICAL_URL
                    .captures(&webpage)
                    .map(|found| found[1].to_owned())
                    .ok_or_else(|| bug(VOD_NAME, &id, "canonical URL"))?
            }
        };
        let api = with_query(
            "https://www.lrt.lt/servisai/stream_url/vod/media_info/",
            "url",
            &canonical,
        )?;
        let media = download_json(client, &api, VOD_NAME, &id)?;
        let item = media
            .get("playlist_item")
            .ok_or_else(|| key_error(&id, "playlist_item"))?;
        let no_formats = || {
            Error::Reported(format!(
                "[{VOD_NAME}] {id}: No video formats found!; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let Some(mut entry) = super::jwplayer::parse_required(item, &id, url)? else {
            return Err(no_formats());
        };
        // `{**jw_data, **traverse_obj(media, {...})}`.
        let fields: [(&str, Option<Value>); 5] = [
            ("id", media.get("id").filter(|id| id.is_string()).cloned()),
            (
                "title",
                media
                    .get("title")
                    .filter(|title| title.is_string())
                    .cloned(),
            ),
            ("description", clean_text(media.get("content"))),
            ("timestamp", timestamp(&media)),
            ("tags", tags(&media)),
        ];
        for (key, value) in fields {
            if let Some(value) = value {
                entry.insert(key.into(), value);
            }
        }
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .map_or_else(|| id.clone(), str::to_owned);
        let title_missing = entry.get("title").is_none_or(Value::is_null);
        let Some((mut info, keys)) =
            super::html5::jwplayer_entry_info(client, entry, url, &id, VOD_NAME)?
        else {
            return Err(no_formats());
        };
        if title_missing {
            info.title = String::new();
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        info.webpage_url = url.clone();
        info.extractor = VOD_NAME.into();
        info.extractor_key = VOD_NAME.into();
        let shape = format!("jwplatform;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

impl Extractor for LrtRadio {
    fn key(&self) -> &'static str {
        RADIO_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = RADIO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(RADIO_NAME, url);
        let api = with_query(
            "https://www.lrt.lt/rest-api/media",
            "url",
            &format!("/mediateka/irasas/{id}/{}", &found["path"]),
        )?;
        let media = download_json(client, &api, RADIO_NAME, &id)?;
        let item = media
            .get("playlist_item")
            .ok_or_else(|| key_error(&id, "playlist_item"))?;
        let file = item.get("file").ok_or_else(|| key_error(&id, "file"))?;
        let Some(file) = file.as_str() else {
            return Err(Error::UnsupportedHtml5(
                "LRT radio playlist files that are not strings are not supported natively".into(),
            ));
        };
        let manifest = Url::parse(file)?;
        let stream =
            super::extract_m3u8_fatal_master(client, &manifest, RADIO_NAME, &id, None, false)?;
        // `('playlist_item', 'category', {str}, filter, all, filter)`.
        let categories = item
            .get("category")
            .and_then(Value::as_str)
            .filter(|category| !category.is_empty())
            .map(|category| Value::Array(vec![category.into()]));
        let thumbnail = item.get("image").and_then(Value::as_str).and_then(|image| {
            Url::parse("https://www.lrt.lt")
                .ok()?
                .join(image)
                .ok()
                .filter(|url| matches!(url.scheme(), "http" | "https"))
                .map(|url| Value::from(url.as_str()))
        });
        let mut fields = vec![("id", Value::from(id.as_str())), ("formats", Value::Null)];
        let rest: [(&str, Option<Value>); 6] = [
            (
                "title",
                media
                    .get("title")
                    .filter(|title| title.is_string())
                    .cloned(),
            ),
            ("tags", tags(&media)),
            ("categories", categories),
            ("description", clean_text(media.get("content"))),
            ("timestamp", timestamp(&media)),
            ("thumbnail", thumbnail),
        ];
        fields.extend(
            rest.into_iter()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        );
        Ok(Some(dict_info(
            url,
            (RADIO_NAME, RADIO_NAME),
            fields,
            stream.formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(stream_suitable(
            "https://www.lrt.lt/mediateka/tiesiogiai/lrt-opus"
        ));
        assert!(vod_suitable(
            "https://www.lrt.lt/mediateka/irasas/2000127261/greita-ir-gardu"
        ));
        assert!(vod_suitable(
            "https://archyvai.lrt.lt/mediateka/video/ziniu-riteriai-ir-damos?episode=49685&season=%2F2013"
        ));
        assert!(radio_suitable(
            "https://www.lrt.lt/radioteka/irasas/2000359728/nemarios-eiles"
        ));
        assert!(!vod_suitable("https://www.lrt.lt/mediateka/video/x"));
    }
}
