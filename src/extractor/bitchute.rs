//! Upstream `BitChuteIE` (`yt_dlp/extractor/bitchute.py`): the beta API's
//! `video/media` URL (HLS, or a progressive file checked across upstream's
//! seed hosts), the `video` and `channel` metadata (non-fatal) and the API's
//! geo-restriction errors. `BitChuteChannelIE`: the old site's channel or
//! playlist page (title, description, video count) and its `extend/` pages
//! (`OnDemandPagedList`, 25 per page) of `url_transparent` entries.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BitChute;

const NAME: &str = "BitChute";
/// `_check_format`'s alternative seed hosts (the URL's own first).
const SEEDS: [&str; 16] = [
    "seed122",
    "seed125",
    "seed126",
    "seed128",
    "seed132",
    "seed150",
    "seed151",
    "seed152",
    "seed153",
    "seed167",
    "seed171",
    "seed177",
    "seed305",
    "seed307",
    "seedp29xb",
    "zb10-7gsop1v78",
];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:(?:www|old)\.)?bitchute\.com/(?:video|embed|torrent/[^/?#]+)/(?P<id>[^/?#&]+)")
        .unwrap()
});
static SEED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(https?://)(seed\d+)(\.bitchute\.com)").unwrap());

/// `BitChuteIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// `_call_api(endpoint, data, display_id, fatal)`: `Ok(None)` after a
/// non-fatal failure's warning.
fn call_api(
    client: &Client,
    endpoint: &str,
    data: &Value,
    id: &str,
    fatal: bool,
) -> Result<Option<Value>> {
    use std::io::Read;
    let note = endpoint.rsplit('/').next().unwrap_or(endpoint);
    let url = Url::parse(&format!("https://api.bitchute.com/api/beta/{endpoint}"))?;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading {note} API JSON"));
    let what = format!("{note} API JSON");
    let failure = |message: String| -> Result<Option<Value>> {
        if fatal {
            return Err(fail(id, &message));
        }
        // `report_warning(e.msg)`: the message carries the ID.
        crate::screen::warning(&format!("[{NAME}] {id}: {message}"));
        Ok(None)
    };
    let response = super::direct::post_expecting(
        client,
        &url,
        &[
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
        ],
        crate::output::python_json_dumps(data),
        &[403],
    );
    let mut response = match response {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            let text = super::request_error_text(NAME, id, &what, &error);
            let message = text
                .split_once(": ")
                .map_or(text.as_str(), |(_, rest)| rest)
                .to_owned();
            return failure(message);
        }
        Err(error) => return Err(error),
    };
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    if status.as_u16() == 403 {
        let errors: Vec<String> = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|body| body.get("errors").and_then(Value::as_array).cloned())
            .into_iter()
            .flatten()
            .filter(|error| error.get("context").and_then(Value::as_str) == Some("reason"))
            .filter_map(|error| {
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        let errors = errors.join(". ");
        if !errors.is_empty() && errors.contains("location") {
            // `raise_geo_restricted(errors)`.
            return Err(crate::geo::restricted(&fail(id, &errors).to_string(), &[]));
        }
        let reason = status.canonical_reason().unwrap_or("Forbidden");
        return failure(format!(
            "Unable to download {what}: HTTP Error 403: {reason} (caused by <HTTPError 403: {reason}>)"
        ));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| fail(id, &format!("Failed to parse JSON  (caused by {error})")))
}

/// `_check_format(video_url, video_id)`: the first seed host answering a HEAD.
fn check_format(client: &Client, media_url: &str, id: &str) -> Result<Option<Map<String, Value>>> {
    let mut urls: Vec<String> = vec![media_url.to_owned()];
    for seed in SEEDS {
        let url = SEED
            .replace(media_url, |found: &regex::Captures<'_>| {
                format!("{}{seed}{}", &found[1], &found[3])
            })
            .into_owned();
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    for url in urls {
        crate::screen::screen(&format!("[{NAME}] {id}: Checking {url}"));
        let Ok(parsed) = Url::parse(&url) else {
            continue;
        };
        match super::direct::head_with_headers(client, &parsed, &[]) {
            Ok(response) => {
                let mut format = Map::new();
                format.insert("url".into(), url.clone().into());
                format.insert(
                    "filesize".into(),
                    response
                        .headers()
                        .get(reqwest::header::CONTENT_LENGTH)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .into(),
                );
                return Ok(Some(format));
            }
            Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
                crate::screen::screen(&format!(
                    "[{NAME}] {id}: URL is invalid, skipping: {}",
                    crate::upstream_error_text(&error)
                ));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

impl Extractor for BitChute {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_GEO_BYPASS = False`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&[])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let data = json!({"video_id": video_id});
        let media = call_api(client, "video/media", &data, &video_id, true)?.unwrap_or_default();
        let Some(media_url) = media.get("media_url") else {
            return Err(fail(
                &video_id,
                &format!(
                    "An extractor error has occurred. (caused by KeyError('media_url')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        };
        let media_url = media_url.as_str().unwrap_or_default().to_owned();
        let mut formats: Vec<crate::VideoFormat> = Vec::new();
        if super::direct::determine_ext(&media_url).as_deref() == Some("m3u8") {
            let manifest = Url::parse(&media_url)?;
            let mut stream =
                super::extract_m3u8_fatal(client, &manifest, NAME, &video_id, Some("hls"), false)?;
            super::hls::live_format_ids(&mut stream.formats, Some("hls"));
            formats.extend(stream.formats);
        } else {
            let map = if super::check_formats_never() {
                let mut map = Map::new();
                map.insert("url".into(), media_url.clone().into());
                Some(map)
            } else {
                check_format(client, &media_url, &video_id)?
            };
            if let Some(mut map) = map {
                let keys: Vec<String> = map.keys().cloned().collect();
                map.retain(|_, value| !value.is_null());
                let ext = map
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(super::direct::determine_ext)
                    .unwrap_or_else(|| "unknown_video".into());
                map.insert("ext".into(), ext.into());
                map.insert("format_id".into(), "".into());
                let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                    .map_err(|error| fail(&video_id, &error.to_string()))?;
                crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
                formats.push(format);
            }
        }
        if formats.is_empty() {
            crate::no_formats::raise_expected(
                NAME,
                Some(&video_id),
                None,
                "Video is unavailable. Please make sure this video is playable in the browser before reporting this issue.",
            )?;
        }
        let video = call_api(client, "video", &data, &video_id, false)?;
        let channel = match video
            .as_ref()
            .and_then(|video| video.pointer("/channel/channel_id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            Some(channel_id) => call_api(
                client,
                "channel",
                &json!({"channel_id": channel_id}),
                &video_id,
                false,
            )?,
            None => None,
        };
        let video = video.unwrap_or(Value::Null);
        let text = |value: &Value, path: &str| {
            value
                .pointer(path)
                .filter(|value| value.is_string())
                .cloned()
        };
        let uploader_url = |value: &Value| {
            value
                .get("profile_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(|id| Value::from(format!("https://www.bitchute.com/profile/{id}/")))
        };
        let mut dict = Map::new();
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                dict.insert(key.into(), value);
            }
        };
        put("title", text(&video, "/video_name"));
        put("description", text(&video, "/description"));
        put(
            "thumbnail",
            video
                .get("thumbnail_url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(Value::from),
        );
        put("channel", text(&video, "/channel/channel_name"));
        put("channel_id", text(&video, "/channel/channel_id"));
        put(
            "channel_url",
            video
                .pointer("/channel/channel_url")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .and_then(|path| {
                    Url::parse("https://www.bitchute.com/")
                        .ok()?
                        .join(path)
                        .ok()
                })
                .map(|url| Value::from(url.as_str())),
        );
        put("uploader_id", text(&video, "/profile_id"));
        put("uploader_url", uploader_url(&video));
        put(
            "timestamp",
            video
                .get("date_published")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map(Value::from),
        );
        put(
            "duration",
            video
                .get("duration")
                .and_then(Value::as_str)
                .and_then(crate::parse_duration)
                .and_then(|duration| serde_json::to_value(duration).ok()),
        );
        let tags: Vec<Value> = video
            .get("hashtags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|tag| tag.as_str().is_some_and(|tag| !tag.is_empty()))
            .cloned()
            .collect();
        put("tags", (!tags.is_empty()).then_some(Value::Array(tags)));
        put(
            "view_count",
            video.get("view_count").and_then(|count| {
                count
                    .as_i64()
                    .or_else(|| count.as_f64().map(|count| count as i64))
                    .or_else(|| count.as_str()?.trim().parse().ok())
                    .map(Value::from)
            }),
        );
        put(
            "is_live",
            Some(Value::Bool(
                video.get("state_id").and_then(Value::as_str) == Some("live"),
            )),
        );
        if let Some(channel) = &channel {
            put("channel", text(channel, "/channel_name"));
            put("channel_id", text(channel, "/channel_id"));
            put(
                "channel_url",
                channel
                    .get("url_slug")
                    .and_then(Value::as_str)
                    .filter(|slug| !slug.is_empty())
                    .map(|slug| Value::from(format!("https://www.bitchute.com/channel/{slug}/"))),
            );
            put("uploader", text(channel, "/profile_name"));
            put("uploader_id", text(channel, "/profile_id"));
            put("uploader_url", uploader_url(channel));
        }
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("formats".into(), json!([]));

        let first = formats
            .first()
            .map_or_else(|| page.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        info.formats = formats;
        info.webpage_url = page.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let titled = dict.contains_key("title");
        if !titled {
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        let mut keys = Vec::new();
        for (key, value) in dict {
            keys.push(key.clone());
            match key.as_str() {
                "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
                "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                "formats" => {}
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!(
                "dict{};{}",
                if titled { "" } else { "-untitled" },
                keys.join(",")
            ),
        ))))
    }
}

pub struct BitChuteChannel;

const CHANNEL_NAME: &str = "BitChuteChannel";
/// `BitChuteChannelIE._TOKEN`.
const TOKEN: &str = "zyG6tQcGPE5swyAEFLqKUwMuMMuF6IO2DZ6ZDQjGfsL0e4dcTLwqkTTul05Jdve7";
/// `BitChuteChannelIE.PAGE_SIZE`.
const PAGE_SIZE: usize = 25;

static CHANNEL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www|old)\.)?bitchute\.com/(?P<type>channel|playlist)/(?P<id>[^/?#&]+)",
    )
    .unwrap()
});
static VIDEO_HREF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a\s[^>]*\bhref=["']/video/([^"'/]+)"#).unwrap());
static VIDEO_COUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<span>(\d+)\s+videos?</span>").unwrap());

/// `BitChuteChannelIE.suitable`.
pub fn channel_suitable(url: &str) -> bool {
    CHANNEL_URL.is_match(url)
}

/// `HTML_CLASS_NAMES[playlist_type]`: the container, title and
/// description classes.
fn class_names(playlist_type: &str) -> (&'static str, &'static str, &'static str) {
    if playlist_type == "channel" {
        (
            "channel-videos-container",
            "channel-videos-title",
            "channel-videos-text",
        )
    } else {
        ("playlist-video", "title", "description")
    }
}

/// `_make_url(playlist_id, playlist_type)`.
fn make_url(playlist_id: &str, playlist_type: &str) -> String {
    format!("https://old.bitchute.com/{playlist_type}/{playlist_id}/")
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("BitChute {what} are not supported natively"))
}

/// Upstream `get_element_by_class(class_name, html)`: the first element's
/// unescaped content.
fn element_by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(text, _)| text)
}

/// `url_result(url, BitChuteIE, video_id, url_transparent=True, **fields)`.
fn url_entry(target: &str, video_id: &str, fields: Vec<(&str, Value)>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = video_id.to_owned();
    let mut raw = Map::new();
    let mut overlay = Map::new();
    for (key, value) in fields {
        if key == "title"
            && let Some(title) = value.as_str()
        {
            info.title = title.to_owned();
        }
        raw.insert(key.into(), value.clone());
        overlay.insert(key.into(), value);
    }
    raw.insert("ie_key".into(), NAME.into());
    raw.insert("id".into(), video_id.into());
    raw.insert("_type".into(), "url_transparent".into());
    raw.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": overlay,
            "ie_key": NAME,
            "extractor": NAME,
            "entry": Value::Object(raw),
        }),
    );
    Ok(info)
}

/// Whether a playlist's pending state is a BitChute channel's or playlist's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == "bitchute"
}

/// `_download_json(f'{playlist_url}extend/', ..., data=urlencode_postdata(...))`.
fn post_page(client: &Client, playlist_url: &str, playlist_id: &str, page: usize) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!(
        "[{CHANNEL_NAME}] {playlist_id}: Downloading page {page}"
    ));
    let url = Url::parse(&format!("{playlist_url}extend/"))?;
    let offset = (page * PAGE_SIZE).to_string();
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("csrfmiddlewaretoken", TOKEN),
            ("name", ""),
            ("offset", offset.as_str()),
        ])
        .finish();
    let cookie = format!("csrftoken={TOKEN}");
    let mut response = super::direct::post(
        client,
        &url,
        &[
            (
                "Content-Type",
                "application/x-www-form-urlencoded; charset=UTF-8",
            ),
            ("Referer", playlist_url),
            ("X-Requested-With", "XMLHttpRequest"),
            ("Cookie", cookie.as_str()),
        ],
        body,
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: CHANNEL_NAME,
            id: playlist_id.to_owned(),
            what: "JSON metadata",
            source: Box::new(error),
        },
        error => error,
    })?;
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    let bytes = super::decode_content(response.headers(), bytes).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&bytes, response.headers());
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{CHANNEL_NAME}] {playlist_id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_fetch_page(playlist_id, playlist_type, page_num)`: the page's videos as
/// `url_transparent` entries (none once the response is not `success`).
fn fetch_page(client: &Client, state: &Value, page: usize) -> Result<Vec<VideoInfo>> {
    let playlist_id = state["playlist_id"].as_str().unwrap_or_default();
    let playlist_type = state["playlist_type"].as_str().unwrap_or_default();
    let playlist_url = make_url(playlist_id, playlist_type);
    let data = post_page(client, &playlist_url, playlist_id, page)?;
    let Value::Object(data) = data else {
        return Err(unsupported("page responses that are not objects"));
    };
    if !data.get("success").is_some_and(super::nuxt::truthy) {
        return Ok(Vec::new());
    }
    let Some(Value::String(html)) = data.get("html") else {
        return Err(unsupported("page responses without HTML"));
    };
    let (container, title_class, description_class) = class_names(playlist_type);
    let videos = crate::html_elements::elements_text_and_html_by_class(container, html)
        .map_err(|error| Error::Reported(format!("[{CHANNEL_NAME}] {playlist_id}: {error}")))?;
    let mut entries = Vec::new();
    for (_, video_html) in videos {
        let Some(video_id) = VIDEO_HREF
            .captures(&video_html)
            .map(|found| found[1].to_owned())
        else {
            continue;
        };
        let cleaned = |class: &str| {
            element_by_class(class, &video_html)
                .map(|text| crate::html::clean_html(&text))
                .map_or(Value::Null, Value::from)
        };
        let duration = element_by_class("video-duration", &video_html)
            .and_then(|text| crate::parse_duration(&text))
            .and_then(|duration| serde_json::to_value(duration).ok())
            .unwrap_or(Value::Null);
        let view_count = element_by_class("video-views", &video_html)
            .and_then(|text| super::youtube::dict::parse_count(&crate::html::clean_html(&text)))
            .map_or(Value::Null, Value::from);
        entries.push(url_entry(
            &format!("https://www.bitchute.com/video/{video_id}"),
            &video_id,
            vec![
                ("title", cleaned(title_class)),
                ("description", cleaned(description_class)),
                ("duration", duration),
                ("view_count", view_count),
            ],
        )?);
    }
    Ok(entries)
}

/// `OnDemandPagedList(page_func, 25)` read by upstream's `PlaylistEntries`
/// (see [`super::paged`]).
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let page_state = state.clone();
    let mut fetch = |page| fetch_page(client, &page_state, page);
    let placeholder = || url_entry("https://www.bitchute.com/video/None", "None", Vec::new());
    super::paged::fetch_requested(state, limit, entries, PAGE_SIZE, &mut fetch, &placeholder)
}

impl Extractor for BitChuteChannel {
    fn key(&self) -> &'static str {
        CHANNEL_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CHANNEL_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let playlist_type = found["type"].to_owned();
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(CHANNEL_NAME);
        crate::screen::screen(&format!(
            "[{CHANNEL_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let webpage = super::download_webpage(
            client,
            &Url::parse(&make_url(&playlist_id, &playlist_type))?,
            CHANNEL_NAME,
            &playlist_id,
        )?;
        let title = crate::html::html_extract_title(&webpage);
        let description = ["description", "og:description", "twitter:description"]
            .iter()
            .find_map(|prop| crate::html::html_search_meta(&webpage, prop));
        // `int_or_none(_html_search_regex(r'<span>(\d+)\s+videos?</span>'))`.
        let playlist_count = VIDEO_COUNT
            .captures(&webpage)
            .and_then(|found| crate::html::clean_html(&found[1]).parse::<u64>().ok());

        let mut extra = Map::new();
        let title = match title {
            Some(title) => title,
            None => {
                extra.insert("title".into(), Value::Null);
                String::new()
            }
        };
        extra.insert(
            "description".into(),
            description.map_or(Value::Null, Value::from),
        );
        extra.insert(
            "playlist_count".into(),
            playlist_count.map_or(Value::Null, Value::from),
        );
        // `OnDemandPagedList`, whose full count, like a generator's, is
        // unknown unless a selection reads past its end.
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;title,description,playlist_count,id{}",
                crate::info_order::GENERATOR
            )
            .into(),
        );
        let state = json!({
            "kind": "bitchute",
            "playlist_id": playlist_id,
            "playlist_type": playlist_type,
            "random_access": true,
        });
        Ok(Some(ExtractResult::Playlist(crate::PlaylistInfo {
            id: playlist_id,
            title,
            webpage_url: page.clone(),
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
    fn channel_urls_follow_upstream_valid_url() {
        for url in [
            "https://www.bitchute.com/channel/bitchute/",
            "https://www.bitchute.com/playlist/wV9Imujxasw9/",
            "https://old.bitchute.com/playlist/wV9Imujxasw9/",
        ] {
            assert!(channel_suitable(url), "{url}");
            assert!(!suitable(url), "{url}");
        }
        assert!(!channel_suitable(
            "https://www.bitchute.com/video/UGlrF9o9b-Q/"
        ));
    }

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.bitchute.com/video/UGlrF9o9b-Q/"));
        assert!(suitable("https://old.bitchute.com/embed/lbb5G1hjPhw/"));
        assert!(suitable(
            "https://www.bitchute.com/torrent/Zee5BE49045h/szoMrox2JEI.webtorrent"
        ));
        assert!(!suitable("https://www.bitchute.com/channel/livesonnet/"));
    }
}
