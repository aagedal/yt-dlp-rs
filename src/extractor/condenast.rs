//! Upstream `CondeNastIE` (`yt_dlp/extractor/condenast.py`): the Condé Nast
//! group's custom HTML5 player, reached through its `player.cnevids.com`
//! embed (`embedjs`/`inline`/`script`) URLs or a `watch`/`video` page. The
//! embed API (with its `video.js`, `loader.js` and inline `config` fallbacks)
//! supplies the sources and captions. Series pages are not supported natively.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::subtitles::{SubtitleFormat, Subtitles};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct CondeNast;

const NAME: &str = "CondeNast";
const EMBED_API: &str = "http://player.cnevids.com/embed-api.json";

/// The supported sites (`_SITES`), in upstream's order.
const SITES: [&str; 19] = [
    "allure",
    "architecturaldigest",
    "arstechnica",
    "bonappetit",
    "brides",
    "cnevids",
    "cntraveler",
    "details",
    "epicurious",
    "glamour",
    "golfdigest",
    "gq",
    "newyorker",
    "self",
    "teenvogue",
    "vanityfair",
    "vogue",
    "wired",
    "wmagazine",
];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        concat!(
            r"^https?://(?:video|www|player(?:-backend)?)\.(?:{sites})\.com/",
            r"(?:",
            r"(?:embed(?:js)?|(?:script|inline)/video)/(?P<id>[0-9a-f]{{24}})(?:/(?P<player_id>[0-9a-f]{{24}}))?(?:.+?\btarget=(?P<target>[^&]+))?|",
            r"(?P<type>watch|series|video)/(?P<display_id>[^/?#]+)",
            r")",
        ),
        sites = SITES.join("|"),
    ))
    .unwrap()
});
static PARAMS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)var\s+params\s*=\s*(\{.+?\})[;,]").unwrap());
static PARAMS_VIDEO_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:data-video-id=|currentVideoId\s*=\s*)["']([\da-f]+)"#).unwrap()
});
static PARAMS_ELEMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(<[^>]+data-js="video-player"[^>]+>)"#).unwrap());
static PRELOADED_STATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)__PRELOADED_STATE__\s*=\s*(\{.+?\});").unwrap());
static CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)var\s+config\s*=\s*(\{.+?\});").unwrap());

/// `_match_valid_url`'s suitability.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Upstream's `params` dict: keys in order, `None` values (Python's `None`).
type Params = Vec<(String, Option<String>)>;

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Condé Nast {what} are not supported natively"))
}

fn param<'a>(params: &'a Params, name: &str) -> Option<&'a Option<String>> {
    params
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

fn set_param(params: &mut Params, name: &str, value: Option<String>) {
    match params.iter_mut().find(|(key, _)| key == name) {
        Some(slot) => slot.1 = value,
        None => params.push((name.to_owned(), value)),
    }
}

/// `query` pairs with Python's `str()` of each value (`None`, not dropped).
fn query_pairs(params: &Params) -> Vec<(String, String)> {
    params
        .iter()
        .map(|(key, value)| (key.clone(), value.clone().unwrap_or_else(|| "None".into())))
        .collect()
}

fn url_with_query(base: &str, pairs: &[(String, String)]) -> Result<Url> {
    Ok(Url::parse_with_params(
        base,
        pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    )?)
}

/// A parsed JSON value as Python's `str()` (for the `var params` query).
fn python_value(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(true) => Some("True".into()),
        Value::Bool(false) => Some("False".into()),
        other => Some(other.to_string()),
    }
}

/// `_extract_video_params(webpage, display_id)`.
fn extract_video_params(webpage: &str, display_id: &str) -> Result<Params> {
    let query = PARAMS
        .captures(webpage)
        .and_then(|found| {
            serde_json::from_str::<Value>(&crate::js_to_json::js_to_json(&found[1])).ok()
        })
        .and_then(|value| value.as_object().cloned())
        .filter(|object| !object.is_empty());
    if let Some(query) = query {
        let mut params: Params = query
            .iter()
            .map(|(key, value)| (key.clone(), python_value(value)))
            .collect();
        let video_id = PARAMS_VIDEO_ID
            .captures(webpage)
            .map(|found| found[1].to_owned());
        set_param(&mut params, "videoId", video_id);
        Ok(params)
    } else {
        let element = PARAMS_ELEMENT
            .captures(webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] Unable to extract player params element; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let attributes = crate::html_elements::extract_attributes(&element);
        let attribute = |key: &str| match attributes.get(key) {
            Some(Value::String(text)) => Some(text.clone()),
            _ => None,
        };
        let missing = |key: &str| {
            Error::Reported(format!(
                "{display_id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let video = attribute("data-video").ok_or_else(|| missing("data-video"))?;
        let player = attribute("data-player").ok_or_else(|| missing("data-player"))?;
        let target = attribute("id").ok_or_else(|| missing("id"))?;
        Ok(vec![
            ("videoId".into(), Some(video)),
            ("playerId".into(), Some(player)),
            ("target".into(), Some(target)),
        ])
    }
}

/// `_extract_video(params)`: the embed API (with its fallbacks) and the
/// formats and captions of the video. `description` is the watch page's
/// preloaded description, placed first as upstream's `info` dict does.
fn extract_video(
    client: &Client,
    page: &Url,
    params: &Params,
    description: Option<Value>,
) -> Result<VideoInfo> {
    let Some(Some(video_id)) = param(params, "videoId").cloned() else {
        return Err(key_error("", "videoId"));
    };

    // New API path.
    let mut embed_query = query_pairs(params);
    embed_query.push(("embedType".into(), "inline".into()));
    let embed_url = url_with_query(EMBED_API, &embed_query)?;
    let mut info_page = download_json(client, &embed_url, NAME, &video_id)
        .ok()
        .filter(|page| page.as_object().is_some_and(|object| !object.is_empty()));

    // Old fallbacks.
    if info_page.is_none()
        && let Some(Some(_)) = param(params, "playerId")
    {
        let video_js = url_with_query(
            "http://player.cnevids.com/player/video.js",
            &query_pairs(params),
        )?;
        info_page = download_json(client, &video_js, NAME, &video_id)
            .ok()
            .filter(|page| page.as_object().is_some_and(|object| !object.is_empty()));
    }
    let mut video_info = info_page
        .as_ref()
        .and_then(|page| page.get("video"))
        .filter(|video| video.as_object().is_some_and(|object| !object.is_empty()))
        .cloned();
    let mut info_text = String::new();
    if video_info.is_none() {
        let loader = url_with_query(
            "http://player.cnevids.com/player/loader.js",
            &query_pairs(params),
        )?;
        info_text = download_webpage(client, &loader, NAME, &video_id)?;
    }
    if video_info.is_none() {
        let target = match param(params, "target") {
            Some(Some(target)) => target.clone(),
            Some(None) => "None".into(),
            None => "embedplayer".into(),
        };
        let inline = url_with_query(
            &format!("https://player.cnevids.com/inline/video/{video_id}.js"),
            &[("target".into(), target)],
        )?;
        info_text = download_webpage(client, &inline, NAME, &video_id)?;
    }
    if video_info.is_none() {
        let config = CONFIG
            .captures(&info_text)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract config; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let parsed: Value =
            serde_json::from_str(&crate::js_to_json::js_to_json(&config)).map_err(|_| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to parse config; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        video_info = Some(
            parsed
                .get("video")
                .cloned()
                .ok_or_else(|| key_error(&video_id, "video"))?,
        );
    }
    let video_info = video_info.unwrap_or(Value::Null);

    let title = video_info
        .get("title")
        .cloned()
        .ok_or_else(|| key_error(&video_id, "title"))?;
    let sources = video_info
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| key_error(&video_id, "sources"))?;

    let mut formats = Vec::new();
    for fdata in sources {
        let Some(src) = fdata
            .get("src")
            .and_then(Value::as_str)
            .filter(|src| !src.is_empty())
        else {
            continue;
        };
        let mime_ext = super::vidyard::mimetype2ext(fdata.get("type"));
        let ext = match mime_ext.as_str() {
            Some(ext) if !ext.is_empty() => ext.to_owned(),
            _ => super::direct::determine_ext(src).unwrap_or_else(|| "unknown_video".into()),
        };
        if ext == "m3u8" {
            let manifest = Url::parse(src)?;
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
            continue;
        }
        let quality = fdata
            .get("quality")
            .and_then(Value::as_str)
            .filter(|q| !q.is_empty());
        let format_id = match quality {
            Some(quality) => format!("{ext}-{quality}"),
            None => ext.clone(),
        };
        formats.push(dict_format(
            vec![
                ("format_id", format_id.into()),
                ("url", src.into()),
                ("ext", ext.into()),
                ("quality", i64::from(quality == Some("high")).into()),
            ],
            NAME,
            &video_id,
        )?);
    }

    // `video_info.get('captions', {}).items()`: a `null` captions value fails.
    let captions = match video_info.get("captions") {
        None => serde_json::Map::new(),
        Some(Value::Object(captions)) => captions.clone(),
        Some(_) => {
            return Err(Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by AttributeError(\"'NoneType' object has no attribute 'items'\")); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
    };
    let mut subtitles = Subtitles::default();
    for (kind, caption) in &captions {
        let Some(caption_url) = caption
            .get("src")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if !matches!(kind.as_str(), "vtt" | "srt" | "tml") {
            continue;
        }
        let url = Url::parse(caption_url)?;
        let ext =
            super::direct::determine_ext(caption_url).unwrap_or_else(|| "unknown_video".into());
        let track = SubtitleFormat {
            name: None,
            url,
            ext,
            protocol: None,
            fragments: Vec::new(),
            extra: Default::default(),
        };
        match subtitles
            .entries
            .iter_mut()
            .find(|(key, _)| key.as_deref() == Some("en"))
        {
            Some((_, tracks)) => tracks.push(track),
            None => subtitles.entries.push((Some("en".into()), vec![track])),
        }
    }

    let mut fields = Vec::new();
    if let Some(description) = description {
        fields.push(("description", description));
    }
    fields.extend([
        ("id", Value::from(video_id.clone())),
        ("formats", Value::Null),
        ("title", title),
        (
            "thumbnail",
            video_info
                .get("poster_frame")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "uploader",
            video_info.get("brand").cloned().unwrap_or(Value::Null),
        ),
        (
            "duration",
            super::streamable::int_or_none(video_info.get("duration")),
        ),
        (
            "tags",
            video_info.get("tags").cloned().unwrap_or(Value::Null),
        ),
        (
            "series",
            video_info
                .get("series_title")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "season",
            video_info
                .get("season_title")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "timestamp",
            video_info
                .get("premiere_date")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map_or(Value::Null, Value::from),
        ),
        (
            "categories",
            video_info.get("categories").cloned().unwrap_or(Value::Null),
        ),
        ("subtitles", Value::Null),
    ]);
    let mut info = dict_info(page, (NAME, NAME), fields, formats);
    info.subtitles = subtitles;
    Ok(info)
}

/// `strip_or_none` for the preloaded description.
fn stripped(value: Option<&Value>) -> Value {
    match value.and_then(Value::as_str).map(str::trim) {
        Some(text) if !text.is_empty() => Value::from(text),
        _ => Value::Null,
    }
}

impl Extractor for CondeNast {
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

        if let Some(video_id) = found.name("id") {
            let params: Params = vec![
                ("videoId".into(), Some(video_id.as_str().to_owned())),
                (
                    "playerId".into(),
                    found.name("player_id").map(|m| m.as_str().to_owned()),
                ),
                (
                    "target".into(),
                    found.name("target").map(|m| m.as_str().to_owned()),
                ),
            ];
            return extract_video(client, url, &params, None).map(Some);
        }

        let display_id = found["display_id"].to_owned();
        if &found["type"] == "series" {
            return Err(unsupported("series pages"));
        }
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let preloaded = PRELOADED_STATE
            .captures(&webpage)
            .and_then(|found| serde_json::from_str::<Value>(&found[1]).ok())
            .and_then(|state| state.pointer("/transformed/video").cloned())
            .filter(|video| video.get("id").is_some());
        match preloaded {
            Some(video) => {
                let params: Params = vec![(
                    "videoId".into(),
                    video.get("id").and_then(Value::as_str).map(str::to_owned),
                )];
                let description = stripped(video.get("description"));
                extract_video(client, url, &params, Some(description)).map(Some)
            }
            None => {
                let params = extract_video_params(&webpage, &display_id)?;
                extract_video(client, url, &params, None).map(Some)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://video.wired.com/watch/3d-printed-speakers-lit-with-led"
        ));
        assert!(suitable(
            "http://player.cnevids.com/embedjs/55f9cf8b61646d1acf00000c/5511d76261646d5566020000.js"
        ));
        assert!(suitable(
            "https://player.cnevids.com/inline/video/59138decb57ac36b83000005.js?target=js-cne-player"
        ));
        assert!(!suitable("https://www.example.com/watch/abc"));
    }
}
