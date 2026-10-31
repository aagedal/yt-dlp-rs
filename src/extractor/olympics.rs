//! Upstream `OlympicsReplayIE` (`yt_dlp/extractor/olympics.py`): olympics.com
//! videos from the Next.js `videoPlaylist` current video (geo restriction,
//! the `metering.olympics.com` tokenized HLS master playlist and live
//! status), else the legacy page's meta tags, `tokenGenerator` URL and
//! JSON-LD.
use super::{
    Extractor, dict_info, download_json_note, download_webpage, extract_m3u8_fatal_master,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct OlympicsReplay;

const NAME: &str = "OlympicsReplay";
const GEO_MESSAGE: &str = "This video is not available from your location due to geo restriction";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?olympics\.com/[a-z]{2}/(?:paris-2024/)?(?:replay|videos?|original-series/episode)/(?P<id>[\w-]+)",
    )
    .unwrap()
});
static IMAGES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"["']image["']:\s*["']([^"']+)["']"#).unwrap());
static THUMBNAIL_SIZE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/images/image/private/t_(?P<width_a>\d+)-(?P<height_a>\d+)_(?P<width>\d+)/primary/[\W\w\d]+")
        .unwrap()
});
static HTTP_REASON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r": Failed to download m3u8 information: HTTP Error \d+: (.*)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Olympics {what} are not supported natively"))
}

/// `traverse_obj(next_data, ('props', 'pageProps', 'page', 'items', lambda _,
/// v: v['name'] == 'videoPlaylist', 'data', 'currentVideo', {dict}, any))`.
fn current_video(next_data: &Value) -> Option<&Value> {
    let items: Vec<&Value> = match next_data.pointer("/props/pageProps/page/items")? {
        Value::Array(items) => items.iter().collect(),
        Value::Object(items) => items.values().collect(),
        _ => return None,
    };
    items
        .into_iter()
        .filter(|item| item.get("name").and_then(Value::as_str) == Some("videoPlaylist"))
        .filter_map(|item| item.pointer("/data/currentVideo"))
        .find(|video| video.is_object())
}

/// `_tokenize_url`: the metering service's `data.url`, queried with the
/// stream URL's own parameters (`parse_qs`), the URL without its query, the
/// service and the JWT.
fn tokenize_url(
    client: &Client,
    url: &str,
    token: &str,
    is_live: bool,
    video_id: &str,
) -> Result<String> {
    let (without_fragment, fragment) = match url.split_once('#') {
        Some((url, fragment)) => (url, Some(fragment)),
        None => (url, None),
    };
    let (base, query) = without_fragment
        .split_once('?')
        .unwrap_or((without_fragment, ""));
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if value.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, values)) => values.push(value.into_owned()),
            None => groups.push((key.into_owned(), vec![value.into_owned()])),
        }
    }
    let stripped = match fragment {
        Some(fragment) => format!("{base}#{fragment}"),
        None => base.to_owned(),
    };
    for (key, value) in [
        ("url", stripped.as_str()),
        ("service-id", if is_live { "live" } else { "vod" }),
        ("user-auth", token),
    ] {
        match groups.iter_mut().find(|(existing, _)| existing == key) {
            Some((_, values)) => *values = vec![value.to_owned()],
            None => groups.push((key.to_owned(), vec![value.to_owned()])),
        }
    }
    let mut api = Url::parse("https://metering.olympics.com/tokengenerator")?;
    {
        let mut pairs = api.query_pairs_mut();
        for (key, values) in &groups {
            for value in values {
                pairs.append_pair(key, value);
            }
        }
    }
    let response = download_json_note(
        client,
        &api,
        NAME,
        video_id,
        "Downloading tokenized m3u8 url",
    )?;
    let data = response
        .get("data")
        .ok_or_else(|| key_error(video_id, "data"))?;
    match data.get("url") {
        Some(Value::String(url)) => Ok(url.clone()),
        Some(_) => Err(unsupported("tokenized URLs that are not strings")),
        None => Err(key_error(video_id, "url")),
    }
}

/// `'georestricted' in e.cause.msg` for the fatal HLS request's HTTP error.
fn georestricted_http(error: &Error) -> bool {
    let Error::Reported(text) = error else {
        return false;
    };
    HTTP_REASON
        .captures(text)
        .is_some_and(|found| found[1].contains("georestricted"))
}

/// `_extract_from_nextjs_data`.
fn from_nextjs_data(
    client: &Client,
    page: &Url,
    webpage: &str,
    video_id: &str,
) -> Result<Option<VideoInfo>> {
    let next_data =
        super::search_nextjs_data(webpage, NAME, video_id).unwrap_or_else(|_| json!({}));
    let Some(data) = current_video(&next_data).filter(|data| super::nuxt::truthy(data)) else {
        return Ok(None);
    };
    let geo_countries: Vec<&str> = data
        .get("countries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if data.get("geoRestrictedVideo") == Some(&Value::Bool(true)) {
        return Err(crate::geo::restricted(
            &format!("[{NAME}] {video_id}: {GEO_MESSAGE}"),
            &geo_countries,
        ));
    }
    let is_live = data.get("streamingStatus").and_then(Value::as_str) == Some("LIVE");
    let m3u8_url = match data
        .get("videoUrl")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
    {
        Some(url) => url,
        None => match data.get("streamUrl") {
            Some(Value::String(url)) => url.clone(),
            Some(_) => return Err(unsupported("stream URLs that are not strings")),
            None => return Err(key_error(video_id, "streamUrl")),
        },
    };
    let token = match data.get("jwtToken") {
        Some(Value::String(token)) => token.clone(),
        Some(_) => return Err(unsupported("JWT tokens that are not strings")),
        None => return Err(key_error(video_id, "jwtToken")),
    };
    let tokenized = tokenize_url(client, &m3u8_url, &token, is_live, video_id)?;
    let manifest =
        Url::parse(&tokenized).map_err(|_| unsupported("tokenized URLs that are not URLs"))?;
    let stream =
        match extract_m3u8_fatal_master(client, &manifest, NAME, video_id, Some("hls"), true) {
            Ok(stream) => stream,
            Err(error) if georestricted_http(&error) => {
                return Err(crate::geo::restricted(
                    &format!("[{NAME}] {video_id}: {GEO_MESSAGE}"),
                    &geo_countries,
                ));
            }
            Err(error) => return Err(error),
        };
    let mut fields = vec![
        ("formats", Value::Null),
        ("subtitles", Value::Null),
        ("is_live", Value::Bool(is_live)),
    ];
    for (key, source) in [("id", "videoID"), ("title", "title")] {
        if let Some(Value::String(value)) = data.get(source) {
            fields.push((key, value.clone().into()));
        }
    }
    if !fields.iter().any(|(key, _)| *key == "id") {
        return Err(unsupported("videos without a string videoID"));
    }
    if let Some(timestamp) = data
        .get("contentDate")
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
    {
        fields.push(("timestamp", timestamp.into()));
    }
    let mut info = dict_info(page, (NAME, NAME), fields, stream.formats);
    info.subtitles = stream.subtitles;
    Ok(Some(info))
}

/// `_html_search_meta(NAMES, webpage)` (non-fatal): the first name with a
/// matching tag, else `unable to extract NAMES[0]`.
fn search_meta(webpage: &str, names: &[&str]) -> Option<String> {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name));
    if found.is_none() {
        super::search_warning(NAME, names[0]);
    }
    found
}

/// The legacy thumbnails: `"image": "A, B"` URLs with the width and the
/// aspect-derived height of their `t_W-H_WIDTH` transformation.
fn legacy_thumbnails(webpage: &str) -> Vec<Value> {
    let list = IMAGES
        .captures(webpage)
        .map(|found| crate::html::clean_html(&found[1]))
        .unwrap_or_default();
    list.replace(['[', ']'], "")
        .split(',')
        .map(|thumbnail| {
            let thumbnail = thumbnail.trim();
            let (width_a, height_a, width) = match THUMBNAIL_SIZE.captures(thumbnail) {
                Some(found) => (
                    found["width_a"].parse::<i64>().ok(),
                    found["height_a"].parse::<i64>().ok(),
                    found["width"].parse::<i64>().ok(),
                ),
                None => (None, None, None),
            };
            let height = match (width, height_a, width_a) {
                (Some(width), Some(height_a), Some(width_a)) if width_a != 0 => {
                    Some(((width * height_a) as f64 / width_a as f64).trunc() as i64)
                }
                _ => None,
            };
            json!({"url": thumbnail, "width": width, "height": height})
        })
        .collect()
}

impl Extractor for OlympicsReplay {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_GEO_BYPASS = False`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&[])
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
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        if let Some(info) = from_nextjs_data(client, url, &webpage, &video_id)? {
            return Ok(Some(info));
        }

        let title = search_meta(&webpage, &["title", "og:title", "twitter:title"]);
        let video_uuid = search_meta(&webpage, &["episode_uid"])
            .ok_or_else(|| unsupported("legacy pages without an episode_uid"))?;
        let m3u8_url = search_meta(&webpage, &["video_url"])
            .ok_or_else(|| unsupported("legacy pages without a video_url"))?;
        let json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract JSON-LD; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        // `json_ld.get('image')`: `_json_ld` results have no `image`.
        let thumbnails = legacy_thumbnails(&webpage);

        let mut api = Url::parse("https://olympics.com/tokenGenerator")?;
        api.query_pairs_mut().append_pair("url", &m3u8_url);
        let tokenized = download_json_note(
            client,
            &api,
            NAME,
            &video_uuid,
            "Downloading legacy tokenized m3u8 url",
        )?;
        let manifest = tokenized
            .as_str()
            .and_then(|url| Url::parse(url).ok())
            .ok_or_else(|| unsupported("legacy tokenized URLs that are not URLs"))?;
        let stream =
            extract_m3u8_fatal_master(client, &manifest, NAME, &video_uuid, Some("hls"), true)?;

        // `{'id': ..., 'title': ..., ..., **json_ld}`: JSON-LD keys replace
        // the page's in place, new ones follow.
        let mut fields: Vec<(String, Value)> = vec![
            ("id".into(), video_uuid.into()),
            ("title".into(), title.map_or(Value::Null, Value::from)),
            ("thumbnails".into(), Value::Array(thumbnails)),
            ("formats".into(), Value::Null),
            ("subtitles".into(), Value::Null),
        ];
        for (key, value) in json_ld {
            if matches!(key.as_str(), "formats" | "subtitles") {
                return Err(unsupported("JSON-LD media formats"));
            }
            match fields.iter_mut().find(|(existing, _)| *existing == key) {
                Some((_, existing)) => *existing = value,
                None => fields.push((key, value)),
            }
        }
        let fields = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        let mut info = dict_info(url, (NAME, NAME), fields, stream.formats);
        info.subtitles = stream.subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=url,width,height");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://olympics.com/fr/video/men-s-109kg-group-a-weightlifting-tokyo-2020-replays"
        ));
        assert!(suitable(
            "https://olympics.com/en/original-series/episode/b-boys-and-b-girls-take-the-spotlight"
        ));
        assert!(suitable(
            "https://www.olympics.com/en/paris-2024/videos/men-s-preliminaries-hockey"
        ));
        assert!(!suitable("https://olympics.com/en/news/some-story"));
    }

    #[test]
    fn legacy_thumbnails_derive_heights_from_the_transformation() {
        let page = r#"{"image": "[https://img.olympics.com/images/image/private/t_16-9_640/primary/a, https://x.test/b.jpg]"}"#;
        assert_eq!(
            legacy_thumbnails(page),
            [
                json!({"url": "https://img.olympics.com/images/image/private/t_16-9_640/primary/a", "width": 640, "height": 360}),
                json!({"url": "https://x.test/b.jpg", "width": null, "height": null}),
            ]
        );
        assert_eq!(
            legacy_thumbnails("<html></html>"),
            [json!({"url": "", "width": null, "height": null})]
        );
    }

    #[test]
    fn only_georestricted_http_reasons_are_geo_errors() {
        let error = |reason: &str| {
            Error::Reported(format!(
                "[{NAME}] x: Failed to download m3u8 information: HTTP Error 403: {reason}"
            ))
        };
        assert!(georestricted_http(&error("georestricted")));
        assert!(!georestricted_http(&error("Forbidden")));
    }
}
