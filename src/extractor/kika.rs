//! Upstream `KikaIE` and `KikaPlaylistIE` (`yt_dlp/extractor/kika.py`):
//! KiKA videos from the `_next-api` video document and its assets (HLS
//! master playlists and progressive files, TTML/WebVTT subtitles when the
//! document has them), and brands as a generator of `url` results read one
//! `videosPageUrl` page at a time.
use super::{Extractor, dict_format, dict_info, download_json, download_json_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, SubtitleFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Kika;
pub struct KikaPlaylist;

const NAME: &str = "Kika";
const PLAYLIST_NAME: &str = "KikaPlaylist";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?kika\.de/[\w/-]+/videos/(?P<id>[a-z-]+\d+)")
        .unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?kika\.de/[\w-]+/(?P<id>[a-z-]+\d+)").unwrap()
});

/// `KikaIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `KikaPlaylistIE._match_id`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("KiKA {what} are not supported natively"))
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

/// `traverse_obj(value, (key, {str}))`.
fn str_value(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

/// `traverse_obj(value, (key, {parse_iso8601}))`.
fn iso8601(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
}

fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `_extract_formats`: assets whose `url` is a URL.
fn formats(client: &Client, assets: &Value, video_id: &str) -> Result<Vec<crate::VideoFormat>> {
    let mut formats = Vec::new();
    let items: Vec<&Value> = match assets.get("assets") {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    for media in items {
        let Some(stream_url) = url_or_none(media.get("url")) else {
            continue;
        };
        let ext = super::direct::determine_ext(&stream_url);
        if ext.as_deref() == Some("m3u8") {
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &Url::parse(&stream_url)?,
                NAME,
                video_id,
                Some("hls"),
                false,
                false,
            ) {
                formats.extend(stream.formats);
            }
            continue;
        }
        let mut fields = vec![
            ("url", Value::from(stream_url.clone())),
            (
                "format_id",
                Value::from(ext.unwrap_or_else(|| "unknown_video".into())),
            ),
        ];
        let width = int_or_none(media.get("frameWidth"));
        let height = int_or_none(media.get("frameHeight"));
        // `filesize` is 0 if unknown, bitrates -1.
        let filesize = int_or_none(media.get("fileSize")).filter(|size| *size != 0);
        let abr = int_or_none(media.get("bitrateAudio")).filter(|rate| *rate != -1);
        let vbr = int_or_none(media.get("bitrateVideo")).filter(|rate| *rate != -1);
        for (key, value) in [
            ("width", width),
            ("height", height),
            ("filesize", filesize),
            ("abr", abr),
            ("vbr", vbr),
        ] {
            if let Some(value) = value {
                fields.push((key, value.into()));
            }
        }
        let mut format = dict_format(fields, NAME, video_id)?;
        // `int_or_none` bitrates stay integers.
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    Ok(formats)
}

impl Extractor for Kika {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_GEO_COUNTRIES = ['DE']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["DE"])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);
        let api = Url::parse(&format!(
            "https://www.kika.de/_next-api/proxy/v1/videos/{video_id}"
        ))?;
        let doc = download_json(client, &api, NAME, &video_id)?;
        let assets_url = doc
            .get("assets")
            .ok_or_else(|| key_error(&video_id, "assets"))?
            .get("url")
            .ok_or_else(|| key_error(&video_id, "url"))?
            .as_str()
            .ok_or_else(|| unsupported("asset URLs that are not strings"))?;
        let assets_url =
            Url::parse(assets_url).map_err(|_| unsupported("asset URLs that are not URLs"))?;
        let assets = download_json(client, &assets_url, NAME, &video_id)?;
        let mut subtitles = crate::Subtitles::default();
        if doc.get("hasSubtitle").is_some_and(super::nuxt::truthy) {
            let mut tracks = Vec::new();
            for (key, ext) in [("videoSubtitle", "ttml"), ("webvttUrl", "vtt")] {
                if let Some(resource) = url_or_none(assets.get(key)) {
                    tracks.push(SubtitleFormat {
                        name: None,
                        url: Url::parse(&resource)?,
                        ext: ext.into(),
                        protocol: None,
                        fragments: Vec::new(),
                        extra: Map::new(),
                    });
                }
            }
            if !tracks.is_empty() {
                subtitles.entries.push((Some("de".into()), tracks));
            }
        }
        let formats = formats(client, &assets, &video_id)?;
        let duration = int_or_none(doc.get("durationInSeconds"))
            .map(Value::from)
            .or_else(|| {
                doc.get("duration")
                    .and_then(Value::as_str)
                    .and_then(crate::parse_duration)
                    .map(|duration| json!(duration))
            });
        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        for (key, value) in [
            ("title", str_value(doc.get("title")).map(Value::from)),
            (
                "description",
                str_value(doc.get("description")).map(Value::from),
            ),
            ("timestamp", iso8601(doc.get("date")).map(Value::from)),
            (
                "modified_timestamp",
                iso8601(doc.get("modificationDate")).map(Value::from),
            ),
            ("duration", duration),
            (
                "episode_number",
                int_or_none(doc.get("episodeNumber")).map(Value::from),
            ),
            (
                "season_number",
                int_or_none(doc.get("season")).map(Value::from),
            ),
        ] {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";subtitle=url,ext");
        }
        Ok(Some(info))
    }
}

/// `url_result(url, ie=KikaIE, **fields)`.
fn url_entry(target: &str, fields: Vec<(&str, Value)>) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // Without an `id` field, a flat entry's ID is KikaIE's temporary ID.
    info.id = VALID_URL
        .captures(target)
        .map(|found| found["id"].to_owned())
        .unwrap_or_default();
    let mut entry = Map::new();
    for (key, value) in fields {
        if key == "id"
            && let Some(id) = value.as_str()
        {
            info.id = id.to_owned();
        }
        entry.insert(key.into(), value);
    }
    entry.insert("ie_key".into(), NAME.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// Whether a playlist's pending state is a KiKA brand's videos.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == PLAYLIST_NAME
}

/// `KikaPlaylistIE._entries`: one page at a time (`Downloading page N`),
/// each page's items with an `api.url` as `url` results, up to `limit`.
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let Value::Object(mut state) = state else {
        return Ok(None);
    };
    let playlist_id = state["playlist_id"].as_str().unwrap_or_default().to_owned();
    let mut queue: std::collections::VecDeque<Value> = match state.shift_remove("queue") {
        Some(Value::Array(queue)) => queue.into(),
        _ => Default::default(),
    };
    let mut page = state["page"].as_u64().unwrap_or(0);
    let mut next = state["next"].as_str().map(str::to_owned);
    loop {
        if limit.is_some_and(|limit| entries.len() >= limit) {
            break;
        }
        if let Some(item) = queue.pop_front() {
            let Some(target) = url_or_none(item.get("api").and_then(|api| api.get("url"))) else {
                continue;
            };
            let mut fields = Vec::new();
            for (key, value) in [
                ("id", str_value(item.get("id")).map(Value::from)),
                ("title", str_value(item.get("title")).map(Value::from)),
                (
                    "duration",
                    int_or_none(item.get("duration")).map(Value::from),
                ),
                ("timestamp", iso8601(item.get("date")).map(Value::from)),
            ] {
                if let Some(value) = value {
                    fields.push((key, value));
                }
            }
            entries.push(url_entry(&target, fields)?);
            continue;
        }
        let Some(page_url) = next.take() else {
            break;
        };
        page += 1;
        let data = download_json_note(
            client,
            &Url::parse(&page_url)?,
            PLAYLIST_NAME,
            &playlist_id,
            &format!("Downloading page {page}"),
        )?;
        if let Some(Value::Array(items)) = data.get("content") {
            queue.extend(items.iter().cloned());
        }
        next = url_or_none(data.get("links").and_then(|links| links.get("next")));
    }
    if queue.is_empty() && next.is_none() {
        return Ok(None);
    }
    state.insert("queue".into(), Value::Array(queue.into()));
    state.insert("page".into(), page.into());
    state.insert("next".into(), next.map_or(Value::Null, Value::from));
    Ok(Some(Value::Object(state)))
}

impl Extractor for KikaPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        start(PLAYLIST_NAME, url);
        let api = Url::parse(&format!(
            "https://www.kika.de/_next-api/proxy/v1/brands/{playlist_id}"
        ))?;
        let brand = download_json(client, &api, PLAYLIST_NAME, &playlist_id)?;
        let first = brand
            .get("videoSubchannel")
            .ok_or_else(|| key_error(&playlist_id, "videoSubchannel"))?
            .get("videosPageUrl")
            .ok_or_else(|| key_error(&playlist_id, "videosPageUrl"))?
            .as_str()
            .ok_or_else(|| unsupported("video page URLs that are not strings"))?
            .to_owned();
        // `playlist_result(entries, playlist_id, title=..., description=...)`:
        // the keyword fields first (`None` values kept), then the ID.
        let title = brand.get("title").cloned().unwrap_or(Value::Null);
        let mut extra = Map::new();
        if !title.is_string() {
            extra.insert("title".into(), title.clone());
        }
        extra.insert(
            "description".into(),
            brand.get("description").cloned().unwrap_or(Value::Null),
        );
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;title,description,id{}",
                crate::info_order::GENERATOR
            )
            .into(),
        );
        let state = json!({
            "kind": PLAYLIST_NAME,
            "playlist_id": playlist_id,
            "next": first,
            "page": 0,
        });
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.as_str().unwrap_or_default().to_owned(),
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_NAME.into(),
            nested: Vec::new(),
            pending: Some(crate::PendingEntries { state }),
            entries: Vec::new(),
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
            "https://www.kika.de/logo/videos/logo-vom-dienstag-achtundzwanzig-oktober-zweitausendfuenfundzwanzig-100"
        ));
        assert!(suitable("https://www.kika.de/kaltstart/videos/video92498"));
        assert!(!suitable(
            "https://www.kika.de/logo/logo-die-welt-und-ich-562"
        ));
        assert!(playlist_suitable(
            "https://www.kika.de/logo/logo-die-welt-und-ich-562"
        ));
        assert!(!playlist_suitable(
            "https://www.kika.de/kaltstart/videos/video92498"
        ));
    }
}
