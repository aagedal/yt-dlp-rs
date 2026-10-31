//! Upstream `GeniusIE` and `GeniusLyricsIE` (`yt_dlp/extractor/genius.py`):
//! Genius videos and articles as Brightcove player `url` results (the page
//! smuggled as the referrer), and lyrics pages' song media as a playlist of
//! `url` results.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Genius;
pub struct GeniusLyrics;

const NAME: &str = "Genius";
const LYRICS_NAME: &str = "GeniusLyrics";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?genius\.com/(?:videos|(?P<article>a))/(?P<id>[^?/#]+)",
    )
    .unwrap()
});
static LYRICS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?genius\.com/(?P<id>[^?/#]+)-lyrics(?:[?/#]|$)").unwrap()
});
static PAGE_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<meta content="\s*(\{.+\})\s*"\s+itemprop="page_data""#).unwrap()
});
static APP_CONFIG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"var\s*APP_CONFIG\s*=").unwrap());
static PRELOADED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)window\.__PRELOADED_STATE__\s*=\s*JSON\.parse\(\s*('\{.+\}')").unwrap()
});

/// `GeniusIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `GeniusLyricsIE._match_id`.
pub fn lyrics_suitable(url: &str) -> bool {
    LYRICS_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Genius {what} are not supported natively"))
}

/// The first JSON value of `text` (`ignore_extra=True`).
fn first_json(text: &str) -> Option<Value> {
    serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .next()
        .and_then(std::result::Result::ok)
}

/// `...` over a list's items or an object's values.
fn branch(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    }
}

/// `traverse_obj(metadata, ((('article', 'media', ...), ('video', None)),
/// ('provider_id', ('dfp_kv', lambda _, v: v['name'] == 'brightcove_video_id',
/// 'values', ...))), get_all=False)`.
fn video_id(metadata: &Value) -> Option<Value> {
    let mut objects = branch(metadata.pointer("/article/media"));
    if let Some(video) = metadata.get("video") {
        objects.push(video);
    }
    for object in objects {
        if let Some(id) = object.get("provider_id").filter(|id| !id.is_null()) {
            return Some(id.clone());
        }
        for item in branch(object.get("dfp_kv")) {
            if item.get("name").and_then(Value::as_str) != Some("brightcove_video_id") {
                continue;
            }
            if let Some(value) = branch(item.get("values"))
                .into_iter()
                .find(|value| !value.is_null())
            {
                return Some(value.clone());
            }
        }
    }
    None
}

/// Python `str(value)` of a JSON scalar.
fn py_str(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        _ => Err(unsupported("player IDs that are not strings or numbers")),
    }
}

impl Extractor for Genius {
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
        let display_id = found["id"].to_owned();
        let is_article = found.name("article").is_some();
        start(NAME, url);
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let metadata = match PAGE_DATA.captures(&webpage) {
            Some(data) => first_json(&crate::html::unescape_html(&data[1])).ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract metadata - Failed to parse JSON"
                ))
            })?,
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract metadata; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let Some(video_id) = video_id(&metadata).filter(super::nuxt::truthy) else {
            // Not all article pages have videos (an expected error there).
            let message = "Brightcove video ID not found in webpage";
            return Err(Error::Reported(if is_article {
                format!("[{NAME}] {display_id}: {message}")
            } else {
                format!(
                    "[{NAME}] {display_id}: {message}; {}",
                    crate::BUG_REPORTS_MESSAGE
                )
            }));
        };
        let video_id = py_str(&video_id)?;
        // `_search_json(..., default={})`.
        let config = match super::search_json(&APP_CONFIG, &webpage, false) {
            Some(Some(Value::Object(config))) => config,
            _ => Map::new(),
        };
        let account_id = match config.get("brightcove_account_id") {
            None => "4863540648001".to_owned(),
            Some(Value::Null) => "None".to_owned(),
            Some(value) => py_str(value)?,
        };
        let player_id = [
            "brightcove_standard_web_player_id",
            "brightcove_standard_no_autoplay_web_player_id",
            "brightcove_modal_web_player_id",
            "brightcove_song_story_web_player_id",
        ]
        .iter()
        .find_map(|key| config.get(*key).filter(|value| !value.is_null()))
        .map(py_str)
        .transpose()?
        .unwrap_or_else(|| "S1ZcmcOC1x".into());
        let target = super::brightcove::smuggle(
            &format!(
                "https://players.brightcove.net/{account_id}/{player_id}_default/index.html?videoId={video_id}"
            ),
            &json!({"referrer": url.as_str()}),
        );
        // `url_result(smuggled, 'BrightcoveNew', video_id)`.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.id = video_id;
        follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

/// `url_result(url)`: a plain `url` entry.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target).map_err(|_| unsupported("media URLs that are not absolute"))?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": null,
            "entry": {"_type": "url", "url": target},
        }),
    );
    Ok(info)
}

/// The `trackingData` item's value whose `key` is `name`.
fn tracking_value(song_info: &Value, name: &str) -> Option<Value> {
    branch(song_info.pointer("/songPage/trackingData"))
        .into_iter()
        .filter(|item| item.get("key").and_then(Value::as_str) == Some(name))
        .find_map(|item| item.get("value").filter(|value| !value.is_null()).cloned())
}

impl Extractor for GeniusLyrics {
    fn key(&self) -> &'static str {
        LYRICS_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = LYRICS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(LYRICS_NAME, url);
        let webpage = download_webpage(client, url, LYRICS_NAME, &display_id)?;
        // `_search_json(..., transform_source=js_to_json,
        // contains_pattern=r"'{(?s:.+)}'")`: the JS string literal, then its
        // JSON text.
        let literal = PRELOADED.captures(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{LYRICS_NAME}] {display_id}: Unable to extract json string; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let json_string = first_json(&crate::js_to_json::js_to_json(&literal[1]))
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{LYRICS_NAME}] {display_id}: Unable to extract json string - Failed to parse JSON"
                ))
            })?;
        let song_info: Value = crate::json::from_str(&json_string)
            .map_err(|_| unsupported("song states that do not parse"))?;
        let song_id = match song_info.pointer("/songPage/song") {
            Some(Value::String(id)) => Some(id.clone()),
            Some(Value::Number(id)) => Some(id.to_string()),
            _ => None,
        }
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{LYRICS_NAME}] {display_id}: Song id not found in webpage; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let text = |value: Option<Value>, default: &str| match value {
            None => Ok(default.to_owned()),
            Some(Value::String(text)) => Ok(text),
            Some(_) => Err(unsupported("tracking values that are not strings")),
        };
        let title = text(tracking_value(&song_info, "Title"), "untitled")?;
        let artist = text(
            tracking_value(&song_info, "Primary Artist"),
            "unknown artist",
        )?;
        let media = song_info
            .get("entities")
            .and_then(|entities| entities.get("songs"))
            .and_then(|songs| songs.get(&song_id))
            .and_then(|song| song.get("media"));
        let mut entries = Vec::new();
        if let Some(Value::Array(media)) = media {
            for item in media {
                let kind = item.get("type").and_then(Value::as_str);
                let Some(media_url) = item.get("url").filter(|url| super::nuxt::truthy(url)) else {
                    continue;
                };
                if !matches!(kind, Some("video" | "audio")) {
                    continue;
                }
                if item.get("provider").and_then(Value::as_str) == Some("spotify") {
                    crate::screen::screen(&format!(
                        "[{LYRICS_NAME}] {song_id}: Skipping Spotify audio embed"
                    ));
                    continue;
                }
                let Some(media_url) = media_url.as_str() else {
                    return Err(unsupported("media URLs that are not strings"));
                };
                entries.push(url_entry(media_url)?);
            }
        }
        // `playlist_result(entries, song_id, title, f'{title} by {artist}')`.
        let mut keys = vec!["id"];
        if !title.is_empty() {
            keys.push("title");
        }
        keys.push("description");
        let mut extra = Map::new();
        extra.insert("description".into(), format!("{title} by {artist}").into());
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: song_id,
            title,
            webpage_url: url.clone(),
            extractor: LYRICS_NAME.into(),
            extractor_key: LYRICS_NAME.into(),
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
            "https://genius.com/videos/Vince-staples-breaks-down-the-meaning-of-when-sparks-fly"
        ));
        assert!(suitable(
            "https://genius.com/a/cordae-anderson-paak-break-down-the-meaning-of-two-tens"
        ));
        assert!(!suitable("https://genius.com/Lil-baby-heyy-lyrics"));
        assert!(lyrics_suitable("https://genius.com/Lil-baby-heyy-lyrics"));
        assert!(!lyrics_suitable("https://genius.com/Lil-baby-heyy-lyricsx"));
    }

    #[test]
    fn video_ids_follow_the_traversal_order() {
        let metadata = json!({
            "article": {"media": [{"dfp_kv": [{"name": "x", "values": ["no"]},
                {"name": "brightcove_video_id", "values": [null, "123"]}]}]},
            "video": {"provider_id": "456"},
        });
        assert_eq!(video_id(&metadata), Some(json!("123")));
        assert_eq!(
            video_id(&json!({"video": {"provider_id": 7}})),
            Some(json!(7))
        );
        assert_eq!(video_id(&json!({})), None);
    }
}
