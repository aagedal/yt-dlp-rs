//! Upstream `DailymotionIE` (`yt_dlp/extractor/dailymotion.py`): a client
//! token, the GraphQL media object, the player metadata's qualities (HLS
//! with randomized headers, progressive H264), subtitles and metadata.
//!
//! `DailymotionPlaylistIE`, `DailymotionUserIE` and `DailymotionSearchIE`
//! (`DailymotionPlaylistBaseIE`): GraphQL `collection`/`channel` video pages
//! and `SEARCH_QUERY` pages as `OnDemandPagedList`s of Dailymotion `url`
//! results, whose pages are pending entries (see [`super::paged`]).
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Dailymotion;

const NAME: &str = "dailymotion";
const MACHINE: &str = "dailymotion";
const KEY: &str = "Dailymotion";
const COMMON_MEDIA_FIELDS: &str =
    "description\n      geoblockedCountries {\n        allowed\n      }\n      xid";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:https?:)?//(?:dai\.ly/|(?:(?:(?:www|touch|geo)\.)?dailymotion\.[a-z]{2,3}|(?:www\.)?lequipe\.fr)/(?:swf/|(?:(?:crawler|embed|swf)/)?video/|player(?:/[\da-z]+)?\.html\?(?:video|(?P<is_playlist>playlist))=))(?P<id>[^/?_&#]+)(?:[\w-]*\?playlist=(?P<playlist_id>x[0-9a-z]+))?",
    )
    .unwrap()
});
static H264: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/H264-(\d+)x(\d+)(?:-(60)/)?").unwrap());

/// `_match_valid_url` (`swf/(?!video)` checked apart).
fn match_url(url: &str) -> Option<(String, bool, Option<String>)> {
    let found = VALID_URL.captures(url)?;
    let id = found["id"].to_owned();
    if url.contains("/swf/") && !url.contains("/swf/video/") && id.eq_ignore_ascii_case("video") {
        return None;
    }
    Some((
        id,
        found.name("is_playlist").is_some(),
        found.name("playlist_id").map(|id| id.as_str().to_owned()),
    ))
}

/// `DailymotionIE.suitable`.
pub fn suitable(url: &str) -> bool {
    match_url(url).is_some()
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// `_generate_blockbuster_headers()`: 2–8 random consonant headers.
fn blockbuster_headers() -> Vec<(String, String)> {
    use std::hash::{BuildHasher, Hasher};
    const LETTERS: &[u8] = b"bcdfghjklmnpqrstvwxz";
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos()),
    );
    let mut state = hasher.finish() | 1;
    let mut next = |bound: u64| {
        // xorshift64.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let mut word = |minimum: u64, maximum: u64| -> String {
        let length = minimum + next(maximum - minimum + 1);
        (0..length)
            .map(|_| LETTERS[next(LETTERS.len() as u64) as usize] as char)
            .collect()
    };
    let count = 2 + (std::process::id() as u64 + 3) % 7;
    (0..count).map(|_| (word(8, 24), word(16, 32))).collect()
}

fn post_json(
    client: &Client,
    ie: &'static str,
    url: &Url,
    id: &str,
    note: &str,
    headers: &[(&str, &str)],
    body: String,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&if id.is_empty() {
        format!("[{ie}] {note}")
    } else {
        format!("[{ie}] {id}: {note}")
    });
    let mut response =
        super::direct::post(client, url, headers, body).map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: ie,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            },
            error => error,
        })?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    serde_json::from_slice(&body).map_err(|error| {
        Error::Reported(format!(
            "[{ie}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_get_token(xid)`: a cookie's token, else a token for the account
/// (`_get_login_info`: a `password` grant) or the client (a
/// `client_credentials` grant). A 400 answer is its `error_description`.
fn token(client: &Client, ie: &'static str, xid: &str) -> Result<String> {
    for name in ["access_token", "client_token"] {
        if let Some(token) = crate::cookies::get_cookie("https://www.dailymotion.com/", name) {
            return Ok(token);
        }
    }
    let url = Url::parse("https://graphql.api.dailymotion.com/oauth/token")?;
    let mut form = vec![
        ("client_id", "f1a362d288c1b98099c7"),
        ("client_secret", "eea605b96e01c796ff369935357eca920c5da4c5"),
    ];
    let login = super::login::get_login_info(ie, Some(MACHINE))?
        .filter(|(username, _)| !username.is_empty());
    match &login {
        Some((username, password)) => form.extend([
            ("grant_type", "password"),
            ("password", password.as_str()),
            ("username", username.as_str()),
        ]),
        None => form.push(("grant_type", "client_credentials")),
    }
    let session = super::login::Session {
        ie,
        id: xid.to_owned(),
        machine: Some(MACHINE),
    };
    let page = session.request(
        client,
        &url,
        (
            Some("Downloading Access Token"),
            "Unable to download JSON metadata",
        ),
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some(super::login::urlencode(&form)),
        &[400],
    )?;
    let response: Value = crate::json::from_str(&page.text).map_err(|error| {
        Error::Reported(format!(
            "[{ie}] {xid}: Failed to parse JSON  (caused by {error})"
        ))
    })?;
    let key_error = |key: &str| {
        Error::Reported(format!(
            "[{ie}] {xid}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    };
    if page.status == 400 {
        let description = response
            .get("error_description")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error("error_description"))?;
        return Err(session.error(description));
    }
    let token = response
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error("access_token"))?
        .to_owned();
    let cookie = if login.is_some() {
        "access_token"
    } else {
        "client_token"
    };
    crate::cookies::set_cookie("www.dailymotion.com", cookie, &token);
    Ok(token)
}

/// `_call_api('media', xid, fields, note)`.
fn media(client: &Client, xid: &str) -> Result<Value> {
    let token = token(client, NAME, xid)?;
    let fields = format!(
        "... on Video {{\n      {COMMON_MEDIA_FIELDS}\n      stats {{\n        likes {{\n          total\n        }}\n        views {{\n          total\n        }}\n      }}\n    }}\n    ... on Live {{\n      {COMMON_MEDIA_FIELDS}\n      audienceCount\n      isOnAir\n    }}"
    );
    let query = format!("{{\n  media(xid: \"{xid}\") {{\n    {fields}\n  }}\n}}");
    let body = crate::output::python_json_dumps(&json!({ "query": query }));
    let authorization = format!("Bearer {token}");
    let response = post_json(
        client,
        NAME,
        &Url::parse("https://graphql.api.dailymotion.com/")?,
        xid,
        "Downloading media JSON metadata",
        &[
            ("Content-Type", "application/json"),
            ("Origin", "https://www.dailymotion.com"),
            ("Authorization", &authorization),
        ],
        body,
    )?;
    match response.pointer("/data/media") {
        Some(media) if !media.is_null() => Ok(media.clone()),
        _ => Err(fail(
            xid,
            response
                .pointer("/errors/0/message")
                .and_then(Value::as_str)
                .unwrap_or("Unknown error"),
        )),
    }
}

/// `_extract_dailymotion_m3u8_formats_and_subtitles`: a request with
/// randomized headers, then retries requiring Chrome and Firefox
/// impersonation; the last error is returned when it is about
/// impersonation, and warned about otherwise.
fn m3u8_formats(
    client: &Client,
    url: &str,
    id: &str,
) -> (Vec<crate::VideoFormat>, crate::Subtitles, Option<String>) {
    let Ok(manifest) = Url::parse(url) else {
        return (Vec::new(), Default::default(), None);
    };
    let headers = blockbuster_headers();
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let failed = |error: Error| {
        format!(
            "Unable to download m3u8 information: {}",
            crate::upstream_error_text(&error)
        )
    };
    let mut last_error = String::new();
    for (note, target) in [
        ("Downloading m3u8 information with randomized headers", None),
        (
            "Retrying m3u8 download with Chrome impersonation",
            Some("chrome"),
        ),
        (
            "Retrying m3u8 download with Firefox impersonation",
            Some("firefox"),
        ),
    ] {
        crate::screen::screen(&format!("[{NAME}] {id}: {note}"));
        let response = match target {
            None => super::direct::request_with_headers(client, &manifest, None, &headers)
                .map_err(failed),
            Some(target) => crate::impersonate::require(&[target]).and_then(|_impersonate| {
                super::direct::request(client, &manifest, None).map_err(failed)
            }),
        };
        match response {
            Ok(response) => {
                return match super::hls::from_response(
                    client,
                    response,
                    &manifest,
                    None,
                    true,
                    Some("hls"),
                ) {
                    Ok(Some(stream)) => (stream.formats, stream.subtitles, None),
                    _ => (Vec::new(), Default::default(), None),
                };
            }
            Err(message) => {
                crate::screen::debug(&format!("{id}: {message}"));
                last_error = message;
            }
        }
    }
    if !last_error.contains("impersonation") {
        crate::screen::warning(&format!("[{NAME}] {id}: {last_error}"));
        return (Vec::new(), Default::default(), None);
    }
    (Vec::new(), Default::default(), Some(last_error))
}

impl Extractor for Dailymotion {
    fn key(&self) -> &'static str {
        KEY
    }

    /// `_GEO_BYPASS = False`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&[])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    /// `DailymotionBaseInfoExtractor` logs in while requesting its token.
    fn netrc_machine(&self) -> Option<&'static str> {
        Some(MACHINE)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some((mut video_id, is_playlist, mut playlist_id)) = match_url(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        self.initialize(client, NAME, &video_id)?;
        family_filter();
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        if is_playlist {
            playlist_id = Some(video_id.clone());
            video_id = String::new();
        }
        // `_yes_playlist(playlist_id, video_id)`.
        if let Some(playlist_id) = &playlist_id {
            let yes = if video_id.is_empty() {
                true
            } else if super::no_playlist() {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading just the video {video_id} because of --no-playlist"
                ));
                false
            } else {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading playlist {playlist_id} - add --no-playlist to download just the video {video_id}"
                ));
                true
            };
            if yes {
                // `url_result(f'http://www.dailymotion.com/playlist/{id}',
                // 'DailymotionPlaylist', id)`.
                let target = format!("http://www.dailymotion.com/playlist/{playlist_id}");
                let mut follow = super::direct::build_at(url.clone(), url, String::new());
                follow.formats.clear();
                follow.direct = None;
                follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
                return Ok(Some(ExtractResult::Video(follow)));
            }
        }
        let media = media(client, &video_id)?;
        let xid = media
            .get("xid")
            .and_then(Value::as_str)
            .unwrap_or(&video_id)
            .to_owned();
        let mut metadata_url = Url::parse(&format!(
            "https://www.dailymotion.com/player/metadata/video/{xid}"
        ))?;
        metadata_url
            .query_pairs_mut()
            .append_pair("app", "com.dailymotion.neon");
        let metadata = super::download_json_note(
            client,
            &metadata_url,
            NAME,
            &xid,
            "Downloading metadata JSON",
        )?;
        if let Some(error) = metadata.get("error").filter(|error| !error.is_null()) {
            let title = error
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| !title.is_empty())
                .or_else(|| error.get("raw_message").and_then(Value::as_str))
                .unwrap_or_default();
            if error.get("code").and_then(Value::as_str) == Some("DM007") {
                // `raise_geo_restricted(msg=title, countries=allowed)`.
                let allowed: Vec<&str> = media
                    .pointer("/geoblockedCountries/allowed")
                    .and_then(Value::as_array)
                    .map(|allowed| allowed.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                return Err(crate::geo::restricted(
                    &fail(&xid, title).to_string(),
                    &allowed,
                ));
            }
            return Err(fail(&xid, &format!("{NAME} said: {title}")));
        }
        let title = metadata
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let is_live = media.get("isOnAir").cloned().unwrap_or(Value::Null);
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let mut expected_error = None;
        for (quality, media_list) in metadata
            .get("qualities")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            for item in media_list.as_array().into_iter().flatten() {
                let Some(media_url) = item
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty())
                else {
                    continue;
                };
                let media_type = item.get("type").and_then(Value::as_str);
                if media_type == Some("application/vnd.lumberjack.manifest") {
                    continue;
                }
                if media_type == Some("application/x-mpegURL") {
                    let (found, subs, error) = m3u8_formats(client, media_url, &video_id);
                    formats.extend(found);
                    super::html5::merge_subtitles(&mut subtitles, subs);
                    expected_error = error;
                } else {
                    let mut format = Map::new();
                    format.insert(
                        "url".into(),
                        media_url.split('#').next().unwrap_or(media_url).into(),
                    );
                    format.insert("format_id".into(), format!("http-{quality}").into());
                    let mut keys = vec!["url", "format_id"];
                    if let Some(found) = H264.captures(media_url) {
                        let fps = found
                            .get(3)
                            .and_then(|fps| fps.as_str().parse::<i64>().ok());
                        format.insert("fps".into(), fps.into());
                        format.insert("height".into(), found[2].parse::<i64>().ok().into());
                        format.insert("width".into(), found[1].parse::<i64>().ok().into());
                        keys.extend(["fps", "height", "width"]);
                    }
                    if format.get("fps").is_none_or(Value::is_null) && quality.ends_with("@60") {
                        format.insert("fps".into(), 60.into());
                        if !keys.contains(&"fps") {
                            keys.push("fps");
                        }
                    }
                    if format.get("fps").is_some_and(Value::is_null) {
                        format.shift_remove("fps");
                    }
                    // Processing's `determine_ext` sees the URL without its fragment.
                    format.insert(
                        "ext".into(),
                        super::direct::determine_ext(
                            media_url.split('#').next().unwrap_or(media_url),
                        )
                        .unwrap_or_else(|| "unknown_video".into())
                        .into(),
                    );
                    let mut format: crate::VideoFormat =
                        serde_json::from_value(Value::Object(format))
                            .map_err(|error| fail(&video_id, &error.to_string()))?;
                    crate::info_order::mark_format(
                        &mut format,
                        &format!("dict:{}", keys.join(",")),
                    );
                    formats.push(format);
                }
            }
        }
        if formats.is_empty()
            && let Some(error) = expected_error
        {
            crate::no_formats::raise_expected(NAME, None, Some(&video_id), &error)?;
        }
        for format in &mut formats {
            if let Some((url, _)) = format.url.as_str().split_once('#')
                && let Ok(url) = Url::parse(url)
            {
                format.url = url;
            }
            if format.fps.is_none() && format.format_id.ends_with("@60") {
                format.fps = Some(crate::info::PyNumber::Int(60));
                crate::info_order::mark_format_update(format, "fps");
            }
        }
        for (language, subtitle) in metadata
            .pointer("/subtitles/data")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let tracks: Vec<crate::SubtitleFormat> = subtitle
                .get("urls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|url| {
                    let ext =
                        super::direct::determine_ext(url).unwrap_or_else(|| "unknown_video".into());
                    serde_json::from_value(json!({"url": url, "ext": ext})).ok()
                })
                .collect();
            let language = Some(language.clone());
            match subtitles
                .entries
                .iter_mut()
                .find(|(key, _)| *key == language)
            {
                Some((_, existing)) => *existing = tracks,
                None => subtitles.entries.push((language, tracks)),
            }
        }
        let mut thumbnails = Vec::new();
        for key in ["posters", "thumbnails"] {
            for (size, url) in metadata
                .get(key)
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                if url.as_str().and_then(super::json_ld::url_or_none).is_none() {
                    continue;
                }
                thumbnails.push(json!({
                    "height": size.parse::<i64>().ok(),
                    "id": size,
                    "url": url,
                }));
            }
        }
        let owner = metadata.get("owner").cloned().unwrap_or_else(|| json!({}));
        let count = |kind: &str| int(media.pointer(&format!("/stats/{kind}s/total")));
        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("title".into(), title.clone().into());
        dict.insert(
            "description".into(),
            media
                .get("description")
                .and_then(Value::as_str)
                .map(crate::html::clean_html)
                .into(),
        );
        dict.insert("thumbnails".into(), Value::Array(thumbnails));
        dict.insert(
            "duration".into(),
            int(metadata.get("duration")).filter(|d| *d != 0).into(),
        );
        dict.insert("timestamp".into(), int(metadata.get("created_time")).into());
        dict.insert(
            "uploader".into(),
            owner.get("screenname").cloned().unwrap_or(Value::Null),
        );
        dict.insert(
            "uploader_id".into(),
            owner
                .get("id")
                .filter(|id| !id.is_null() && *id != "")
                .or_else(|| metadata.get("screenname"))
                .cloned()
                .unwrap_or(Value::Null),
        );
        dict.insert(
            "age_limit".into(),
            if metadata.get("explicit").and_then(Value::as_bool) == Some(true) {
                18
            } else {
                0
            }
            .into(),
        );
        dict.insert(
            "tags".into(),
            metadata.get("tags").cloned().unwrap_or(Value::Null),
        );
        dict.insert(
            "view_count".into(),
            count("view")
                .filter(|views| *views != 0)
                .or_else(|| int(media.get("audienceCount")))
                .into(),
        );
        dict.insert("like_count".into(), count("like").into());
        dict.insert("formats".into(), json!([]));
        dict.insert("subtitles".into(), json!({}));
        dict.insert("is_live".into(), is_live);
        Ok(Some(ExtractResult::Video(video(
            url, dict, formats, subtitles,
        ))))
    }
}

/// A video from upstream's dict (`dict;KEYS` order).
fn video(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
) -> VideoInfo {
    let first = formats
        .first()
        .map_or_else(|| url.clone(), |format| format.url.clone());
    let mut info = super::direct::build_at(first.clone(), &first, String::new());
    info.direct = None;
    if formats.is_empty() {
        info.extra
            .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
    }
    info.formats = formats;
    info.subtitles = subtitles;
    info.webpage_url = url.clone();
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
    let mut keys = Vec::new();
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "formats" | "subtitles" => {}
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    crate::info_order::mark_info(
        info,
        &format!(
            "dict;{};subtitle=url;thumbnail=height,id,url",
            keys.join(",")
        ),
    )
}

/// `_real_initialize`: the `ff` cookie, else `age_restricted(18,
/// age_limit)`, is the family filter, stored back in the `ff` cookie.
fn family_filter() -> bool {
    let family = match crate::cookies::get_cookie("https://www.dailymotion.com/", "ff") {
        Some(ff) if !ff.is_empty() => ff == "on",
        _ => super::age_restricted(18),
    };
    crate::cookies::set_cookie(
        "www.dailymotion.com",
        "ff",
        if family { "on" } else { "off" },
    );
    family
}

pub struct DailymotionPlaylist;
pub struct DailymotionUser;
pub struct DailymotionSearch;

const PLAYLIST_NAME: &str = "dailymotion:playlist";
const PLAYLIST_KEY: &str = "DailymotionPlaylist";
const USER_NAME: &str = "dailymotion:user";
const USER_KEY: &str = "DailymotionUser";
const SEARCH_NAME: &str = "dailymotion:search";
const SEARCH_KEY: &str = "DailymotionSearch";
/// `DailymotionPlaylistBaseIE._PAGE_SIZE`.
const PAGE_SIZE: usize = 100;
/// `DailymotionSearchIE._PAGE_SIZE`.
const SEARCH_PAGE_SIZE: usize = 20;
const SEARCH_QUERY: &str = "query SEARCH_QUERY( $query: String! $page: Int $limit: Int ) { search { videos( query: $query first: $limit page: $page ) { edges { node { xid } } } } } ";

static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:https?://)?(?:www\.)?dailymotion\.[a-z]{2,3}/playlist/(?P<id>x[0-9a-z]+)")
        .unwrap()
});
static SEARCH_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?dailymotion\.[a-z]{2,3}/search/(?P<id>[^/?#]+)/videos")
        .unwrap()
});
/// `DailymotionUserIE._VALID_URL` without its lookahead (checked apart).
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<site>https?://(?:www\.)?dailymotion\.[a-z]{2,3}/)(?:(?:old/)?user/)?(?P<id>[^/?#]+)",
    )
    .unwrap()
});

/// `DailymotionPlaylistIE.suitable`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `DailymotionSearchIE.suitable`.
pub fn search_suitable(url: &str) -> bool {
    SEARCH_URL.is_match(url)
}

/// `DailymotionUserIE._match_id`, with the
/// `(?!(?:embed|swf|#|video|playlist|search|crawler)/)` lookahead.
fn user_id(url: &str) -> Option<String> {
    let found = USER_URL.captures(url)?;
    let rest = &url[found["site"].len()..];
    if [
        "embed", "swf", "#", "video", "playlist", "search", "crawler",
    ]
    .iter()
    .any(|name| {
        rest.strip_prefix(name)
            .is_some_and(|rest| rest.starts_with('/'))
    }) {
        return None;
    }
    Some(found["id"].to_owned())
}

/// `DailymotionUserIE.suitable`.
pub fn user_suitable(url: &str) -> bool {
    user_id(url).is_some()
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Dailymotion {what} are not supported natively"))
}

/// An `ExtractorError` raised while the paged entries are read, outside
/// `extract()`: without the `[IE] ` prefix (upstream fills `ie` in only for
/// errors of `_real_extract`).
fn lazy_error(ie: &str, error: Error) -> Error {
    lazy_error_without(ie, "", error)
}

/// [`lazy_error`] for a request upstream makes without a video ID (`id`
/// being the one this port's request reports).
fn lazy_error_without(ie: &str, id: &str, error: Error) -> Error {
    let text = error.to_string();
    let text = text
        .strip_prefix(&format!("[{ie}] {id}: "))
        .or_else(|| text.strip_prefix(&format!("[{ie}] : ")))
        .or_else(|| text.strip_prefix(&format!("[{ie}] ")))
        .map(str::to_owned);
    match text {
        Some(text) => Error::Reported(text),
        None => error,
    }
}

/// `url_result(url, DailymotionIE.ie_key(), xid)`.
fn url_entry(target: &str, id: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = id.to_owned();
    let mut entry = Map::new();
    entry.insert("ie_key".into(), KEY.into());
    entry.insert("id".into(), id.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// The GraphQL endpoint's headers (`_HEADERS` with the token's
/// `Authorization`).
fn graphql(
    client: &Client,
    ie: &'static str,
    token_id: &str,
    id: &str,
    note: &str,
    body: String,
) -> Result<Value> {
    // `_get_token` requests its token without a video ID.
    let token =
        token(client, ie, token_id).map_err(|error| lazy_error_without(ie, token_id, error))?;
    let authorization = format!("Bearer {token}");
    post_json(
        client,
        ie,
        &Url::parse("https://graphql.api.dailymotion.com/")?,
        id,
        note,
        &[
            ("Content-Type", "application/json"),
            ("Origin", "https://www.dailymotion.com"),
            ("Authorization", &authorization),
        ],
        body,
    )
}

/// `DailymotionPlaylistBaseIE._fetch_page(playlist_id, page)`: the
/// `collection`/`channel` object's videos page (`_call_api`).
fn fetch_object_page(
    client: &Client,
    ie: &'static str,
    object_type: &str,
    xid: &str,
    family: bool,
    page: usize,
) -> Result<Vec<VideoInfo>> {
    let page = page + 1;
    let fields = format!(
        "videos(allowExplicit: {}, first: {PAGE_SIZE}, page: {page}) {{\n      edges {{\n        node {{\n          xid\n          url\n        }}\n      }}\n    }}",
        if family { "false" } else { "true" }
    );
    let query = format!("{{\n  {object_type}(xid: \"{xid}\") {{\n    {fields}\n  }}\n}}");
    let body = crate::output::python_json_dumps(&json!({ "query": query }));
    let response = graphql(
        client,
        ie,
        xid,
        xid,
        &format!("Downloading page {page}"),
        body,
    )?;
    let object = response
        .get("data")
        .and_then(|data| data.get(object_type))
        .ok_or_else(|| unsupported("API responses without data"))?;
    if !super::nuxt::truthy(object) {
        // `ExtractorError(resp['errors'][0]['message'], expected=True)`.
        return match response.pointer("/errors/0/message") {
            Some(Value::String(message)) => Err(Error::Reported(message.clone())),
            _ => Err(unsupported("API errors without a message")),
        };
    }
    let edges = object
        .pointer("/videos/edges")
        .and_then(Value::as_array)
        .ok_or_else(|| unsupported("video pages without edges"))?;
    let mut entries = Vec::with_capacity(edges.len());
    for edge in edges {
        let (Some(url), Some(xid)) = (
            edge.pointer("/node/url").and_then(Value::as_str),
            edge.pointer("/node/xid").and_then(Value::as_str),
        ) else {
            return Err(unsupported("video nodes without a url and xid"));
        };
        entries.push(url_entry(url, xid)?);
    }
    Ok(entries)
}

/// `DailymotionSearchIE._fetch_page(term, page)` (`_call_search_api`).
fn fetch_search_page(client: &Client, term: &str, page: usize) -> Result<Vec<VideoInfo>> {
    let page = page + 1;
    let body = crate::output::python_json_dumps(&json!({
        "operationName": "SEARCH_QUERY",
        "query": SEARCH_QUERY,
        "variables": {"limit": 20, "page": page, "query": term},
    }));
    let response = graphql(
        client,
        SEARCH_NAME,
        term,
        "",
        &format!("Searching \"{term}\" page {page}"),
        body,
    )?;
    let search = response
        .pointer("/data/search")
        .filter(|search| search.as_object().is_some_and(|search| !search.is_empty()));
    let Some(search) = search else {
        // `ExtractorError(...)` (not expected).
        let message = response
            .pointer("/errors/0/message")
            .and_then(Value::as_str)
            .unwrap_or("Could not fetch search data");
        return Err(Error::Reported(format!(
            "{message}; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    // `traverse_obj(response, ('videos', 'edges', ..., 'node', 'xid'))`.
    let edges: Vec<&Value> = match search.pointer("/videos/edges") {
        Some(Value::Array(edges)) => edges.iter().collect(),
        Some(Value::Object(edges)) => edges.values().collect(),
        _ => Vec::new(),
    };
    let mut entries = Vec::new();
    for xid in edges
        .into_iter()
        .filter_map(|edge| edge.pointer("/node/xid"))
    {
        let xid = match xid {
            Value::Null => continue,
            Value::String(xid) => xid.clone(),
            Value::Number(xid) => xid.to_string(),
            _ => return Err(unsupported("search results whose xid is not a string")),
        };
        entries.push(url_entry(
            &format!("https://www.dailymotion.com/video/{xid}"),
            &xid,
        )?);
    }
    Ok(entries)
}

/// Whether a playlist's pending state is a Dailymotion playlist's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == "dailymotion"
}

/// `OnDemandPagedList(fetch_page, PAGE_SIZE)` read by upstream's
/// `PlaylistEntries` (see [`super::paged`]).
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let object_type = state["object_type"].as_str().unwrap_or_default().to_owned();
    let id = state["id"].as_str().unwrap_or_default().to_owned();
    let family = state["family"].as_bool().unwrap_or_default();
    let (ie, page_size) = match object_type.as_str() {
        "collection" => (PLAYLIST_NAME, PAGE_SIZE),
        "channel" => (USER_NAME, PAGE_SIZE),
        _ => (SEARCH_NAME, SEARCH_PAGE_SIZE),
    };
    let mut fetch = |page| {
        if ie == SEARCH_NAME {
            fetch_search_page(client, &id, page)
        } else {
            fetch_object_page(client, ie, &object_type, &id, family, page)
        }
        .map_err(|error| lazy_error(ie, error))
    };
    let placeholder = || url_entry("https://www.dailymotion.com/video/None", "None");
    super::paged::fetch_requested(state, limit, entries, page_size, &mut fetch, &placeholder)
}

/// `playlist_result(OnDemandPagedList(...), id[, title])`.
fn paged_playlist(
    url: &Url,
    (ie, key): (&'static str, &'static str),
    object_type: &str,
    id: String,
    title: Option<String>,
    family: bool,
) -> ExtractResult {
    let mut extra = Map::new();
    // `OnDemandPagedList`, whose full count, like a generator's, is unknown
    // unless a selection reads past its end.
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!(
            "playlist-dict;id{}{}",
            if title.is_some() { ",title" } else { "" },
            crate::info_order::GENERATOR
        )
        .into(),
    );
    let state = json!({
        "kind": "dailymotion",
        "object_type": object_type,
        "id": id,
        "family": family,
        "random_access": true,
    });
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: ie.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: Some(crate::PendingEntries { state }),
        entries: Vec::new(),
        extra,
    })
}

/// `DailymotionPlaylistBaseIE._real_extract` for the playlist and user
/// extractors (and the search extractor's own).
fn extract_paged<E: Extractor + ?Sized>(
    extractor: &E,
    client: &Client,
    url: &Url,
    (ie, key): (&'static str, &'static str),
    id: Option<String>,
) -> Result<Option<ExtractResult>> {
    let Some(id) = id else {
        return Ok(None);
    };
    crate::screen::set_extractor(ie);
    extractor.initialize(client, ie, &id)?;
    let family = family_filter();
    crate::screen::screen(&format!(
        "[{ie}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    Ok(Some(match ie {
        SEARCH_NAME => {
            // `urllib.parse.unquote_plus(self._match_id(url))`.
            let term = percent_encoding::percent_decode_str(&id.replace('+', " "))
                .decode_utf8_lossy()
                .into_owned();
            paged_playlist(url, (ie, key), "search", term.clone(), Some(term), family)
        }
        PLAYLIST_NAME => paged_playlist(url, (ie, key), "collection", id, None, family),
        _ => paged_playlist(url, (ie, key), "channel", id, None, family),
    }))
}

macro_rules! paged_extractor {
    ($type:ty, $name:expr, $key:expr, $id:expr) => {
        impl Extractor for $type {
            fn key(&self) -> &'static str {
                $key
            }

            fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
                Ok(None)
            }

            fn netrc_machine(&self) -> Option<&'static str> {
                Some(MACHINE)
            }

            fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
                let id: fn(&str) -> Option<String> = $id;
                extract_paged(self, client, url, ($name, $key), id(url.as_str()))
            }
        }
    };
}

paged_extractor!(DailymotionPlaylist, PLAYLIST_NAME, PLAYLIST_KEY, |url| {
    PLAYLIST_URL
        .captures(url)
        .map(|found| found["id"].to_owned())
});
paged_extractor!(DailymotionSearch, SEARCH_NAME, SEARCH_KEY, |url| {
    SEARCH_URL.captures(url).map(|found| found["id"].to_owned())
});
paged_extractor!(DailymotionUser, USER_NAME, USER_KEY, user_id);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_urls_follow_upstream_valid_urls() {
        assert!(playlist_suitable(
            "http://www.dailymotion.com/playlist/xv4bw_nqtv_sport/1#video=xl8v3q"
        ));
        assert!(!playlist_suitable(
            "https://www.dailymotion.com/playlist/abc"
        ));
        assert!(search_suitable(
            "http://www.dailymotion.com/search/king%20of%20turtles/videos"
        ));
        assert!(!search_suitable("http://www.dailymotion.com/search/king"));
        assert_eq!(
            user_id("https://www.dailymotion.com/user/nqtv").as_deref(),
            Some("nqtv")
        );
        assert_eq!(
            user_id("https://www.dailymotion.com/old/user/nqtv").as_deref(),
            Some("nqtv")
        );
        assert_eq!(
            user_id("https://www.dailymotion.com/old/nqtv").as_deref(),
            Some("old")
        );
        assert_eq!(
            user_id("http://www.dailymotion.com/UnderProject").as_deref(),
            Some("UnderProject")
        );
        for url in [
            "https://www.dailymotion.com/video/x5kesuj",
            "https://www.dailymotion.com/embed/video/x5kesuj",
            "https://www.dailymotion.com/playlist/xv4bw",
            "https://www.dailymotion.com/search/x/videos",
            "https://www.dailymotion.com/crawler/video/x5kesuj",
            "https://www.dailymotion.com/",
        ] {
            assert!(!user_suitable(url), "{url}");
        }
        // Only the lookahead's words followed by `/` are excluded.
        assert!(user_suitable("https://www.dailymotion.com/videos"));
    }

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert_eq!(
            match_url("https://www.dailymotion.com/video/x5kesuj")
                .map(|(id, ..)| id)
                .as_deref(),
            Some("x5kesuj")
        );
        assert!(suitable("https://dai.ly/x5kesuj"));
        assert!(suitable("https://www.dailymotion.com/embed/video/x7tgad0"));
        assert!(suitable(
            "https://geo.dailymotion.com/player/xf7zn.html?video=x8u4owg"
        ));
        assert_eq!(
            match_url("https://www.dailymotion.com/video/x73czzz?playlist=x6hynp"),
            Some(("x73czzz".into(), false, Some("x6hynp".into())))
        );
        assert!(!suitable("https://www.dailymotion.com/user/someone"));
    }
}
