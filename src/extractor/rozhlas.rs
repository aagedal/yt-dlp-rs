//! Upstream `RozhlasVltavaIE` (`yt_dlp/extractor/rozhlas.py`): a `*.rozhlas.cz`
//! page's `mujRozhlasPlayer` playlist of episodes with their direct
//! `audioLinks`. DASH and HLS audio links (upstream's `_extract_mpd_formats`
//! and `_extract_m3u8_formats`) are not supported natively, and `RozhlasIE`
//! (`prehravac.rozhlas.cz`, a single URL-only result) is not ported yet.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct RozhlasVltava;

const VLTAVA_NAME: &str = "RozhlasVltava";

static VLTAVA_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:\w+\.rozhlas|english\.radio)\.cz/[\w-]+-(?P<id>\d+)").unwrap()
});
static PLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(<div class="mujRozhlasPlayer" data-player='[^']+'>)"#).unwrap()
});

/// `_match_valid_url` for `RozhlasVltavaIE`.
pub fn vltava_suitable(url: &str) -> bool {
    VLTAVA_VALID_URL.is_match(url)
}

fn search_error(video_id: &str, name: &str) -> Error {
    Error::Reported(format!(
        "[{VLTAVA_NAME}] {video_id}: Unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Rozhlas {what} are not supported natively"))
}

/// `int_or_none` for a JSON value.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `str_or_none` for a JSON value.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

/// `traverse_obj(value, path)` over object keys only.
fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .filter(|value| !value.is_null())
}

impl Extractor for RozhlasVltava {
    fn key(&self) -> &'static str {
        VLTAVA_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VLTAVA_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(VLTAVA_NAME);
        crate::screen::screen(&format!(
            "[{VLTAVA_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, VLTAVA_NAME, &video_id)?;

        // `extract_attributes(...)['data-player']` then `['data']`.
        let player = PLAYER
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| search_error(&video_id, "player"))?;
        let player = crate::html_elements::extract_attributes(&player);
        let player = match player.get("data-player") {
            Some(Value::String(text)) => text.clone(),
            _ => return Err(search_error(&video_id, "player")),
        };
        let data = crate::json::from_str::<Value>(&player)
            .map_err(|_| unsupported("player data that does not parse"))?
            .get("data")
            .cloned()
            .ok_or_else(|| unsupported("player data without a `data` key"))?;

        let playlist = data
            .get("playlist")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut entries = Vec::with_capacity(playlist.len());
        for entry in &playlist {
            entries.push(extract_video(entry, url)?);
        }

        let playlist_id = str_or_none(data.get("embedId")).unwrap_or(video_id.clone());
        let title = at(&data, &["series", "title"])
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;_type,id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: VLTAVA_NAME.into(),
            extractor_key: VLTAVA_NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// `RozhlasBaseIE._extract_formats` for the direct audio links.
fn extract_formats(entry: &Value, audio_id: &str) -> Result<Vec<crate::VideoFormat>> {
    let mut formats = Vec::new();
    let links = entry
        .get("audioLinks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for audio in links {
        let Some(Value::String(url)) = audio.get("url").filter(|url| url.is_string()) else {
            continue;
        };
        if url.is_empty() {
            continue;
        }
        let ext = audio.get("variant").cloned().unwrap_or(Value::Null);
        match ext.as_str() {
            Some("dash") | Some("hls") => {
                return Err(unsupported("DASH and HLS audio links"));
            }
            _ => {}
        }
        let ext_text = str_or_none(Some(&ext)).unwrap_or_default();
        let mut format = dict_format(
            vec![
                ("url", url.clone().into()),
                ("ext", ext.clone()),
                ("format_id", ext.clone()),
                (
                    "abr",
                    int_or_none(audio.get("bitrate")).map_or(Value::Null, Value::from),
                ),
                ("acodec", ext_text.clone().into()),
                ("vcodec", "none".into()),
            ],
            VLTAVA_NAME,
            audio_id,
        )?;
        // `int_or_none` abr: an integer in the dict's formats.
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    Ok(formats)
}

/// `RozhlasVltavaIE._extract_video`.
fn extract_video(entry: &Value, url: &Url) -> Result<VideoInfo> {
    let audio_id = at(entry, &["meta", "ga", "contentId"])
        .and_then(|value| str_or_none(Some(value)))
        .ok_or_else(|| Error::Reported(format!("[{VLTAVA_NAME}] KeyError: 'contentId'")))?;
    let chapter_number = int_or_none(at(entry, &["meta", "ga", "contentSerialPart"]));
    let chapter = if chapter_number.is_some() {
        at(entry, &["meta", "ga", "contentNameShort"]).cloned()
    } else {
        None
    };
    let formats = extract_formats(entry, &audio_id)?;
    let mut fields = vec![
        ("id", audio_id.clone().into()),
        ("chapter", chapter.unwrap_or(Value::Null)),
        (
            "chapter_number",
            chapter_number.map_or(Value::Null, Value::from),
        ),
        ("formats", Value::Null),
    ];
    if let Some(title) = at(entry, &["meta", "ga", "contentName"]) {
        fields.push(("title", title.clone()));
    }
    if let Some(description) = at(entry, &["title"]) {
        fields.push(("description", description.clone()));
    }
    if let Some(duration) = int_or_none(at(entry, &["duration"])) {
        fields.push(("duration", duration.into()));
    }
    if let Some(artist) = at(entry, &["meta", "ga", "contentAuthor"]) {
        fields.push(("artist", artist.clone()));
    }
    if let Some(channel) = at(entry, &["meta", "ga", "contentCreator"]) {
        fields.push(("channel_id", channel.clone()));
    }
    Ok(dict_info(url, (VLTAVA_NAME, VLTAVA_NAME), fields, formats))
}

/// Upstream `RozhlasIE` (`prehravac.rozhlas.cz/audio/<id>`): the player page's
/// title, description and duration around the `media.rozhlas.cz` MP3 URL,
/// returned as a URL-only result (no media request is made).
pub struct RozhlasAudio;

const AUDIO_NAME: &str = "Rozhlas";
static AUDIO_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?prehravac\.rozhlas\.cz/audio/(?P<id>[0-9]+)").unwrap()
});
/// `<h3>(.+?)</h3>\s*<p[^>]*>.*?</p>\s*<div[^>]+id=["']player-track`.
static AUDIO_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<h3>(.+?)</h3>\s*<p[^>]*>.*?</p>\s*<div[^>]+id=["']player-track"#).unwrap()
});
/// `<p[^>]+title=(["'])(?P<url>...)\1[^>]*>.*?</p>\s*<div[^>]+id=["']player-track`
/// (the quote backreference is spelled out as two alternatives).
static AUDIO_DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<p[^>]+title=(?:"([^"\n]+)"|'([^'\n]+)')[^>]*>.*?</p>\s*<div[^>]+id=["']player-track"#,
    )
    .unwrap()
});
static AUDIO_DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-duration=["'](\d+)"#).unwrap());

/// `_match_id` for `RozhlasIE`.
pub fn audio_suitable(url: &str) -> bool {
    AUDIO_VALID_URL.is_match(url)
}

impl Extractor for RozhlasAudio {
    fn key(&self) -> &'static str {
        AUDIO_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = AUDIO_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let audio_id = found["id"].to_owned();
        crate::screen::set_extractor(AUDIO_NAME);
        crate::screen::screen(&format!(
            "[{AUDIO_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = Url::parse(&format!("http://prehravac.rozhlas.cz/audio/{audio_id}"))?;
        let webpage = download_webpage(client, &page, AUDIO_NAME, &audio_id)?;

        // `_html_search_regex(...) or remove_start(_og_search_title(...), 'Radio Wave - ')`.
        // `_html_search_regex` cleans its match (`clean_html(...).strip()`).
        let title = AUDIO_TITLE
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .filter(|title| !title.is_empty())
            .or_else(|| {
                crate::html::og_search_property(&webpage, "title").map(|title| {
                    title
                        .strip_prefix("Radio Wave - ")
                        .unwrap_or(&title)
                        .to_owned()
                })
            });
        let description = AUDIO_DESCRIPTION.captures(&webpage).and_then(|found| {
            found
                .get(1)
                .or_else(|| found.get(2))
                .map(|value| crate::html::clean_html(value.as_str()).trim().to_owned())
        });
        let duration = AUDIO_DURATION
            .captures(&webpage)
            .and_then(|found| found[1].parse::<i64>().ok());

        let media = Url::parse(&format!("http://media.rozhlas.cz/_audio/{audio_id}.mp3"))?;
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp3".into()),
            "",
        );
        for format in &mut info.formats {
            format.ext = "mp3".into();
            format.format_id = String::new();
            format.vcodec = Some("none".into());
        }
        info.format_id = None;
        info.id = audio_id;
        let mut keys = vec!["id", "url"];
        match title {
            Some(title) => {
                info.title = title;
                keys.push("title");
            }
            None => {
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        if let Some(description) = description {
            info.metadata.description = Some(description);
            keys.push("description");
        }
        if let Some(duration) = duration {
            crate::metadata_parser::set_field(&mut info, "duration", duration.into());
            keys.push("duration");
        }
        keys.push("vcodec");
        crate::metadata_parser::set_field(&mut info, "vcodec", "none".into());
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = AUDIO_NAME.into();
        info.extractor_key = AUDIO_NAME.into();
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!(
                "untyped-dict;{}{}",
                keys.join(","),
                if keys.contains(&"title") {
                    ""
                } else {
                    "-untitled"
                }
            ),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(vltava_suitable(
            "https://wave.rozhlas.cz/papej-masicko-porcujeme-a-bilancujeme-filmy-a-serialy-ktere-letos-zabily-8891337"
        ));
        assert!(!vltava_suitable("https://www.mujrozhlas.cz/nespavci"));
    }

    #[test]
    fn audio_urls_follow_upstream_valid_url() {
        assert!(audio_suitable("http://prehravac.rozhlas.cz/audio/3421320"));
        assert!(audio_suitable(
            "https://www.prehravac.rozhlas.cz/audio/3421320/embed"
        ));
        assert!(!audio_suitable("http://prehravac.rozhlas.cz/audio/"));
    }
}
