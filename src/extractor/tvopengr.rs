//! Upstream `TVOpenGrBaseIE`, `TVOpenGrWatchIE` and `TVOpenGrEmbedIE`
//! (`yt_dlp/extractor/tvopengr.py`): tvopen.gr and ethnos.gr pages (and
//! their embeds) resolve to the canonical tvopen.gr watch page, whose
//! `VideoObject` JSON-LD is merged with the player API's stream, HLS and
//! DASH formats.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage, follow_url};
use crate::{Error, ExtractResult, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct TvOpenGrWatch;
pub struct TvOpenGrEmbed;

const WATCH_NAME: &str = "tvopengr:watch";
const WATCH_KEY: &str = "TVOpenGrWatch";
const EMBED_NAME: &str = "tvopengr:embed";
const EMBED_KEY: &str = "TVOpenGrEmbed";
const API_ENDPOINT: &str = "https://www.tvopen.gr/templates/data/player";

static WATCH_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?P<netloc>(?:www\.)?(?:tvopen|ethnos)\.gr)/watch/(?P<id>\d+)/(?P<slug>[^/]+)",
    )
    .unwrap()
});
static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:https?:)?//(?:www\.|cdn\.|)(?:tvopen|ethnos).gr/embed/(?P<id>\d+)").unwrap()
});

/// `TVOpenGrWatchIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    WATCH_URL.is_match(url)
}

/// `TVOpenGrEmbedIE._match_valid_url`.
pub fn embed_suitable(url: &str) -> bool {
    EMBED_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("TVOpenGr {what} are not supported natively"))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `_return_canonical_url`: the page's `og:url`, followed by its extractor.
fn return_canonical(
    client: &Client,
    url: &Url,
    id: &str,
    name: &'static str,
) -> Result<ExtractResult> {
    let webpage = download_webpage(client, url, name, id)?;
    let canonical = crate::html::og_search_property(&webpage, "url")
        .ok_or_else(|| unsupported("pages without an OpenGraph URL"))?;
    Ok(follow_url(url, canonical))
}

/// `_search_json_ld(webpage, video_id, expected_type='VideoObject')`: the
/// first `VideoObject` of the (`@graph`-expanded) JSON-LD, else `{}`.
fn video_object(webpage: &str) -> Map<String, Value> {
    for item in super::json_ld::yield_json_ld(webpage) {
        // Only top-level dicts with an `@context` are considered.
        let Some(context) = item.get("@context").cloned() else {
            continue;
        };
        let graph = item.as_object().is_some_and(|object| {
            object.len() == 2 && object.contains_key("@context") && object.contains_key("@graph")
        });
        let candidates: Vec<Value> = if graph {
            match &item["@graph"] {
                Value::Array(children) => children.clone(),
                other => vec![other.clone()],
            }
        } else {
            vec![item]
        };
        for mut candidate in candidates {
            if !candidate.is_object() {
                continue;
            }
            if !is_video_object(&candidate) {
                continue;
            }
            // `_json_ld` only needs `@context` at the top level.
            if let Some(object) = candidate.as_object_mut() {
                object.entry("@context").or_insert_with(|| context.clone());
            }
            return super::json_ld::json_ld(&candidate);
        }
    }
    Map::new()
}

fn is_video_object(value: &Value) -> bool {
    match value.get("@type") {
        Some(Value::String(kind)) => kind == "VideoObject",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "VideoObject"),
        _ => false,
    }
}

/// `re.sub(r'(?<=/imgHandler/)\d+', width, url)`: the regex crate has no
/// look-behind, so the marker is matched instead.
fn with_width(url: &str, width: u32) -> String {
    const MARKER: &str = "/imgHandler/";
    let mut out = String::new();
    let mut rest = url;
    while let Some(position) = rest.find(MARKER) {
        let end = position + MARKER.len();
        out.push_str(&rest[..end]);
        rest = &rest[end..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            out.push_str(&width.to_string());
            rest = &rest[digits..];
        }
    }
    out.push_str(rest);
    out
}

/// `scale_thumbnails_to_max_format_width(formats, thumbnails, r'(?<=/imgHandler/)\d+')`.
fn scale_thumbnails(formats: &[VideoFormat], thumbnails: &Value) -> Result<Value> {
    let max = formats
        .iter()
        .map(|format| (format.width.unwrap_or(0), format.height.unwrap_or(0)))
        .max()
        .unwrap_or((0, 0));
    let Value::Array(items) = thumbnails else {
        return Err(unsupported("thumbnails that are not lists"));
    };
    if max.0 == 0 {
        return Ok(thumbnails.clone());
    }
    let mut scaled = Vec::new();
    for thumbnail in items {
        let Some(object) = thumbnail.as_object() else {
            return Err(unsupported("thumbnails that are not objects"));
        };
        let url = object
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| super::clyp::key_error("thumbnails", "url"))?;
        // `merge_dicts({'url': ...}, {'width': ..., 'height': ...}, thumbnail)`:
        // the first non-`None` value of a key wins.
        let mut merged = Map::new();
        merged.insert("url".into(), with_width(url, max.0).into());
        merged.insert("width".into(), max.0.into());
        merged.insert("height".into(), max.1.into());
        for (key, value) in object {
            if !value.is_null() && !merged.contains_key(key) {
                merged.insert(key.clone(), value.clone());
            }
        }
        scaled.push(Value::Object(merged));
    }
    Ok(Value::Array(scaled))
}

/// `_extract_formats_and_subs`: the `stream`, `httpstream` and `mpegdash`
/// entries of the player API.
fn formats_and_subs(
    client: &Client,
    response: &Value,
    id: &str,
) -> Result<(Vec<VideoFormat>, crate::Subtitles)> {
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    let Some(entries) = response.as_object() else {
        return Err(unsupported("player responses that are not objects"));
    };
    for (format_id, format_url) in entries {
        if !matches!(format_id.as_str(), "stream" | "httpstream" | "mpegdash") {
            continue;
        }
        let Some(format_url) = format_url.as_str() else {
            return Err(unsupported("player sources that are not strings"));
        };
        let manifest = Url::parse(format_url)?;
        match super::direct::determine_ext(format_url).as_deref() {
            Some("m3u8") => {
                if let Some(stream) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    WATCH_NAME,
                    id,
                    Some(format_id),
                    true,
                    false,
                ) {
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            }
            Some("mpd") => {
                // `_extract_mpd_formats_and_subtitles(url, id, 'mp4', fatal=False)`:
                // the `mp4` is the `mpd_id`.
                if let Some(stream) = super::extract_mpd_optional_note(
                    client,
                    &manifest,
                    WATCH_NAME,
                    id,
                    Some("mp4"),
                    "Downloading MPD manifest",
                ) {
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            }
            _ => formats.push(dict_format(
                vec![
                    ("url", format_url.into()),
                    ("format_id", format_id.as_str().into()),
                ],
                WATCH_NAME,
                id,
            )?),
        }
    }
    Ok((formats, subtitles))
}

/// `_real_extract` of the watch page: the JSON-LD `VideoObject` with the
/// player API's formats, thumbnails scaled to the largest format and the
/// page's `description` span.
fn watch(client: &Client, url: &Url, id: &str, slug: &str) -> Result<VideoInfo> {
    let webpage = download_webpage(client, url, WATCH_NAME, id)?;
    let mut info = video_object(&webpage);
    let api = super::update_url_query(&Url::parse(API_ENDPOINT)?, &[("cid", id)]);
    let response = download_json(client, &api, WATCH_NAME, id)?;
    let (formats, subtitles) = formats_and_subs(client, &response, id)?;
    info.insert("formats".into(), Value::Null);
    info.insert("subtitles".into(), Value::Null);
    let thumbnails = info
        .get("thumbnails")
        .cloned()
        .ok_or_else(|| super::clyp::key_error(id, "thumbnails"))?;
    info.insert(
        "thumbnails".into(),
        scale_thumbnails(&formats, &thumbnails)?,
    );
    let elements =
        crate::html_elements::elements_text_and_html_by_attribute("class", "description", &webpage)
            .map_err(|_| unsupported("pages whose HTML does not parse"))?;
    let (text, html) = elements.into_iter().next().ok_or_else(|| {
        Error::Reported(format!(
            "{id}: An extractor error has occurred. (caused by StopIteration()); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    if !text.is_empty() && html.starts_with("<span ") {
        info.insert("description".into(), text.into());
    }
    info.insert("id".into(), id.into());
    info.insert("display_id".into(), slug.into());
    let keys: Vec<&str> = info.keys().map(String::as_str).collect();
    let shape = format!("dict;{};thumbnail=url,width,height", keys.join(","));
    let fields: Vec<(&str, Value)> = info
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    let mut result = dict_info(url, (WATCH_NAME, WATCH_KEY), fields, formats);
    result.subtitles = subtitles;
    Ok(crate::info_order::mark_info(result, &shape))
}

impl Extractor for TvOpenGrWatch {
    fn key(&self) -> &'static str {
        WATCH_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = WATCH_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let netloc = found["netloc"].to_owned();
        let id = found["id"].to_owned();
        let slug = found["slug"].to_owned();
        announce(WATCH_NAME, url);
        if !netloc.contains("tvopen.gr") {
            return return_canonical(client, url, &id, WATCH_NAME).map(Some);
        }
        watch(client, url, &id, &slug).map(|info| Some(ExtractResult::Video(info)))
    }
}

impl Extractor for TvOpenGrEmbed {
    fn key(&self) -> &'static str {
        EMBED_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = EMBED_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(EMBED_NAME, url);
        return_canonical(client, url, &id, EMBED_NAME).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.ethnos.gr/watch/101009/nikoskaprabelosdenexoymekanenanasthenhsemethmethmetallaxhomikron"
        ));
        assert!(suitable(
            "https://www.tvopen.gr/watch/100979/se28099agapaomenalla7cepeisodio267cmhthrargiapashskakias"
        ));
        assert!(!suitable("https://www.tvopen.gr/watch/100979"));
        assert!(embed_suitable("https://cdn.ethnos.gr/embed/100963"));
        assert!(embed_suitable("//www.tvopen.gr/embed/100963"));
    }

    #[test]
    fn thumbnail_width_follows_the_marker() {
        assert_eq!(
            with_width(
                "https://opentv-static.siliconweb.com/imgHandler/1920/a.jpg",
                640
            ),
            "https://opentv-static.siliconweb.com/imgHandler/640/a.jpg"
        );
        assert_eq!(
            with_width("https://a/imgHandler/x/1920.jpg", 640),
            "https://a/imgHandler/x/1920.jpg"
        );
    }
}
