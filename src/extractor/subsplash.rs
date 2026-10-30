//! Upstream `SubsplashBaseIE`, `SubsplashIE` and `SubsplashPlaylistIE`
//! (`yt_dlp/extractor/subsplash.py`): the guest API token (cookie, the page's
//! `x-api-token` header or its embedded tokens), a media item's HLS and direct
//! MP4 formats and metadata, and a series' published items. A series' pages
//! are all fetched up front (upstream fetches them lazily).
use super::{
    Extractor, dict_format, dict_info, download_json_note_headers, extract_m3u8_master_optional,
    search_json, update_url_query,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Subsplash;
pub struct SubsplashPlaylist;

const NAME: &str = "Subsplash";
const PLAYLIST_NAME: &str = "subsplash:playlist";
const PLAYLIST_KEY: &str = "SubsplashPlaylist";
const API: &str = "https://core.subsplash.com/media/v1";
const PAGE_SIZE: usize = 15;
const ITEM_INCLUDE: &str =
    "images,audio.audio-outputs,audio.video,video.video-outputs,video.playlists,document,broadcast";
const SERIES_INCLUDE: &str =
    "images,audio.audio-outputs,audio.video,video.video-outputs,video.playlists,document";

static VALID_URL: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(
            r"^https?://(?:www\.)?subsplash\.com/(?:u/)?[^/?#]+/[^/?#]+/(?:d/|mi/\+)(?P<id>\w+)",
        )
        .unwrap(),
        Regex::new(r"^https?://(?:\w+\.)?subspla\.sh/(?P<id>\w+)").unwrap(),
    ]
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?subsplash\.com/[^/?#]+/(?:our-videos|media)/ms/\+(?P<id>\w+)")
        .unwrap()
});
static SHOEBOX_TOKENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script[^>]+\bid="shoebox-tokens"[^>]*>"#).unwrap());
static GUEST_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\\"tokens\\":\{\\"guest\\":\\"([A-Za-z0-9._-]+)\\""#).unwrap());

/// `_match_valid_url` for the single-item URLs.
fn item_id(url: &str) -> Option<String> {
    VALID_URL
        .iter()
        .find_map(|pattern| pattern.captures(url))
        .map(|found| found["id"].to_owned())
}

/// `_match_valid_url` for the series URLs (`SubsplashPlaylistIE`).
fn series_id(url: &str) -> Option<String> {
    PLAYLIST_URL
        .captures(url)
        .map(|found| found["id"].to_owned())
}

/// `_get_cookies(url)['ss-token-guest'].value`, when non-empty.
fn guest_cookie(url: &Url) -> Option<String> {
    crate::cookies::get_cookie(url.as_str(), "ss-token-guest").filter(|token| !token.is_empty())
}

/// `_download_webpage_handle(url, display_id)` with the response's
/// `x-api-token` header (the handle's headers are not otherwise exposed).
fn download_page(client: &Client, url: &Url, id: &str) -> Result<(String, Option<String>)> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading webpage"));
    let mut response = super::direct::request_with_headers(client, url, None, &[]).map_err(
        |error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: NAME,
                id: id.to_owned(),
                what: "webpage",
                source: Box::new(error),
            },
            error => error,
        },
    )?;
    let api_token = response
        .headers()
        .get("x-api-token")
        .and_then(|value| value.to_str().ok())
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let webpage = super::decode_webpage(&body, response.headers());
    Ok((webpage, api_token))
}

/// `_get_headers(url, display_id)`: the `Authorization` header value, or
/// `None` after an `Unable to extract auth token` warning.
fn authorization(client: &Client, url: &Url, display_id: &str) -> Result<Option<String>> {
    let mut token = guest_cookie(url);
    if token.is_none() {
        let (webpage, api_token) = download_page(client, url, display_id)?;
        token = guest_cookie(url)
            .or(api_token)
            .or_else(|| {
                // `_search_json(..., default={}).get('apiToken')`.
                search_json(&SHOEBOX_TOKENS, &webpage, false)
                    .flatten()
                    .as_ref()
                    .and_then(|tokens| tokens.get("apiToken"))
                    .and_then(Value::as_str)
                    .filter(|token| !token.is_empty())
                    .map(str::to_owned)
            })
            .or_else(|| {
                GUEST_TOKEN
                    .captures(&webpage)
                    .map(|found| found[1].to_owned())
            });
    }
    match token {
        Some(token) => Ok(Some(format!("Bearer {token}"))),
        None => {
            crate::screen::warning(&format!("[{NAME}] Unable to extract auth token"));
            Ok(None)
        }
    }
}

/// `_download_json(url, id, headers=...)`.
fn get_json(
    client: &Client,
    url: &Url,
    id: &str,
    authorization: Option<&str>,
    note: &str,
) -> Result<Value> {
    let headers: Vec<(&str, &str)> = authorization
        .map(|value| vec![("Authorization", value)])
        .unwrap_or_default();
    download_json_note_headers(client, url, NAME, id, note, &headers)
}

/// `_extract_video(data, video_id)`: the formats (an HLS master, then the first
/// MP4 output with a usable URL) and the metadata fields in upstream's order.
fn video_info(
    client: &Client,
    data: Option<&Value>,
    id: &str,
    page: &Url,
    entry: bool,
) -> Result<VideoInfo> {
    let mut formats: Vec<VideoFormat> = Vec::new();
    let video_data = data
        .and_then(|data| data.pointer("/_embedded/video/_embedded"))
        .filter(|value| value.is_object());
    let hls = video_data
        .and_then(|video| video.pointer("/playlists/0/_links/related/href"))
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none);
    if let Some(manifest) = hls.and_then(|url| Url::parse(&url).ok())
        && let Some(stream) =
            extract_m3u8_master_optional(client, &manifest, NAME, id, Some("hls"), false, false)
    {
        formats.extend(stream.formats);
    }
    let mp4 = video_data
        .and_then(|video| video.get("video-outputs"))
        .and_then(Value::as_array)
        .and_then(|outputs| {
            outputs.iter().find(|output| {
                output
                    .pointer("/_links/related/href")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .is_some()
            })
        });
    if let Some(output) = mp4
        && let Some(href) = output
            .pointer("/_links/related/href")
            .and_then(Value::as_str)
    {
        formats.push(dict_format(
            vec![
                ("url", href.into()),
                ("format_id", "direct".into()),
                ("quality", 1.into()),
                (
                    "height",
                    super::streamable::int_or_none(output.get("height")),
                ),
                ("width", super::streamable::int_or_none(output.get("width"))),
                (
                    "filesize",
                    super::streamable::int_or_none(output.get("file_size")),
                ),
            ],
            NAME,
            id,
        )?);
    }

    let mut fields: Vec<(&str, Value)> = vec![("id", id.into()), ("formats", Value::Null)];
    let text = |key: &str| -> Option<Value> {
        data.and_then(|data| data.get(key))
            .filter(|value| value.is_string())
            .cloned()
    };
    let media = |path: &str| -> Option<Value> { data.and_then(|data| data.pointer(path)).cloned() };
    if let Some(title) = text("title") {
        fields.push(("title", title));
    }
    if let Some(description) = text("summary_text") {
        fields.push(("description", description));
    }
    if let Some(thumbnail) = data
        .and_then(|data| data.pointer("/_embedded/images/0/_links/related/href"))
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
    {
        fields.push(("thumbnail", thumbnail.into()));
    }
    // `int_or_none(scale=1000)`.
    if let Some(duration) =
        super::streamable::int_or_none(media("/_embedded/video/duration").as_ref()).as_i64()
    {
        fields.push(("duration", duration.div_euclid(1000).into()));
    }
    for (key, name) in [
        ("date", "timestamp"),
        ("published_at", "release_timestamp"),
        ("updated_at", "modified_timestamp"),
    ] {
        if let Some(timestamp) = text(key)
            .as_ref()
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.push((name, timestamp.into()));
        }
    }
    if entry {
        // The series' entries add these after the item's fields.
        fields.push(("webpage_url", page.as_str().into()));
        fields.push(("extractor_key", NAME.into()));
        fields.push(("extractor", NAME.into()));
    }
    Ok(dict_info(page, (NAME, NAME), fields, formats))
}

impl Extractor for Subsplash {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(video_id) = item_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let auth = authorization(client, url, &video_id)?;
        let api = update_url_query(
            &Url::parse(&format!("{API}/media-items"))?,
            &[("filter[short_code]", &video_id), ("include", ITEM_INCLUDE)],
        );
        let data = get_json(
            client,
            &api,
            &video_id,
            auth.as_deref(),
            "Downloading JSON metadata",
        )?;
        let item = data.pointer("/_embedded/media-items/0");
        Ok(Some(video_info(client, item, &video_id, url, false)?))
    }
}

impl Extractor for SubsplashPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(display_id) = series_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(PLAYLIST_NAME);
        crate::screen::screen(&format!(
            "[{PLAYLIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let auth = authorization(client, url, &display_id)?;
        let auth = auth.as_deref();
        let series_api = update_url_query(
            &Url::parse(&format!("{API}/media-series"))?,
            &[("filter[short_code]", &display_id)],
        );
        let data = get_json(
            client,
            &series_api,
            &display_id,
            auth,
            "Downloading JSON metadata",
        )?;
        let series = data
            .pointer("/_embedded/media-series/0")
            .filter(|value| value.is_object())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Subsplash series without metadata are not supported natively".into(),
                )
            })?;
        let series_uuid = series
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Subsplash series without an id are not supported natively".into(),
                )
            })?
            .to_owned();
        let title = series
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let count = series
            .get("media_items_count")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Subsplash series without an item count are not supported natively".into(),
                )
            })?;
        // `math.ceil(count / PAGE_SIZE)`.
        let total_pages = (count.max(0) as usize).div_ceil(PAGE_SIZE);

        let mut entries = Vec::new();
        for page in 0..total_pages {
            let number = (page + 1).to_string();
            let size = PAGE_SIZE.to_string();
            let items_api = update_url_query(
                &Url::parse(&format!("{API}/media-items"))?,
                &[
                    (
                        "filter[broadcast.status|broadcast.status]",
                        "null|on-demand",
                    ),
                    ("filter[media_series]", &series_uuid),
                    ("filter[status]", "published"),
                    ("include", SERIES_INCLUDE),
                    ("page[number]", &number),
                    ("page[size]", &size),
                    ("sort", "-position"),
                ],
            );
            let data = get_json(
                client,
                &items_api,
                &series_uuid,
                auth,
                &format!("Downloading page {}", page + 1),
            )?;
            let items = data
                .pointer("/_embedded/media-items")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            for item in items {
                let Some(entry_id) = item
                    .get("short_code")
                    .and_then(Value::as_str)
                    .filter(|code| !code.is_empty())
                else {
                    continue;
                };
                let page_url = Url::parse(&format!("https://subspla.sh/{entry_id}"))?;
                entries.push(video_info(client, Some(item), entry_id, &page_url, true)?);
            }
        }

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id,
            title,
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    item_id(url).is_some()
}

/// URL matching for `--list-extractors`.
pub fn playlist_suitable(url: &str) -> bool {
    series_id(url).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(item_id(
            "https://subsplash.com/u/skywatchtv/media/d/5whnx5s-the-grand-delusion-taking-place-right-now"
        ).is_some());
        assert_eq!(
            item_id("https://subsplash.com/laiglesiadelcentro/vid/mi/+ecb6a6b?autoplay=true")
                .as_deref(),
            Some("ecb6a6b")
        );
        assert_eq!(
            item_id("https://prophecywatchers.subspla.sh/8gps8cx").as_deref(),
            Some("8gps8cx")
        );
        assert!(item_id("https://subsplash.com/skywatchtv/our-videos/ms/+dbyjzp8").is_none());
        assert_eq!(
            series_id("https://subsplash.com/skywatchtv/our-videos/ms/+dbyjzp8").as_deref(),
            Some("dbyjzp8")
        );
        assert!(series_id("https://subsplash.com/").is_none());
    }
}
