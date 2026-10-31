//! Upstream `PornHubIE`, `PornHubUserIE`, `PornHubPagedVideoListIE` and
//! `PornHubPlaylistIE` (`yt_dlp/extractor/pornhub.py`): a view page's
//! flashvars media (MP4 and HLS masters with the site's headers), a user's
//! `/videos` pages, a channel/model/pornstar video list and a playlist's
//! first page plus its `viewChunked` pages. Logins and the PhantomJS
//! challenge are not supported.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage_handle};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct PornHub;
pub struct PornHubUser;
pub struct PornHubPagedVideoList;
pub struct PornHubPlaylist;

const NAME: &str = "PornHub";
const USER_NAME: &str = "PornHubUser";
const PAGED_NAME: &str = "PornHubPagedVideoList";
const PLAYLIST_NAME: &str = "PornHubPlaylist";

const HOST: &str = r"(?:(?P<host>pornhub(?:premium)?\.(?:com|net|org))|pornhubvybmsymdol4iibwgwtkpwmeyd6luq2gxajgjzfjvotyt5zhyd\.onion)";

static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"https?://(?:(?:[a-zA-Z0-9.-]+\.)?{HOST}/(?:(?:view_video\.php|video/show)\?viewkey=|embed/)|(?:www\.)?thumbzilla\.com/video/)(?P<id>[\da-z]+)"
    ))
    .unwrap()
});
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^(?P<url>https?://(?:[a-zA-Z0-9.-]+\.)?{HOST}/(?:(?:user|channel)s|model|pornstar)/(?P<id>[^/?#&]+))(?P<rest>.*)$"
    ))
    .unwrap()
});
static PAGED_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^https?://(?:[^/]+\.)?{HOST}/(?P<id>(?:[^/]+/)*[^/?#&]+)"
    ))
    .unwrap()
});
static UPLOAD_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"(?P<url>https?://(?:[^/]+\.)?{HOST}/(?:(?:user|channel)s|model|pornstar)/(?P<id>[^/]+)/videos/upload)"
    ))
    .unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"(?P<url>https?://(?:[^/]+\.)?{HOST}/playlist/(?P<id>[^/?#&]+))"
    ))
    .unwrap()
});
static FLASHVARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+flashvars_\d+\s*=\s*(\{.+?\});").unwrap());
static PAGE_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bpage=(\d+)").unwrap());
static HAS_MORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<li[^>]+\bclass=["']page_next|<link[^>]+\brel=["']next|<button[^>]+\bid=["']moreDataBtn"#,
    )
    .unwrap()
});
static CONTAINER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)(<div[^>]+class=["']container.+)"#).unwrap());
static ENTRY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"href="/?(view_video\.php\?.*\bviewkey=[\da-z]+[^"]*)"[^>]*\s+title="([^"]+)""#)
        .unwrap()
});
static ERROR_DIV: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<div[^>]+class=["'][^"']*\b(?:removed|userMessageSection)\b[^"']*["'][^>]*>(?P<error>.+?)</div>"#,
    )
    .unwrap()
});
static ERROR_SECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<section[^>]+class=["']noVideo["'][^>]*>(?P<error>.+?)</section>"#).unwrap()
});
static GEO_BLOCKED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"class=["']geoBlocked["']|>\s*This content is unavailable in your country"#)
        .unwrap()
});
static TITLE_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<h1[^>]+class=["']title["'][^>]*>(?P<title>.+?)</h1>"#).unwrap()
});
static DATA_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div[^>]+data-video-title=["'](?P<title>[^"']+)["']"#).unwrap());
static SHARE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"shareTitle["']\s*[=:]\s*["'](?P<title>[^"']+)["']"#).unwrap());
static UPLOADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)From:&nbsp;.+?<(?:a\b[^>]+\bhref=["']/(?:(?:user|channel)s|model|pornstar)/|span\b[^>]+\bclass=["']username)[^>]+>(.+?)<"#,
    )
    .unwrap()
});
static VIEW_COUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<span class="count">([\d,\.]+)</span> [Vv]iews"#).unwrap());
static LIKE_COUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span[^>]+\bclass="votesUp"[^>]*>([\d,\.]+)</span>|<span[^>]+\bclass=["']votesUp["'][^>]*\bdata-rating=["'](\d+)"#)
        .unwrap()
});
static DISLIKE_COUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span[^>]+\bclass="votesDown"[^>]*>([\d,\.]+)</span>|<span[^>]+\bclass=["']votesDown["'][^>]*\bdata-rating=["'](\d+)"#)
        .unwrap()
});
static COMMENT_COUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"All Comments\s*<span>\(([\d,.]+)\)").unwrap());
static UPLOAD_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/(\d{6}/\d{2})/").unwrap());
static HEIGHT_IN_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?P<height>\d+)[pP]?_\d+[kK]").unwrap());
static PLAYLIST_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+playlistId\s*=\s*"([^"]+)""#).unwrap());
static ITEMS_COUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+itemsCount\s*=\s*([0-9]+)\s*\|\|").unwrap());
static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+token\s*=\s*"([^"]+)""#).unwrap());
static MODEL_PROFILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+MODEL_PROFILE\s*=").unwrap());

/// `_match_valid_url` of [`PornHub`].
pub fn suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

/// `PornHubUserIE._match_valid_url` (`(?:[?#&]|/(?!videos)|$)` after the ID).
pub fn user_suitable(url: &str) -> bool {
    user_match(url).is_some()
}

/// `PornHubPagedVideoListIE`'s own pattern, minus the classes that
/// `suitable` excludes.
pub fn paged_suitable(url: &str) -> bool {
    match PAGED_URL.captures(url) {
        Some(found) => {
            !found["id"].starts_with("playlist/")
                && !VIDEO_URL.is_match(url)
                && user_match(url).is_none()
                && !UPLOAD_URL.is_match(url)
        }
        None => false,
    }
}

/// `PornHubPlaylistIE._match_valid_url`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// [`USER_URL`] with the `(?!videos)` lookahead applied.
fn user_match(url: &str) -> Option<regex::Captures<'_>> {
    let found = USER_URL.captures(url)?;
    let rest = &found["rest"];
    let accepted = rest.is_empty()
        || rest.starts_with(['?', '#', '&'])
        || (rest.starts_with('/') && !rest.starts_with("/videos"));
    accepted.then_some(found)
}

/// `_download_webpage(..., impersonate=True)`: the extractor's impersonation
/// warning (once) when no target is available.
fn impersonated<T>(name: &'static str, request: impl FnOnce() -> Result<T>) -> Result<T> {
    let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
        crate::screen::warning_once(&format!("[{name}] {message}"));
    });
    request()
}

/// `_set_age_cookies(host)`.
fn set_age_cookies(host: &str) {
    for (name, value) in [
        ("age_verified", "1"),
        ("accessAgeDisclaimerPH", "1"),
        ("accessAgeDisclaimerUK", "1"),
        ("accessPH", "1"),
    ] {
        crate::cookies::set_cookie(host, name, value);
    }
}

/// `_get_headers(host)`.
fn site_headers(host: &str) -> Value {
    json!({
        "Origin": format!("https://www.{host}"),
        "Referer": format!("https://www.{host}/"),
    })
}

fn html_text(text: &str) -> String {
    crate::html::clean_html(text)
}

/// `_html_search_regex(pattern, webpage, name, group=...)` with
/// `clean_html`.
fn search_clean(pattern: &Regex, html: &str) -> Option<String> {
    pattern.captures(html).and_then(|found| {
        found
            .iter()
            .skip(1)
            .flatten()
            .next()
            .map(|m| html_text(m.as_str()))
    })
}

/// `str_to_int` of the count's text (`,`, `.` and `+` dropped).
fn count(html: &str, pattern: &Regex) -> Value {
    pattern
        .captures(html)
        .and_then(|found| {
            found
                .iter()
                .skip(1)
                .flatten()
                .next()
                .map(|m| m.as_str().replace([',', '.', '+'], ""))
        })
        .and_then(|text| text.parse::<i64>().ok())
        .map_or(Value::Null, Value::from)
}

/// `remove_start(value, prefix)` of a JSON string.
fn remove_start(value: Option<&Value>, prefix: &str) -> Value {
    match value.and_then(Value::as_str) {
        Some(text) => Value::from(text.strip_prefix(prefix).unwrap_or(text)),
        None => Value::Null,
    }
}

/// `url_result(target, ie, video_title=title)` as a playlist entry.
fn url_entry(target: &str, title: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), NAME.into());
    entry.insert("title".into(), title.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `_extract_entries(webpage, host)`.
fn extract_entries(webpage: &str, host: &str) -> Result<Vec<VideoInfo>> {
    // Only the container with the main playlist content (not the drop-down
    // menu, which uses a similar pattern).
    let container = CONTAINER.captures(webpage).map_or(webpage, |found| {
        found.get(1).map_or(webpage, |m| m.as_str())
    });
    let mut seen: Vec<(String, String)> = Vec::new();
    for found in ENTRY.captures_iter(container) {
        let pair = (found[1].to_owned(), found[2].to_owned());
        if !seen.contains(&pair) {
            seen.push(pair);
        }
    }
    seen.iter()
        .map(|(video_url, title)| url_entry(&format!("http://www.{host}/{video_url}"), title))
        .collect()
}

/// The `_real_extract` of [`PornHubUser`]: `url_result(videos_url, ...)`.
fn follow_entry(url: &Url, target: String) -> ExtractResult {
    let mut follow = super::direct::build_at(url.clone(), url, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
    ExtractResult::Video(follow)
}

impl PornHub {
    fn download_page(client: &Client, url: &Url, id: &str) -> Result<(String, Url)> {
        impersonated(NAME, || download_webpage_handle(client, url, NAME, id))
    }
}

impl Extractor for PornHub {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let host = found
            .name("host")
            .map_or("pornhub.com".to_owned(), |m| m.as_str().to_owned());
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        set_age_cookies(&host);

        // `dl_webpage('pc')`: the platform cookie and a viewkey check.
        crate::cookies::set_cookie(&host, "platform", "pc");
        let page_url = Url::parse(&format!(
            "https://www.{host}/view_video.php?viewkey={video_id}"
        ))?;
        let (webpage, final_url) = Self::download_page(client, &page_url, &video_id)?;
        if !final_url
            .query_pairs()
            .filter(|(key, _)| key == "viewkey")
            .last()
            .is_some_and(|(_, value)| value == video_id)
        {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Redirection detected; the video may be deleted or require login"
            )));
        }

        let error_msg = ERROR_DIV
            .captures(&webpage)
            .or_else(|| ERROR_SECTION.captures(&webpage))
            .map(|found| found["error"].to_owned());
        if let Some(error_msg) = error_msg {
            let text = crate::html::clean_html(&error_msg);
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: PornHub said: {text}"
            )));
        }
        if GEO_BLOCKED.is_match(&webpage) {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: This video is not available from your location due to geo restriction"
            )));
        }

        let title = crate::html::html_search_meta_clean(&webpage, "twitter:title")
            .or_else(|| search_clean(&TITLE_TAG, &webpage))
            .or_else(|| search_clean(&DATA_TITLE, &webpage))
            .or_else(|| search_clean(&SHARE_TITLE, &webpage));

        let flashvars: Map<String, Value> = FLASHVARS
            .captures(&webpage)
            .and_then(|found| serde_json::from_str::<Value>(&found[1]).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let mut subtitles = Map::new();
        let mut thumbnail = Value::Null;
        let mut duration = Value::Null;
        let mut video_urls: Vec<(String, Option<i64>)> = Vec::new();
        if !flashvars.is_empty() {
            if let Some(subtitle_url) = flashvars
                .get("closedCaptionsFile")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            {
                subtitles.insert("en".into(), json!([{"url": subtitle_url, "ext": "srt"}]));
            }
            thumbnail = flashvars.get("image_url").cloned().unwrap_or(Value::Null);
            duration = super::streamable::int_or_none(flashvars.get("video_duration"));
            if let Some(definitions) = flashvars.get("mediaDefinitions").and_then(Value::as_array) {
                for definition in definitions {
                    let Some(video_url) = definition.get("videoUrl").and_then(Value::as_str) else {
                        continue;
                    };
                    if video_url.is_empty() || video_urls.iter().any(|(url, _)| url == video_url) {
                        continue;
                    }
                    let quality = super::streamable::int_or_none(definition.get("quality"));
                    video_urls.push((video_url.to_owned(), quality.as_i64()));
                }
            }
        }

        if video_urls.is_empty() {
            if Regex::new(r#"<[^>]+\bid=["']lockedPlayer"#)
                .is_ok_and(|locked| locked.is_match(&webpage))
            {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Video {video_id} is locked"
                )));
            }
            return Err(Error::UnsupportedHtml5(format!(
                "{NAME} pages without flashvars media definitions (the JavaScript variable fallbacks) are not supported natively"
            )));
        }

        // `downloadBtn` links that are not yet listed.
        static DOWNLOAD_BUTTON: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"<a[^>]+\bclass=["']downloadBtn\b[^>]+\bhref=["'](?P<url>[^"']+)["']"#)
                .unwrap()
        });
        for found in DOWNLOAD_BUTTON.captures_iter(&webpage) {
            let video_url = found["url"].to_owned();
            if !video_urls.iter().any(|(url, _)| *url == video_url) {
                video_urls.push((video_url, None));
            }
        }

        let mut upload_date = None;
        let mut formats = Vec::new();
        let headers = site_headers(&host);
        let header_pairs: Vec<(&str, &str)> = vec![
            ("Origin", headers["Origin"].as_str().unwrap_or_default()),
            ("Referer", headers["Referer"].as_str().unwrap_or_default()),
        ];
        for (video_url, height) in &video_urls {
            if upload_date.is_none() {
                upload_date = UPLOAD_PATH
                    .captures(video_url)
                    .map(|found| found[1].replace('/', ""));
            }
            if video_url.contains("/video/get_media") {
                let medias = match Url::parse(video_url) {
                    Ok(media_url) => {
                        impersonated(NAME, || download_json(client, &media_url, NAME, &video_id))
                            .unwrap_or(Value::Null)
                    }
                    Err(_) => Value::Null,
                };
                for media in medias.as_array().into_iter().flatten() {
                    let Some(media_url) = media
                        .get("videoUrl")
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                    else {
                        continue;
                    };
                    let quality = super::streamable::int_or_none(media.get("quality")).as_i64();
                    add_format(
                        client,
                        &video_id,
                        &header_pairs,
                        &media_url,
                        quality,
                        &mut formats,
                    )?;
                }
                continue;
            }
            add_format(
                client,
                &video_id,
                &header_pairs,
                video_url,
                *height,
                &mut formats,
            )?;
        }

        let model_profile = super::search_json(&MODEL_PROFILE, &webpage, true)
            .flatten()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let uploader = search_clean(&UPLOADER, &webpage)
            .map(Value::from)
            .or_else(|| model_profile.get("username").cloned())
            .unwrap_or(Value::Null);
        let uploader_id = remove_start(model_profile.get("modelProfileLink"), "/model/");
        let view_count = count(&webpage, &VIEW_COUNT);
        let like_count = count(&webpage, &LIKE_COUNT);
        let dislike_count = count(&webpage, &DISLIKE_COUNT);
        let comment_count = count(&webpage, &COMMENT_COUNT);

        let mut info = super::json_ld::search(&webpage);
        // "description provided in JSON-LD is irrelevant".
        info.insert("description".into(), Value::Null);

        let labelled = |label: &str| -> Vec<Value> {
            static ELEMENT: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r#"(?s)<(?P<tag>\w+)\b[^>]*\bdata-label=["'](?P<label>[^"']+)["'][^>]*>(?P<inner>.*?)</\w+>"#)
                    .unwrap()
            });
            ELEMENT
                .captures_iter(&webpage)
                .filter(|found| &found["label"] == label)
                .map(|found| Value::from(html_text(&found["inner"])))
                .collect()
        };
        let tags = labelled("tag");
        let categories = labelled("category");
        let cast = labelled("pornstar");

        let mut literal: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("uploader", uploader),
            ("uploader_id", uploader_id),
            ("upload_date", upload_date.map_or(Value::Null, Value::from)),
            ("title", title.map_or(Value::Null, Value::from)),
            ("thumbnail", thumbnail),
            ("duration", duration),
            ("view_count", view_count),
            ("like_count", like_count),
            ("dislike_count", dislike_count),
            ("comment_count", comment_count),
            ("formats", Value::Null),
            ("age_limit", 18.into()),
            ("tags", Value::Array(tags)),
            ("categories", Value::Array(categories)),
            ("cast", Value::Array(cast)),
            ("subtitles", Value::Object(subtitles)),
            ("http_headers", headers),
        ];
        // `merge_dicts(literal, info)`: JSON-LD fills what is missing.
        let mut fields: Vec<(String, Value)> = Vec::new();
        for (key, value) in literal.drain(..) {
            if key == "formats" {
                fields.push((key.into(), Value::Null));
            } else if !value.is_null() {
                fields.push((key.into(), value));
            }
        }
        for (key, value) in info {
            if value.is_null() || fields.iter().any(|(existing, _)| *existing == key) {
                continue;
            }
            fields.push((key, value));
        }
        let fields: Vec<(&str, Value)> = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

/// `add_format` of the video page: DASH and HLS masters with the site's
/// headers, or a direct MP4 entry.
fn add_format(
    client: &Client,
    video_id: &str,
    headers: &[(&str, &str)],
    format_url: &str,
    height: Option<i64>,
    formats: &mut Vec<crate::VideoFormat>,
) -> Result<()> {
    let ext = super::direct::determine_ext(format_url).unwrap_or_default();
    let manifest = Url::parse(format_url)?;
    if ext == "mpd" {
        if let Some(info) = super::extract_mpd_optional(client, &manifest, NAME, video_id, false) {
            formats.extend(info.formats);
        }
        return Ok(());
    }
    if ext == "m3u8" {
        if let Some(info) = super::extract_m3u8_master_optional_headers_id(
            client,
            &manifest,
            NAME,
            video_id,
            Some("hls"),
            headers,
        ) {
            formats.extend(info.formats);
        }
        return Ok(());
    }
    let height = height.or_else(|| {
        HEIGHT_IN_URL
            .captures(format_url)
            .and_then(|found| found["height"].parse().ok())
    });
    let format_id = height.map(|height| format!("{height}p"));
    let mut fields: Vec<(&str, Value)> = vec![("url", format_url.into())];
    if let Some(format_id) = format_id {
        fields.push(("format_id", format_id.into()));
    }
    fields.push(("height", height.map_or(Value::Null, Value::from)));
    // The site's headers are the format's own, after its derived keys.
    let mut format = dict_format(fields, NAME, video_id)?;
    crate::info_order::mark_int_rates(&mut format);
    format.http_headers = Some(
        headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).into()))
            .collect(),
    );
    formats.push(format);
    Ok(())
}

impl Extractor for PornHubUser {
    fn key(&self) -> &'static str {
        USER_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = user_match(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(USER_NAME);
        let host = found
            .name("host")
            .map_or("pornhub.com".to_owned(), |m| m.as_str().to_owned());
        set_age_cookies(&host);
        let base = format!("{}/videos", &found["url"]);
        let mut videos = Url::parse(&base)?;
        if let Some(page) = PAGE_NUMBER.captures(url.as_str()) {
            videos = super::update_url_query(&videos, &[("page", &page[1])]);
        }
        Ok(Some(follow_entry(url, videos.to_string())))
    }
}

impl Extractor for PornHubPagedVideoList {
    fn key(&self) -> &'static str {
        PAGED_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !paged_suitable(url.as_str()) {
            return Ok(None);
        }
        let found = PAGED_URL
            .captures(url.as_str())
            .unwrap_or_else(|| unreachable!());
        let item_id = found["id"].to_owned();
        let host = VIDEO_URL
            .captures(url.as_str())
            .and_then(|found| found.name("host").map(|m| m.as_str().to_owned()))
            .or_else(|| {
                PAGED_URL
                    .captures(url.as_str())
                    .and_then(|found| found.name("host").map(|m| m.as_str().to_owned()))
            })
            .unwrap_or_else(|| "pornhub.com".to_owned());
        crate::screen::set_extractor(PAGED_NAME);
        crate::screen::screen(&format!(
            "[{PAGED_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        set_age_cookies(&host);

        let has_page = PAGE_NUMBER.is_match(url.as_str());
        let first_page: i64 = PAGE_NUMBER
            .captures(url.as_str())
            .and_then(|found| found[1].parse().ok())
            .unwrap_or(1);
        let mut entries = Vec::new();
        let mut page_num = first_page;
        loop {
            let page_text = page_num.to_string();
            let page_url = super::update_url_query(url, &[("page", page_text.as_str())]);
            let note_id = item_id.clone();
            let webpage = match impersonated(PAGED_NAME, || {
                download_webpage_handle(client, &page_url, PAGED_NAME, &note_id)
            }) {
                Ok((webpage, _)) => webpage,
                Err(error) => {
                    if page_num == first_page {
                        return Err(error);
                    }
                    break;
                }
            };
            let page_entries = extract_entries(&webpage, &host)?;
            if page_entries.is_empty() {
                break;
            }
            entries.extend(page_entries);
            if has_page || !HAS_MORE.is_match(&webpage) {
                break;
            }
            page_num += 1;
        }

        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: item_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: PAGED_NAME.into(),
            extractor_key: PAGED_NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra: untitled_extra(),
        })))
    }
}

impl Extractor for PornHubPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let host = found
            .name("host")
            .map_or("pornhub.com".to_owned(), |m| m.as_str().to_owned());
        let item_id = found["id"].to_owned();
        let playlist_url = Url::parse(&found["url"])?;
        crate::screen::set_extractor(PLAYLIST_NAME);
        crate::screen::screen(&format!(
            "[{PLAYLIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        set_age_cookies(&host);

        let (webpage, _) = download_webpage_handle(client, &playlist_url, PLAYLIST_NAME, &item_id)?;
        let missing = |what: &str| {
            Error::Reported(format!(
                "[{PLAYLIST_NAME}] {item_id}: Unable to extract {what}; please report this issue"
            ))
        };
        let playlist_id = PLAYLIST_ID
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| missing("playlist_id"))?;
        let video_count: f64 = ITEMS_COUNT
            .captures(&webpage)
            .and_then(|found| found[1].parse().ok())
            .ok_or_else(|| missing("video_count"))?;
        let token = TOKEN
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| missing("token"))?;
        let page_count = ((video_count - 36.0) / 40.0).ceil() as i64 + 1;

        let mut entries = extract_entries(&webpage, &host)?;
        for page_num in 2..=page_count {
            let chunk_url = Url::parse(&format!("https://www.{host}/playlist/viewChunked"))?;
            let page_text = page_num.to_string();
            let chunk_url = super::update_url_query(
                &chunk_url,
                &[
                    ("id", playlist_id.as_str()),
                    ("page", page_text.as_str()),
                    ("token", token.as_str()),
                ],
            );
            let (chunk, _) = impersonated(PLAYLIST_NAME, || {
                download_webpage_handle(client, &chunk_url, PLAYLIST_NAME, &item_id)
            })?;
            let page_entries = extract_entries(&chunk, &host)?;
            if page_entries.is_empty() {
                break;
            }
            entries.extend(page_entries);
        }

        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: item_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra: untitled_extra(),
        })))
    }
}

/// Playlist without a title: upstream's dict keeps only `id` of the title keys.
fn untitled_extra() -> Map<String, Value> {
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "playlist-dict;id".into(),
    );
    extra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.pornhub.com/view_video.php?viewkey=648719015"
        ));
        assert!(suitable(
            "https://www.thumbzilla.com/video/ph56c6114abd99a/horny-girlfriend-sex"
        ));
        assert!(!suitable("https://www.pornhub.com/model/zoe_ph"));
        assert!(user_suitable("https://www.pornhub.com/model/zoe_ph"));
        assert!(user_suitable("https://www.pornhub.com/model/zoe_ph?abc=1"));
        assert!(!user_suitable(
            "https://www.pornhub.com/model/zoe_ph/videos"
        ));
        assert!(paged_suitable(
            "https://www.pornhub.com/channels/povd/videos"
        ));
        assert!(!paged_suitable("https://www.pornhub.com/playlist/44121572"));
        assert!(playlist_suitable(
            "https://www.pornhub.com/playlist/44121572"
        ));
    }
}
