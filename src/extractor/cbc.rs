//! Upstream `CBCListenIE` and `CBCPlayerIE` (`yt_dlp/extractor/cbc.py`): a
//! listen clip's JSON API (falling back to its page's preloaded state) as an
//! untyped MP3 result, and a player clip's `__INITIAL_STATE__` with its
//! `medianet` audio assets. The CBC Gem, playlist, live and ThePlatform-era
//! paths are not ported; they keep their errors.
use super::clyp::key_error;
use super::{
    Extractor, dict_format, dict_info, direct, download_json, download_json_note, download_webpage,
    search_json,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct CBCListen;
pub struct CBCPlayer;

const LISTEN_NAME: &str = "cbc.ca:listen";
const LISTEN_KEY: &str = "CBCListen";
const PLAYER_NAME: &str = "cbc.ca:player";
const PLAYER_KEY: &str = "CBCPlayer";

static LISTEN_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?cbc\.ca/listen/(?:cbc-podcasts|live-radio)/[\w-]+/[\w-]+/(?P<id>\d+)",
    )
    .unwrap()
});
static PLAYER_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?:cbcplayer:|https?://(?:www\.)?cbc\.ca/(?:player/play/(?:video/)?|i/caffeine/syndicate/\?mediaId=))(?P<id>(?:\d\.)?\d+)",
    )
    .unwrap()
});
static LISTEN_STATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__PRELOADED_STATE__\s*=").unwrap());
static PLAYER_STATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__INITIAL_STATE__\s*=").unwrap());

/// `_match_id` for `CBCListenIE`.
pub fn listen_suitable(url: &str) -> bool {
    LISTEN_VALID_URL.is_match(url)
}

/// `_match_id` for `CBCPlayerIE`.
pub fn player_suitable(url: &str) -> bool {
    PLAYER_VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("CBC {what} is not supported natively"))
}

/// `_search_json(start, webpage, name, video_id, transform_source=js_to_json)`.
fn search_state(
    start: &Regex,
    webpage: &str,
    (extractor, name): (&str, &str),
    id: &str,
) -> Result<Value> {
    search_json(start, webpage, true).flatten().ok_or_else(|| {
        Error::Reported(format!(
            "[{extractor}] {id}: Unable to extract {name}; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })
}

/// `value / scale` as a float, or `None`.
fn scaled_float(value: Option<&Value>, scale: f64) -> Value {
    match super::clyp::float_or_none(value) {
        Value::Number(number) => number
            .as_f64()
            .and_then(|number| serde_json::Number::from_f64(number / scale))
            .map_or(Value::Null, Value::Number),
        _ => Value::Null,
    }
}

/// `int_or_none(value / 1000)`.
fn scaled_int(value: Option<&Value>) -> Value {
    match super::streamable::int_or_none(value) {
        Value::Number(number) => json!(number.as_i64().unwrap_or_default() / 1000),
        _ => Value::Null,
    }
}

/// Upstream `url_or_none` for a traversed string.
fn url_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| Url::parse(text).is_ok())
        .map(str::to_owned)
}

/// `_download_json(url, video_id, fatal=False)`: HTTP errors are warnings.
fn download_json_optional(client: &Client, url: &Url, id: &str) -> Result<Option<Value>> {
    match download_json(client, url, LISTEN_NAME, id) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{LISTEN_NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// An untyped MP3 result for a top-level `url` (no `formats`), with the
/// given metadata fields in upstream's order.
fn untyped_audio(page: &Url, id: &str, media: Url, fields: Vec<(&str, Value)>) -> VideoInfo {
    let mut info = direct::with_format(direct::build_at(media.clone(), &media, "mp3".into()), "");
    for format in &mut info.formats {
        format.ext = "mp3".into();
        format.format_id = String::new();
    }
    info.format_id = None;
    info.id = id.to_owned();
    let mut keys = vec!["id", "url"];
    for (key, value) in fields {
        if value.is_null() {
            continue;
        }
        match key {
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "description" => {
                info.metadata.description = value.as_str().map(str::to_owned);
            }
            _ => crate::metadata_parser::set_field(&mut info, key, value),
        }
        keys.push(key);
    }
    info.webpage_url = page.clone();
    info.direct = None;
    info.extractor = LISTEN_NAME.into();
    info.extractor_key = LISTEN_KEY.into();
    let titled = keys.contains(&"title");
    crate::info_order::mark_info(
        info,
        &format!(
            "untyped-dict;{}{}",
            keys.join(","),
            if titled { "" } else { "-untitled" }
        ),
    )
}

impl Extractor for CBCListen {
    fn key(&self) -> &'static str {
        LISTEN_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = LISTEN_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(LISTEN_NAME);
        crate::screen::screen(&format!(
            "[{LISTEN_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://www.cbc.ca/listen/api/v1/clips/{video_id}"
        ))?;
        let response = download_json_optional(client, &api, &video_id)?;
        let data = response
            .as_ref()
            .and_then(|value| value.get("data"))
            .filter(|value| value.as_object().is_some_and(|object| !object.is_empty()))
            .cloned();
        let data = match data {
            Some(data) => data,
            None => {
                crate::screen::warning(&format!(
                    "[{LISTEN_NAME}] API failed to return data. Falling back to webpage parsing"
                ));
                let webpage = download_webpage(client, url, LISTEN_NAME, &video_id)?;
                search_state(
                    &LISTEN_STATE,
                    &webpage,
                    (LISTEN_NAME, "preloaded state"),
                    &video_id,
                )?;
                // The episode lookup over the page state is not ported.
                return Err(unsupported("listen pages without the clip API"));
            }
        };
        // `('src', 'url')` is a key alternative in upstream's traversal.
        let media = match url_text(data.get("src")).or_else(|| url_text(data.get("url"))) {
            Some(media) => Url::parse(&media)?,
            None => return Err(unsupported("listen clips without an audio URL")),
        };
        let fields = vec![
            ("title", data.get("title").cloned().unwrap_or(Value::Null)),
            (
                "description",
                data.get("description").cloned().unwrap_or(Value::Null),
            ),
            ("release_timestamp", scaled_int(data.get("releasedAt"))),
            ("timestamp", scaled_int(data.get("airdate"))),
            (
                "duration",
                crate::extractor::streamable::int_or_none(data.get("duration")),
            ),
        ];
        Ok(Some(untyped_audio(url, &video_id, media, fields)))
    }
}

impl Extractor for CBCPlayer {
    fn key(&self) -> &'static str {
        PLAYER_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = PLAYER_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(PLAYER_NAME);
        crate::screen::screen(&format!(
            "[{PLAYER_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page_url = Url::parse(&format!("https://www.cbc.ca/player/play/{video_id}"))?;
        let webpage = download_webpage(client, &page_url, PLAYER_NAME, &video_id)?;
        let state = search_state(
            &PLAYER_STATE,
            &webpage,
            (PLAYER_NAME, "initial state"),
            &video_id,
        )?;
        let data = state
            .get("video")
            .and_then(|video| video.get("currentClip"))
            .ok_or_else(|| key_error(&video_id, "video"))?;
        let media = data.get("media").cloned().unwrap_or(Value::Null);

        let assets: Vec<&Value> = data
            .pointer("/media/assets")
            .and_then(Value::as_array)
            .map(|assets| {
                assets
                    .iter()
                    .filter(|asset| {
                        url_text(asset.get("key")).is_some()
                            && asset.get("type").is_some_and(super::nuxt::truthy)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if assets.is_empty() && data.get("mediaId").is_some_and(Value::is_string) {
            // Upstream's deprecated ThePlatform hand-off.
            return Err(unsupported("ThePlatform-era clips"));
        }
        if media.get("streamType").and_then(Value::as_str) == Some("Live") {
            return Err(unsupported("live clips"));
        }
        if media
            .get("textTracks")
            .and_then(Value::as_array)
            .is_some_and(|tracks| {
                tracks
                    .iter()
                    .any(|track| url_text(track.get("src")).is_some())
            })
        {
            return Err(unsupported("subtitle tracks"));
        }
        // Upstream keeps timed chapters; a lone chapter without an end time
        // starting at 0 is dropped as pointless.
        let chapters = media
            .get("chapters")
            .and_then(Value::as_array)
            .map(|chapters| {
                chapters
                    .iter()
                    .filter(|chapter| chapter.get("startTime").is_some_and(|v| !v.is_null()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let pointless = chapters.len() == 1
            && chapters[0]
                .get("startTime")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                == 0.0
            && chapters[0].get("endTime").is_none_or(Value::is_null);
        if !chapters.is_empty() && !pointless {
            return Err(unsupported("chapters"));
        }

        let mut formats = Vec::new();
        for asset in assets {
            let asset_key = url_text(asset.get("key")).unwrap_or_default();
            let asset_type = asset
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if asset_type != "medianet" {
                crate::screen::warning(&format!(
                    "[{PLAYER_NAME}] Skipping unsupported asset type \"{asset_type}\": {asset_key}"
                ));
                continue;
            }
            let asset_url = Url::parse(&asset_key)?;
            let asset_data = download_json_note(
                client,
                &asset_url,
                PLAYER_NAME,
                &video_id,
                "Downloading medianet JSON",
            )?;
            let param = |name: &str| {
                asset_data
                    .get("params")
                    .and_then(Value::as_array)
                    .and_then(|params| {
                        params
                            .iter()
                            .find(|param| param.get("name").and_then(Value::as_str) == Some(name))
                    })
                    .and_then(|param| param.get("value"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            };
            let mime = param("contentType").unwrap_or_default();
            let ext = direct::mimetype2ext_or(&mime, None);
            if ext == "m3u8" {
                return Err(unsupported("HLS assets"));
            }
            let media_url = asset_data
                .get("url")
                .cloned()
                .ok_or_else(|| key_error(&video_id, "url"))?;
            let vcodec = if param("mediaType").as_deref() == Some("audio") {
                Value::from("none")
            } else {
                Value::Null
            };
            formats.push(dict_format(
                vec![
                    ("url", media_url),
                    (
                        "ext",
                        if ext.is_empty() {
                            Value::Null
                        } else {
                            ext.into()
                        },
                    ),
                    ("vcodec", vcodec),
                ],
                PLAYER_NAME,
                &video_id,
            )?);
        }

        let mut fields: Vec<(&str, Value)> = Vec::new();
        let mut push = |key: &'static str, value: Value| {
            if !value.is_null() {
                fields.push((key, value));
            }
        };
        push(
            "title",
            data.get("title")
                .and_then(Value::as_str)
                .map_or(Value::Null, Value::from),
        );
        push(
            "description",
            data.get("description")
                .and_then(Value::as_str)
                .map_or(Value::Null, |text| text.trim().into()),
        );
        push(
            "thumbnail",
            url_text(data.pointer("/image/url")).map_or(Value::Null, |thumbnail| {
                // `update_url(query=None)`.
                let mut url = Url::parse(&thumbnail).unwrap_or_else(|_| page_url.clone());
                url.set_query(None);
                url.as_str().into()
            }),
        );
        push("timestamp", scaled_float(data.get("publishedAt"), 1000.0));
        push(
            "media_type",
            media
                .get("clipType")
                .and_then(Value::as_str)
                .map_or(Value::Null, Value::from),
        );
        push(
            "series",
            data.get("showName")
                .and_then(Value::as_str)
                .map_or(Value::Null, Value::from),
        );
        push(
            "season_number",
            crate::extractor::streamable::int_or_none(media.get("season")),
        );
        push(
            "duration",
            super::clyp::float_or_none(media.get("duration")),
        );
        push(
            "location",
            media
                .get("region")
                .and_then(Value::as_str)
                .map_or(Value::Null, Value::from),
        );
        if let Some(tags) = data.get("tags").and_then(Value::as_array) {
            push(
                "tags",
                tags.iter()
                    .filter_map(|tag| tag.get("name").and_then(Value::as_str))
                    .map(Value::from)
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        // `('media', 'genre', all)`: a lone string is a one-item list.
        match media.get("genre") {
            Some(Value::Array(genres)) => push("genres", Value::Array(genres.clone())),
            Some(Value::String(genre)) => push("genres", json!([genre])),
            _ => {}
        }
        if let Some(categories) = data.get("categories").and_then(Value::as_array) {
            push(
                "categories",
                categories
                    .iter()
                    .filter_map(|category| category.get("name").and_then(Value::as_str))
                    .map(Value::from)
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        fields.push(("id", video_id.clone().into()));
        fields.push(("formats", Value::Null));
        fields.push(("subtitles", Value::Null));
        fields.push(("chapters", json!([])));
        fields.push(("is_live", json!(false)));
        Ok(Some(dict_info(
            url,
            (PLAYER_NAME, PLAYER_KEY),
            fields,
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert!(player_suitable(
            "http://www.cbc.ca/i/caffeine/syndicate/?mediaId=2657631896"
        ));
        assert!(player_suitable("https://www.cbc.ca/player/play/1.2985700"));
        assert!(player_suitable("cbcplayer:2683190193"));
        assert!(!player_suitable("https://www.cbc.ca/news/canada/"));
        assert!(listen_suitable(
            "https://www.cbc.ca/listen/cbc-podcasts/1353-the-naked-emperor/episode/16142603-introducing-understood-who-broke-the-internet"
        ));
        assert!(!listen_suitable("https://www.cbc.ca/listen/"));
    }
}
