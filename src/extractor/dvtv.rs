//! Upstream `DVTVIE` (`yt_dlp/extractor/dvtv.py`): a video page's
//! `BBXPlayer.setup(...)` or its `playlist.push(...)` entries, whose HLS,
//! DASH and MP4 tracks become formats.
use super::{Extractor, dict_format, dict_info, direct, download_webpage, nrk};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct DvTv;

const NAME: &str = "dvtv";
const KEY: &str = "DVTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://video\.aktualne\.cz/(?:[^/]+/)+r~(?P<id>[0-9a-f]{32})")
        .unwrap()
});
static PLAYLIST_ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)playlist\.push\((\{.+?\})\);").unwrap());
static BBX_SETUP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)BBXPlayer\.setup\((.+?)\);").unwrap());
/// `re.sub(r'\w+?\((.+)\)', r'\1', item)`: calls around the player data.
static CALL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\w+?\((.+)\)").unwrap());
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)[pP]").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("DVTV {what} are not supported natively"))
}

/// `_parse_video_metadata(js, video_id, timestamp)`.
fn parse_video_metadata(
    page: &Url,
    js: &str,
    video_id: &str,
    timestamp: Option<i64>,
    client: &Client,
) -> Result<VideoInfo> {
    let mut data: serde_json::Map<String, Value> =
        match crate::json::from_str(&crate::js_to_json::js_to_json(js)) {
            Ok(Value::Object(data)) => data,
            Ok(_) => return Err(unsupported("player data that are not objects")),
            Err(error) => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Failed to parse JSON (caused by {error})"
                )));
            }
        };
    let title = match data.get("title") {
        Some(Value::String(title)) => crate::html::unescape_html(title),
        Some(_) => return Err(unsupported("titles that are not strings")),
        None => return Err(super::clyp::key_error(video_id, "title")),
    };

    // `data.update(live_starter)` when the player has a live starter.
    let live_starter = data
        .get("plugins")
        .and_then(|plugins| plugins.get("liveStarter"))
        .and_then(Value::as_object)
        .cloned();
    for (key, value) in live_starter.into_iter().flatten() {
        data.insert(key, value);
    }

    let mut formats = Vec::new();
    let tracks = data
        .get("tracks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|tracks| tracks.values())
        .filter_map(Value::as_array)
        .flatten();
    for video in tracks {
        let Some(video_url) = video
            .get("src")
            .and_then(Value::as_str)
            .filter(|src| !src.is_empty())
        else {
            continue;
        };
        let video_type = video.get("type").and_then(Value::as_str);
        let mime_ext = video_type
            .filter(|mime| !mime.is_empty())
            .map(|mime| direct::mimetype2ext_or(mime, None));
        let ext = direct::determine_ext(video_url)
            .or(mime_ext)
            .unwrap_or_default();
        let page_url =
            Url::parse(video_url).map_err(|_| unsupported("track URLs that do not parse"))?;
        if video_type == Some("application/vnd.apple.mpegurl") || ext == "m3u8" {
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &page_url,
                NAME,
                video_id,
                Some("hls"),
                false,
                false,
            ) {
                formats.extend(stream.formats);
            }
        } else if video_type == Some("application/dash+xml") || ext == "mpd" {
            if let Some(stream) =
                super::extract_mpd_optional(client, &page_url, NAME, video_id, false)
            {
                formats.extend(stream.formats);
            }
        } else {
            let label = video
                .get("label")
                .and_then(Value::as_str)
                .filter(|label| !label.is_empty());
            let height = HEIGHT
                .captures(label.unwrap_or_default())
                .and_then(|found| found[1].parse::<i64>().ok());
            let format_id = match label {
                Some(label) => format!("http-{ext}-{label}"),
                None => format!("http-{ext}"),
            };
            formats.push(dict_format(
                vec![
                    ("url", video_url.into()),
                    ("format_id", format_id.into()),
                    ("height", height.map_or(Value::Null, Value::from)),
                ],
                NAME,
                video_id,
            )?);
        }
    }

    let id = data
        .get("mediaid")
        .filter(|mediaid| super::nuxt::truthy(mediaid))
        .map_or_else(
            || video_id.to_owned(),
            |mediaid| match mediaid {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            },
        );
    Ok(dict_info(
        page,
        (NAME, KEY),
        vec![
            ("id", id.into()),
            ("title", title.into()),
            (
                "description",
                data.get("description").cloned().unwrap_or(Value::Null),
            ),
            (
                "thumbnail",
                data.get("image").cloned().unwrap_or(Value::Null),
            ),
            (
                "duration",
                super::streamable::int_or_none(data.get("duration")),
            ),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
            ("formats", Value::Null),
        ],
        formats,
    ))
}

impl Extractor for DvTv {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let timestamp = crate::html::html_search_meta(&webpage, "article:published_time")
            .and_then(|date| crate::dates::parse_iso8601(&date));

        let items: Vec<&str> = PLAYLIST_ITEM
            .captures_iter(&webpage)
            .map(|found| found.get(1).map_or("", |item| item.as_str()))
            .collect();
        if !items.is_empty() {
            let mut entries = Vec::new();
            for item in items {
                entries.push(parse_video_metadata(
                    url, item, &video_id, timestamp, client,
                )?);
            }
            let title = crate::html::html_search_meta(&webpage, "twitter:title");
            let mut result = nrk::playlist_result(url, NAME, video_id, title, None, entries, true);
            if let ExtractResult::Playlist(playlist) = &mut result {
                playlist.extractor_key = KEY.into();
            }
            return Ok(Some(result));
        }

        if let Some(found) = BBX_SETUP.captures(&webpage) {
            // Upstream drops the calls (`htmldeentitize(...)`) around the data.
            let item = CALL.replace_all(&found[1], "$1");
            let info = parse_video_metadata(url, &item, &video_id, timestamp, client)?;
            return Ok(Some(ExtractResult::Video(info)));
        }

        Err(Error::Reported(format!(
            "[{NAME}] {video_id}: Could not find neither video nor playlist; {}",
            crate::BUG_REPORTS_MESSAGE
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://video.aktualne.cz/dvtv/vondra-o-ceskem-stoleti-pri-pohledu-na-havla-mi-bylo-trapne/r~dc0768de855511e49e4b0025900fea04/"
        ));
        assert!(suitable(
            "http://video.aktualne.cz/v-cechach-poprve-zazni-zelenkova-zrestaurovana-mse/r~45b4b00483ec11e4883b002590604f2e/"
        ));
        assert!(!suitable("https://video.aktualne.cz/dvtv/"));
    }
}
