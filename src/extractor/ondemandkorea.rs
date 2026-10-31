//! Upstream `OnDemandKoreaIE` (`yt_dlp/extractor/ondemandkorea.py`): an
//! episode's playback API (`odkmedia.io`, 403 and 404 answers read as
//! JSON) and its HLS manifests, preferring the `_1080p` variant of a `_720p`
//! manifest when it exists. The program page (`OnDemandKoreaProgramIE`):
//! its episodes API pages as `OnDemandKorea` url entries with their titles.
use super::{
    Extractor, dict_info, direct, download_json_note_headers, extract_m3u8_master_optional,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct OnDemandKorea;
pub struct OnDemandKoreaProgram;

const NAME: &str = "OnDemandKorea";
const PROGRAM_NAME: &str = "OnDemandKoreaProgram";
/// `OnDemandKoreaProgramIE._PAGE_SIZE`.
const PROGRAM_PAGE_SIZE: usize = 100;
/// `str(uuid.uuid4())`: the port has no UUID generator, so the device ID is
/// fixed (the playback API only uses it for ad targeting).
const DEVICE_ID: &str = "00000000-0000-4000-8000-000000000000";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"https?://(?:www\.)?ondemandkorea\.com/(?:en/)?player/vod/[a-z0-9-]+\?(?:[^#]+&)?contentId=(?P<id>\d+)",
    )
    .unwrap()
});
static PROGRAM_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"https?://(?:www\.)?ondemandkorea\.com/(?:en/)?player/vod/(?P<id>[a-z0-9-]+)(?:$|#)",
    )
    .unwrap()
});
static QUALITY_720: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_720(p?)\.m3u8").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `OnDemandKoreaProgramIE._VALID_URL`.
pub fn program_suitable(url: &str) -> bool {
    PROGRAM_URL.is_match(url)
}

/// `url_result(target, ie=OnDemandKoreaIE, video_title=title)` as a playlist entry.
fn program_entry(target: &str, title: Option<&str>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), NAME.into());
    if let Some(title) = title {
        entry.insert("title".into(), title.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": PROGRAM_NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for OnDemandKoreaProgram {
    fn key(&self) -> &'static str {
        PROGRAM_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PROGRAM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(PROGRAM_NAME);
        crate::screen::screen(&format!(
            "[{PROGRAM_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        // `OnDemandPagedList`: pages are fetched until one has fewer than
        // `_PAGE_SIZE` results (`page` is 1-based).
        let mut entries = Vec::new();
        for page in 1.. {
            let api = Url::parse_with_params(
                &format!("https://odkmedia.io/odx/api/v3/program/{display_id}/episodes/"),
                &[
                    ("page", page.to_string()),
                    ("page_size", PROGRAM_PAGE_SIZE.to_string()),
                ],
            )?;
            let data = download_json_note_headers(
                client,
                &api,
                PROGRAM_NAME,
                &display_id,
                &format!("Downloading page {page}"),
                &[("service-name", "odk")],
            )?;
            let results = data
                .pointer("/result/results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for episode in &results {
                let episode_id = match episode.get("id") {
                    Some(Value::String(text)) => text.clone(),
                    Some(Value::Number(number)) => number.to_string(),
                    _ => {
                        return Err(super::clyp::key_error(&display_id, "id"));
                    }
                };
                let title = episode.get("title").and_then(Value::as_str);
                entries.push(program_entry(
                    &format!(
                        "https://www.ondemandkorea.com/player/vod/{display_id}?contentId={episode_id}"
                    ),
                    title,
                )?);
            }
            if results.len() < PROGRAM_PAGE_SIZE {
                break;
            }
        }

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: PROGRAM_NAME.into(),
            extractor_key: PROGRAM_NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("OnDemandKorea {what} are not supported natively"))
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `_download_json(..., fatal=False, expected_status=(403, 404))`: the body
/// of an expected error status is still read, a failure is a warning.
fn playback(client: &Client, video_id: &str) -> Result<Option<Value>> {
    use std::io::Read;
    let url = Url::parse_with_params(
        &format!("https://odkmedia.io/odx/api/v3/playback/{video_id}/"),
        &[("did", DEVICE_ID)],
    )?;
    crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading JSON metadata"));
    let mut response =
        match direct::request_expecting(client, &url, &[("service-name", "odk")], &[403, 404]) {
            Ok(response) => response,
            Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
                let text = super::request_error_text(NAME, video_id, "JSON metadata", &error);
                crate::screen::warning(&text);
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(video_id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, response.headers());
    match crate::json::from_str::<Value>(&text) {
        Ok(data) => Ok(Some(data)),
        Err(_) => {
            crate::screen::warning(&format!("[{NAME}] {video_id}: Failed to parse JSON"));
            Ok(None)
        }
    }
}

/// `try_geo_bypass`: the `stream_url` of a tokenised source URL, if any.
fn geo_bypass(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .query_pairs()
                .find(|(key, _)| key == "stream_url")
                .map(|(_, value)| value.into_owned())
        })
        .and_then(|stream| super::json_ld::url_or_none(&stream))
        .unwrap_or_else(|| url.to_owned())
}

/// `traverse_obj(data, ('text_tracks', ...))` with a `url` (only tracks
/// without URLs are dropped; a track with one is a subtitle entry).
fn has_text_tracks(data: &Value) -> bool {
    data.get("text_tracks")
        .and_then(Value::as_array)
        .is_some_and(|tracks| {
            tracks.iter().any(|track| {
                track
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .is_some()
            })
        })
}

/// `join_nonempty(*paths, from_dict=data, delim=': ')`.
fn joined_title(data: &Value) -> String {
    let title_paths: [&[&str]; 2] = [&["episode", "program", "title"], &["episode", "title"]];
    title_paths
        .iter()
        .filter_map(|path| {
            path.iter()
                .try_fold(data, |value, key| value.get(*key))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
        })
        .collect::<Vec<_>>()
        .join(": ")
}

/// `parse_age_limit` for the labels the playback API gives (`All`, digits).
fn age_limit(name: &str) -> Result<Option<i64>> {
    let label = name.replace('R', "");
    if label.is_empty() || label == "All" {
        return Ok(None);
    }
    let digits = label.trim_end_matches('+');
    match digits.parse::<i64>() {
        Ok(limit) => Ok(Some(limit)),
        Err(_) => Err(unsupported("age ratings that are not numeric")),
    }
}

impl Extractor for OnDemandKorea {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = found["id"].to_owned();
        let response = playback(client, &video_id)?;
        let Some(data) = response
            .as_ref()
            .and_then(|response| response.get("result"))
            .filter(|result| result.is_object())
            .cloned()
        else {
            // `traverse_obj(data, ('messages', '__default'), 'title')`.
            let message = response
                .as_ref()
                .and_then(|response| {
                    response
                        .get("messages")
                        .and_then(|messages| messages.get("__default"))
                        .and_then(Value::as_str)
                        .or_else(|| response.get("title").and_then(Value::as_str))
                })
                .unwrap_or("Got empty response from playback API");
            return Err(Error::Reported(format!("[{NAME}] {video_id}: {message}")));
        };

        if data.get("manifest").is_some() {
            return Err(unsupported("playback manifests"));
        }
        if has_text_tracks(&data) {
            return Err(unsupported("playback text tracks"));
        }

        let mut formats = Vec::new();
        for source in data
            .get("sources")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(m3u8_url) = source
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(|url| geo_bypass(&url))
            else {
                continue;
            };
            let mod_url = QUALITY_720
                .replace_all(&m3u8_url, "_1080${1}.m3u8")
                .into_owned();
            if mod_url != m3u8_url {
                let manifest = Url::parse(&mod_url)?;
                if let Some(higher) = extract_m3u8_master_optional(
                    client, &manifest, NAME, &video_id, None, false, false,
                ) {
                    formats.extend(higher.formats);
                    continue;
                }
            }
            let manifest = Url::parse(&m3u8_url)?;
            if let Some(stream) =
                extract_m3u8_master_optional(client, &manifest, NAME, &video_id, None, false, false)
            {
                formats.extend(stream.formats);
            }
        }

        let episode = data.get("episode").cloned().unwrap_or(Value::Null);
        let is_series = episode.get("kind").and_then(Value::as_str) == Some("series");
        let program = episode.get("program").cloned().unwrap_or(Value::Null);
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", Value::from(video_id.as_str())),
            ("title", Value::from(joined_title(&data))),
        ];
        if let Some(thumbnail) = episode
            .get("images")
            .and_then(|images| images.get("thumbnail"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            fields.push(("thumbnail", Value::from(thumbnail)));
        }
        if let Some(release) = episode
            .get("release_date")
            .and_then(Value::as_str)
            .map(|date| date.replace('-', ""))
            .and_then(|date| crate::dates::unified_strdate(&date, false))
        {
            fields.push(("release_date", Value::from(release)));
        }
        let duration = super::clyp::float_or_none(data.get("duration"));
        if let Some(seconds) = duration.as_f64() {
            fields.push(("duration", Value::from(seconds / 1000.0)));
        }
        if let Some(name) = data
            .get("age_rating")
            .and_then(|rating| rating.get("name"))
            .and_then(Value::as_str)
            && let Some(limit) = age_limit(name)?
        {
            fields.push(("age_limit", Value::from(limit)));
        }
        if is_series {
            if let Some(series) = program.get("title").and_then(Value::as_str) {
                fields.push(("series", Value::from(series)));
            }
            if let Some(series_id) = str_or_none(program.get("id")) {
                fields.push(("series_id", Value::from(series_id)));
            }
            if let Some(episode_title) = episode.get("title").and_then(Value::as_str) {
                fields.push(("episode", Value::from(episode_title)));
            }
            let number = super::streamable::int_or_none(episode.get("number"));
            if !number.is_null() {
                fields.push(("episode_number", number));
            }
        }
        fields.push(("formats", Value::Null));
        fields.push(("subtitles", Value::Null));

        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = Subtitles::default();
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.ondemandkorea.com/player/vod/ask-us-anything?contentId=686471"
        ));
        assert!(suitable(
            "https://www.ondemandkorea.com/en/player/vod/capture-the-moment-how-is-that-possible?contentId=1605006"
        ));
        assert!(!suitable(
            "https://www.ondemandkorea.com/player/vod/uskn-news"
        ));
    }

    #[test]
    fn quality_probe_replaces_the_720_manifest() {
        let url = "https://hls.example/x/master_720p.m3u8";
        assert_eq!(
            QUALITY_720.replace_all(url, "_1080${1}.m3u8"),
            "https://hls.example/x/master_1080p.m3u8"
        );
    }

    #[test]
    fn program_urls_follow_upstream_valid_url() {
        assert!(program_suitable(
            "https://www.ondemandkorea.com/player/vod/uskn-news"
        ));
        assert!(program_suitable(
            "https://www.ondemandkorea.com/en/player/vod/the-land"
        ));
        assert!(!program_suitable(
            "https://www.ondemandkorea.com/player/vod/uskn-news?contentId=3518970"
        ));
    }
}
