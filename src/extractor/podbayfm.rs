//! Upstream `PodbayFMIE` and `PodbayFMChannelIE` (`yt_dlp/extractor/podbayfm.py`):
//! an episode page's `__NEXT_DATA__` episode as an untyped MP3 result, and a
//! channel's paged podcast API episodes as a playlist of them.
use super::{Extractor, download_json_note, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct PodbayFm;
pub struct PodbayFmChannel;

const NAME: &str = "PodbayFM";
const CHANNEL_NAME: &str = "PodbayFMChannel";
const PAGE_SIZE: usize = 10;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://podbay\.fm/p/[^/?#]+/e/(?P<id>\d+)").unwrap()
});
static CHANNEL_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://podbay\.fm/p/(?P<id>[^/?#]+)/?(?:$|[?#])").unwrap());

/// `PodbayFMIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `PodbayFMChannelIE._match_id`.
pub fn channel_suitable(url: &str) -> bool {
    CHANNEL_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Podbay {what} are not supported natively"))
}

/// `jwt_decode_hs256` (errors are dropped by `traverse_obj`).
pub(crate) fn jwt_payload(jwt: &str) -> Option<Value> {
    let parts: Vec<&str> = jwt.split('.').collect();
    let [_, payload, _] = parts.as_slice() else {
        return None;
    };
    let payload = payload.trim_end_matches('=');
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// `result_from_props(props)`, plus the channel's extractor and page URL.
fn result_from_props(props: &Value, page: &Url, channel: Option<&str>) -> Result<VideoInfo> {
    let id = props
        .get("_id")
        .and_then(Value::as_str)
        .ok_or_else(|| unsupported("episodes without a string ID"))?
        .to_owned();
    let title = props
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let media = props
        .get("mediaURL")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none);
    let Some(media) = media else {
        if title.is_none() {
            crate::screen::warning(
                "Extractor failed to obtain \"title\". Creating a generic title instead",
            );
        }
        return Err(Error::Reported(format!(
            "[{NAME}] {id}: No video formats found!; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let description = props
        .get("description")
        .and_then(Value::as_str)
        .map(crate::html::clean_html);
    let thumbnail = props
        .get("image")
        .and_then(Value::as_str)
        .and_then(jwt_payload)
        .and_then(|image| image.get("url").and_then(Value::as_str).map(str::to_owned))
        .and_then(|image| super::json_ld::url_or_none(&image));
    let timestamp = super::streamable::int_or_none(props.get("timestamp"));
    let duration = super::streamable::int_or_none(props.get("duration"));
    let media = Url::parse(&media)?;
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
    info.id = id;
    let mut keys = vec!["id"];
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
    keys.push("url");
    if let Some(description) = description {
        info.metadata.description = Some(description);
        keys.push("description");
    }
    if let Some(thumbnail) = thumbnail {
        info.metadata.thumbnail = Some(thumbnail);
        keys.push("thumbnail");
    }
    for (key, value) in [("timestamp", timestamp), ("duration", duration)] {
        if !value.is_null() {
            crate::metadata_parser::set_field(&mut info, key, value);
            keys.push(key);
        }
    }
    keys.extend(["ext", "vcodec"]);
    crate::metadata_parser::set_field(&mut info, "vcodec", "none".into());
    info.webpage_url = page.clone();
    info.direct = None;
    info.extractor = NAME.into();
    info.extractor_key = NAME.into();
    if let Some(channel) = channel {
        let timestamp = props
            .get("timestamp")
            .map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .ok_or_else(|| unsupported("channel episodes without a timestamp"))?;
        info.webpage_url = Url::parse(&format!("https://podbay.fm/p/{channel}/e/{timestamp}"))?;
        keys.extend(["extractor", "extractor_key", "webpage_url"]);
    }
    Ok(crate::info_order::mark_info(
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
    ))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `_fetch_page(channel_id, pagenum)`: the API page's `podcast`.
fn fetch_podcast(client: &Client, channel_id: &str, pagenum: usize) -> Result<Value> {
    let api = super::update_url_query(
        &Url::parse("https://podbay.fm/api/podcast")?,
        &[
            ("reverse", "true"),
            ("page", &pagenum.to_string()),
            ("slug", channel_id),
        ],
    );
    // Upstream passes the note as the video ID and the ID as the note.
    let data = download_json_note(
        client,
        &api,
        CHANNEL_NAME,
        &format!("Downloading channel JSON page {}", pagenum + 1),
        channel_id,
    )?;
    data.get("podcast")
        .cloned()
        .ok_or_else(|| key_error(channel_id, "podcast"))
}

/// Whether a playlist's pending state is a PodbayFM channel's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == "podbayfm"
}

/// `OnDemandPagedList(... first_page ..., 10)` read by upstream's
/// `PlaylistEntries` (see [`super::paged`]); page 0 is the extraction's.
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let channel_id = state["channel_id"].as_str().unwrap_or_default().to_owned();
    let page = Url::parse(state["url"].as_str().unwrap_or_default())?;
    let first_page = state["first_page"].clone();
    let mut fetch = |pagenum: usize| -> Result<Vec<VideoInfo>> {
        let podcast = if pagenum == 0 {
            first_page.clone()
        } else {
            fetch_podcast(client, &channel_id, pagenum)?
        };
        let episodes = match podcast.get("episodes") {
            Some(Value::Array(episodes)) => episodes,
            _ => return Err(unsupported("channel pages without an episode list")),
        };
        episodes
            .iter()
            .map(|episode| result_from_props(episode, &page, Some(&channel_id)))
            .collect()
    };
    let placeholder = || Ok(super::direct::build_at(page.clone(), &page, "mp3".into()));
    super::paged::fetch_requested(state, limit, entries, PAGE_SIZE, &mut fetch, &placeholder)
}

impl Extractor for PodbayFm {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let episode_id = found["id"].to_owned();
        announce(NAME, url);
        let webpage = download_webpage(client, url, NAME, &episode_id)?;
        let data = super::search_nextjs_data(&webpage, NAME, &episode_id)?;
        let props = data
            .get("props")
            .ok_or_else(|| key_error(&episode_id, "props"))?
            .get("pageProps")
            .ok_or_else(|| key_error(&episode_id, "pageProps"))?
            .get("episode")
            .ok_or_else(|| key_error(&episode_id, "episode"))?;
        result_from_props(props, url, None).map(Some)
    }
}

impl Extractor for PodbayFmChannel {
    fn key(&self) -> &'static str {
        CHANNEL_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CHANNEL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let channel_id = found["id"].to_owned();
        announce(CHANNEL_NAME, url);
        let first_page = fetch_podcast(client, &channel_id, 0)?;
        let title = first_page.get("title").cloned();
        let state = json!({
            "kind": "podbayfm",
            "channel_id": channel_id,
            "url": url.as_str(),
            "first_page": first_page,
            "random_access": true,
        });
        let mut keys = vec!["id"];
        let title = match title {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => Some(title),
            Some(_) => return Err(unsupported("channel titles that are not strings")),
        };
        if title.is_some() {
            keys.push("title");
        }
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;{}{}",
                keys.join(","),
                crate::info_order::GENERATOR
            )
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: channel_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: CHANNEL_NAME.into(),
            extractor_key: CHANNEL_NAME.into(),
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
            "https://podbay.fm/p/behind-the-bastards/e/1647338400"
        ));
        assert!(channel_suitable("https://podbay.fm/p/behind-the-bastards"));
        assert!(!channel_suitable(
            "https://podbay.fm/p/behind-the-bastards/e/1647338400"
        ));
        assert!(!suitable("https://podbay.fm/p/behind-the-bastards"));
    }
}
