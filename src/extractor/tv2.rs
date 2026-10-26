//! Upstream `TV2IE` and `TV2ArticleIE` (`yt_dlp/extractor/tv2.py`): a
//! tv2.no video's `sumo` asset metadata and per-protocol (HLS, DASH)
//! playback streams, each URL checked like `_is_valid_url`, with upstream's
//! 401 geo/login/description errors; and an article's embedded assets as a
//! playlist. Upstream's Katsomo and MTV Uutiset extractors are marked
//! broken and not ported.
use super::clyp::{float_or_none, key_error};
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_json_note, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Tv2;
pub struct Tv2Article;

const NAME: &str = "TV2";
const ARTICLE_NAME: &str = "TV2Article";
const PROTOCOLS: [&str; 2] = ["HLS", "DASH"];
const GEO_COUNTRIES: [&str; 1] = ["NO"];
const PLAY_BODY: &str = r#"{"device":{"id":"1-1-1","name":"Nettleser (HTML)"}}"#;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?tv2\.no/v(?:ideo)?\d*/(?:[^?#]+/)*(?P<id>\d+)").unwrap()
});
// Upstream's `(?!v(?:ideo)?\d*/)` after the host is checked in `article_id`.
static ARTICLE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?tv2\.no/(?P<rest>[^?#]+/(?P<id>\d+))").unwrap()
});
static VIDEO_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^v(?:ideo)?\d*/").unwrap());
static ASSET_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-assetid=["'](\d+)"#).unwrap());
static CONTENTBOX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)(?:TV2ContentboxVideo|TV2\.TV2Video)\((\{.+?\})\)").unwrap());

/// `TV2IE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `TV2ArticleIE._match_valid_url`.
pub fn article_suitable(url: &str) -> bool {
    article_id(url).is_some()
}

fn article_id(url: &str) -> Option<String> {
    let found = ARTICLE_URL.captures(url)?;
    // `[^?#]+/(\d+)` backtracks to the last `/digits`; the negative
    // lookahead applies to what follows the host.
    (!VIDEO_PREFIX.is_match(&found["rest"])).then(|| found["id"].to_owned())
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("TV 2 {what} are not supported natively"))
}

/// `strip_or_none`.
fn strip_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => Value::from(text.trim()),
        _ => Value::Null,
    }
}

/// `x.get(a) or x.get(b)`.
fn either<'a>(asset: &'a Value, first: &str, second: &str) -> Option<&'a Value> {
    asset
        .get(first)
        .filter(|value| super::nuxt::truthy(value))
        .or_else(|| asset.get(second))
}

/// `_download_json(play URL, data=..., headers={'content-type': ...})['playback']`,
/// with upstream's handling of a 401 error body.
fn playback(client: &Client, video_id: &str, protocol: &str) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading playabck JSON"));
    let url = Url::parse(&format!(
        "https://api.sumo.tv2.no/play/{video_id}?stream={protocol}"
    ))?;
    let mut response = super::direct::post_expecting(
        client,
        &url,
        &[("content-type", "application/json")],
        PLAY_BODY.to_owned(),
        &[401],
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: NAME,
            id: video_id.to_owned(),
            what: "JSON metadata",
            source: Box::new(error),
        },
        error => error,
    })?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    if status == 401 {
        let data: Value = crate::json::from_str(&text).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let error = data
            .get("error")
            .ok_or_else(|| key_error(video_id, "error"))?;
        match error.get("code").and_then(Value::as_str) {
            Some("ASSET_PLAYBACK_INVALID_GEO_LOCATION") => {
                return Err(crate::geo::restricted(
                    &format!(
                        "[{NAME}] {video_id}: This video is not available from your location due to geo restriction"
                    ),
                    &GEO_COUNTRIES,
                ));
            }
            Some("SESSION_NOT_AUTHENTICATED") => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: {}",
                    super::login::login_required_message(
                        super::login::LOGIN_REQUIRED,
                        None,
                        Some(super::login::Hint::Cookies),
                    )
                )));
            }
            _ => {}
        }
        let description = error
            .get("description")
            .ok_or_else(|| key_error(video_id, "description"))?
            .as_str()
            .ok_or_else(|| unsupported("error descriptions that are not strings"))?;
        return Err(Error::Reported(format!(
            "[{NAME}] {video_id}: {description}; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    let data: Value = crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
        ))
    })?;
    data.get("playback")
        .cloned()
        .ok_or_else(|| key_error(video_id, "playback"))
}

impl Extractor for Tv2 {
    fn key(&self) -> &'static str {
        NAME
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&GEO_COUNTRIES)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!("https://sumo.tv2.no/rest/assets/{video_id}"))?;
        let asset = download_json_note(client, &api, NAME, &video_id, "Downloading metadata JSON")?;
        let title = asset
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "title"))?;
        let is_live = asset.get("live") == Some(&Value::Bool(true));
        let mut formats = Vec::new();
        let mut format_urls: Vec<String> = Vec::new();
        let mut data = Value::Null;
        // Upstream returns the last stream's `video_url` as `url`; without
        // any stream it is unbound.
        let mut last_url: Option<Value> = None;
        for protocol in PROTOCOLS {
            data = playback(client, &video_id, protocol)?;
            let items = match data.get("streams") {
                None => Vec::new(),
                Some(Value::Array(items)) => items.clone(),
                Some(_) => return Err(unsupported("stream lists that are not arrays")),
            };
            for item in &items {
                last_url = Some(item.get("url").cloned().unwrap_or(Value::Null));
                let Some(video_url) = item
                    .get("url")
                    .filter(|value| super::nuxt::truthy(value))
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| unsupported("stream URLs that are not strings"))
                    })
                    .transpose()?
                else {
                    continue;
                };
                if format_urls.contains(&video_url) {
                    continue;
                }
                let kind = match item.get("type") {
                    None | Some(Value::Null) => "None".to_owned(),
                    Some(Value::String(kind)) => kind.clone(),
                    Some(other) => other.to_string(),
                };
                let format_id = format!("{}-{kind}", protocol.to_lowercase());
                if !super::is_valid_url(client, &video_url, NAME, &video_id, &format_id)? {
                    continue;
                }
                format_urls.push(video_url.clone());
                let ext = super::direct::determine_ext(&video_url);
                let drm = data.get("drmProtected").is_some_and(super::nuxt::truthy);
                match ext.as_deref() {
                    Some("f4m") => {
                        let manifest = Url::parse(&video_url)?;
                        formats.extend(super::extract_f4m_optional(
                            client,
                            NAME,
                            &manifest,
                            &video_id,
                            Some(&format_id),
                        )?);
                    }
                    Some("m3u8") => {
                        if !drm {
                            let manifest = Url::parse(&video_url)?;
                            if let Some(stream) = super::extract_m3u8_master_optional(
                                client,
                                &manifest,
                                NAME,
                                &video_id,
                                Some(&format_id),
                                false,
                                is_live,
                            ) {
                                formats.extend(stream.formats);
                            }
                        }
                    }
                    Some("mpd") => {
                        let manifest = Url::parse(&video_url)?;
                        if let Some(stream) = super::extract_mpd_optional_note(
                            client,
                            &manifest,
                            NAME,
                            &video_id,
                            Some(&format_id),
                            "Downloading MPD manifest",
                        ) {
                            formats.extend(stream.formats);
                        }
                    }
                    Some("ism") => {}
                    _ if video_url.ends_with(".ism/Manifest") => {}
                    _ => formats.push(dict_format(
                        vec![("url", video_url.into()), ("format_id", format_id.into())],
                        NAME,
                        &video_id,
                    )?),
                }
            }
        }
        if formats.is_empty() && data.get("drmProtected").is_some_and(super::nuxt::truthy) {
            // `report_drm`.
            crate::no_formats::raise_expected(
                NAME,
                Some(&video_id),
                None,
                "This video is DRM protected",
            )?;
        }
        let Some(last_url) = last_url else {
            return Err(Error::Reported(
                "cannot access local variable 'video_url' where it is not associated with a value"
                    .into(),
            ));
        };
        let thumbnails: Vec<Value> = match asset.get("images") {
            Some(images) if super::nuxt::truthy(images) => images
                .as_object()
                .ok_or_else(|| unsupported("image maps that are not objects"))?
                .iter()
                .map(|(kind, url)| json!({"id": kind, "url": url}))
                .collect(),
            _ => Vec::new(),
        };
        let categories = match asset.get("tags") {
            None => vec![Value::from("")],
            Some(Value::String(tags)) => tags.split(',').map(Value::from).collect(),
            Some(_) => return Err(unsupported("tag lists that are not strings")),
        };
        let timestamp = either(&asset, "live_broadcast_time", "update_time")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("url", last_url),
                ("title", title),
                ("description", strip_or_none(asset.get("description"))),
                ("thumbnails", Value::Array(thumbnails)),
                ("timestamp", timestamp),
                (
                    "duration",
                    float_or_none(either(&asset, "accurateDuration", "duration")),
                ),
                ("view_count", int_or_none(asset.get("views"))),
                ("categories", Value::Array(categories)),
                ("formats", Value::Null),
                ("is_live", is_live.into()),
            ],
            formats,
        );
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=id,url");
        }
        Ok(Some(info))
    }
}

impl Extractor for Tv2Article {
    fn key(&self) -> &'static str {
        ARTICLE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(playlist_id) = article_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(ARTICLE_NAME);
        crate::screen::screen(&format!(
            "[{ARTICLE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, ARTICLE_NAME, &playlist_id)?;
        // The old embed pattern, else the new one.
        let mut assets: Vec<String> = ASSET_ID
            .captures_iter(&webpage)
            .map(|found| found[1].to_owned())
            .collect();
        if assets.is_empty() {
            for found in CONTENTBOX.captures_iter(&webpage) {
                let source = crate::js_to_json::js_to_json(&found[1]);
                // `_parse_json(..., fatal=False)`: Python's decoder message.
                if let Some(error) = crate::json::python_decode_error_text(&source) {
                    crate::screen::warning(&format!(
                        "[{ARTICLE_NAME}] {playlist_id}: Failed to parse JSON: {error}"
                    ));
                    continue;
                }
                let Ok(video) = crate::json::from_str::<Value>(&source) else {
                    continue;
                };
                match video.get("assetId") {
                    Some(Value::String(asset)) if !asset.is_empty() => assets.push(asset.clone()),
                    Some(Value::Number(asset)) if asset.as_f64() != Some(0.0) => {
                        assets.push(asset.to_string());
                    }
                    _ => {}
                }
            }
        }
        let mut entries = Vec::new();
        for asset in assets {
            let target = format!("http://www.tv2.no/v/{asset}");
            let entry_url = Url::parse(&target)?;
            let mut entry = super::direct::build_at(entry_url.clone(), &entry_url, String::new());
            entry.formats.clear();
            entry.direct = None;
            entry.extra.insert(
                crate::TRANSPARENT.into(),
                json!({
                    "url": target,
                    "force_videoid": null,
                    "fields": {},
                    "ie_key": NAME,
                    "extractor": NAME,
                    "entry": {"ie_key": NAME, "_type": "url", "url": target},
                }),
            );
            entries.push(entry);
        }
        let strip = |text: String| match text.strip_suffix(" - TV2.no") {
            Some(stripped) => stripped.to_owned(),
            None => text,
        };
        let title = crate::html::og_search_property(&webpage, "title").map(strip);
        if title.is_none() {
            super::search_warning(ARTICLE_NAME, "OpenGraph title");
        }
        let description = crate::html::og_search_property(&webpage, "description").map(strip);
        if description.is_none() {
            super::search_warning(ARTICLE_NAME, "OpenGraph description");
        }
        let title = title.unwrap_or_default();
        let mut keys = vec!["id"];
        if !title.is_empty() {
            keys.push("title");
        }
        let mut extra = Map::new();
        if let Some(description) = description {
            keys.push("description");
            extra.insert("description".into(), description.into());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title,
            webpage_url: url.clone(),
            extractor: ARTICLE_NAME.into(),
            extractor_key: ARTICLE_NAME.into(),
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
        assert!(suitable("http://www.tv2.no/v/1791207/"));
        assert!(suitable(
            "https://www.tv2.no/video/nyhetene/her-kolliderer-romsonden/1234567"
        ));
        assert!(suitable("https://tv2.no/v2/sport/1234567"));
        assert!(!suitable("https://www.tv2.no/a/6930542"));
        assert!(article_suitable("https://www.tv2.no/a/6930542"));
        assert!(article_suitable(
            "http://www.tv2.no/nyheter/innenriks/6930542"
        ));
        assert!(!article_suitable("http://www.tv2.no/v/1791207/"));
        assert!(!article_suitable("https://www.tv2.no/video/sport/1234567"));
    }
}
