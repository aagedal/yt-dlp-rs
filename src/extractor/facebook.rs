//! Upstream `FacebookIE` (`yt_dlp/extractor/facebook.py`): the server JS
//! (`handleServerJS`/`s.handle`) and `bigPipe` pagelet `VideoConfig`
//! instances, the page's relay prefetched data (`parse_graphql_video`:
//! legacy playable URLs, the video delivery response's DASH manifests,
//! progressive and HLS URLs, and captions), watch parties (`livingRoom`
//! recaps) and the tahoe player fallback, with `extract_metadata`'s webpage
//! and JSON-LD fields; `FacebookReelIE`, `FacebookPluginsVideoIE` and
//! `FacebookRedirectURLIE` redirects; and `FacebookAdsIE`'s ad library
//! snapshots.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Facebook;
pub struct FacebookReel;
pub struct FacebookPluginsVideo;
pub struct FacebookRedirectUrl;
pub struct FacebookAds;

const NAME: &str = "facebook";
const KEY: &str = "Facebook";
const REDIRECT_NAME: &str = "FacebookRedirectURL";
const ADS_NAME: &str = "facebook:ads";
const ADS_KEY: &str = "FacebookAds";
const VIDEO_PAGE_TEMPLATE: &str = "https://www.facebook.com/video/video.php?v=";
const SUPPORTED_PAGELETS: &str =
    r"(?:pagelet_group_mall|permalink_video_pagelet|hyperfeed_story_id_[0-9a-f]+)";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:https?://(?:[\w-]+\.)?(?:facebook\.com|facebookwkhpilnemxj7asaniu7vnjjbiltxjqhye3mhbshg7kx5tfyd\.onion)/(?:[^#]*?\#!/)?(?:(?:permalink\.php|video/video\.php|photo\.php|video\.php|video/embed|story\.php|watch(?:/live)?/?)\?(?:.*?)(?:v|video_id|story_fbid)=|[^/]+/videos/(?:[^/]+/)?|[^/]+/posts/|events/(?:[^/]+/)?|groups/[^/]+/(?:permalink|posts)/(?:[\da-f]+/)?|watchparty/)|facebook:)(?P<id>pfbid[A-Za-z0-9]+|\d+)",
    )
    .unwrap()
});
static REEL_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:[\w-]+\.)?facebook\.com/reel/(?P<id>\d+)").unwrap());
static PLUGINS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:[\w-]+\.)?facebook\.com/plugins/video\.php\?.*?\bhref=(?P<id>https.+)",
    )
    .unwrap()
});
static REDIRECT_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:[\w-]+\.)?facebook\.com/flx/warn[/?]").unwrap());
static ADS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:[\w-]+\.)?facebook\.com/ads/library/?\?(?:[^#]+&)?id=(?P<id>\d+)")
        .unwrap()
});
static RELAY_VIDEO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"data-sjs>(\{.*?"(?:dash_manifest|playable_url(?:_quality_hd)?).*?\})</script>"#)
        .unwrap()
});
static RELAY_LOGIN_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-sjs>(\{.*?"login_data"\s*:\s*\{.*?\})</script>"#).unwrap());
static RELAY_API_CONFIG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"data-sjs>(\{.*?\[\s*"RelayAPIConfigDefaults"\s*,.*?\})</script>"#).unwrap()
});
/// The `server js data` patterns, tried in order.
static SERVER_JS_DATA: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"handleServerJS\((\{.+\})(?:\);|,")"#).unwrap(),
        Regex::new(r"\bs\.handle\((\{.+?\})\);").unwrap(),
    ]
});
/// The `js data` (`bigPipe` pagelet) patterns, tried in order.
static PAGELET_DATA: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(&format!(
            r#"bigPipe\.onPageletArrive\((\{{.+?\}})\)\s*;\s*\}}\s*\)\s*,\s*["']onPageletArrive\s+{SUPPORTED_PAGELETS}"#
        ))
        .unwrap(),
        Regex::new(&format!(
            r#"bigPipe\.onPageletArrive\((\{{.*?id\s*:\s*"{SUPPORTED_PAGELETS}".*?\}})\);"#
        ))
        .unwrap(),
    ]
});
static TAHOE_JS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"for\s+\(\s*;\s*;\s*\)\s*;(.+)").unwrap());
static PKG_COHORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"pkg_cohort["']\s*:\s*["'](.+?)["']"#).unwrap());
static CLIENT_REVISION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"client_revision["']\s*:\s*(\d+),"#).unwrap());
static DTSG_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""DTSGInitialData"\s*,\s*\[\]\s*,\s*\{\s*"token"\s*:\s*"([^"]+)""#).unwrap()
});
static SERVER_JS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"data-sjs>(\{.*?ScheduledServerJS.*?\})</script>").unwrap());
static UTIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<abbr[^>]+data-utime=["'](\d+)"#).unwrap());
static VIEW_COUNT: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"\bviewCount\s*:\s*["']([\d,.]+)"#).unwrap(),
        Regex::new(r#"video_view_count["']\s*:\s*(\d+)"#).unwrap(),
    ]
});
static OWNER_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"ownerName\s*:\s*"([^"]+)""#).unwrap());
static FACEBOOK_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\|\s*Facebook$").unwrap());
static IMAGE_EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(?:jpg|png)").unwrap());

/// `FacebookIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `FacebookReelIE._match_valid_url`.
pub fn reel_suitable(url: &str) -> bool {
    REEL_URL.is_match(url)
}

/// `FacebookPluginsVideoIE._match_valid_url`.
pub fn plugins_suitable(url: &str) -> bool {
    PLUGINS_URL.is_match(url)
}

/// `FacebookRedirectURLIE._match_valid_url`.
pub fn redirect_suitable(url: &str) -> bool {
    REDIRECT_URL.is_match(url)
}

/// `FacebookAdsIE._match_valid_url`.
pub fn ads_suitable(url: &str) -> bool {
    ADS_URL.is_match(url)
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn float(value: Option<&Value>, scale: f64) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .map(|number| number / scale)
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn children(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(object) => object.values().collect(),
        _ => Vec::new(),
    }
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// `_parse_json(text, video_id, fatal=False)`: the value, or a warning.
fn parse_json(text: &str, id: &str) -> Option<Value> {
    match serde_json::from_str(text) {
        Ok(value) => Some(value),
        Err(error) => {
            crate::screen::warning(&format!("[{NAME}] {id}: Failed to parse JSON: {error}"));
            None
        }
    }
}

/// `_search_regex(patterns, text, name, default=None)`: the first matching
/// pattern's group.
fn search_first<'a>(patterns: &[Regex], text: &'a str) -> Option<&'a str> {
    patterns
        .iter()
        .find_map(|pattern| pattern.captures(text))
        .and_then(|found| found.get(1))
        .map(|group| group.as_str())
}

/// `extract_video_data(instances)`: each `VideoConfig` instance's
/// `videoData`.
fn extract_video_data(instances: Option<&Value>) -> Vec<Value> {
    let mut video_data = Vec::new();
    for item in instances.and_then(Value::as_array).into_iter().flatten() {
        if item.pointer("/1/0").and_then(Value::as_str) != Some("VideoConfig") {
            continue;
        }
        let Some(video_item) = item.pointer("/2/0") else {
            continue;
        };
        if truthy(video_item.get("video_id")) {
            video_data.push(video_item.get("videoData").cloned().unwrap_or(Value::Null));
        }
    }
    video_data
}

/// `extract_from_jsmods_instances(js_data)`.
fn jsmods_video_data(js_data: Option<&Value>) -> Vec<Value> {
    match js_data {
        Some(data) if truthy(Some(data)) => {
            extract_video_data(data.pointer("/jsmods/instances").filter(|v| v.is_array()))
        }
        _ => Vec::new(),
    }
}

/// Python's `str(value)` of a JSON scalar.
fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `urlencode_postdata(pairs)` (`urllib.parse.urlencode`'s `quote_plus`).
fn urlencode(pairs: &[(String, String)]) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    const QUOTE_PLUS: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'_')
        .remove(b'.')
        .remove(b'-')
        .remove(b'~');
    let quote = |text: &str| {
        utf8_percent_encode(text, QUOTE_PLUS)
            .to_string()
            .replace("%20", "+")
    };
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", quote(key), quote(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `yield_all_relay_data(filter)`: every matching `data-sjs` document
/// (`{}` for an unparsable one).
fn all_relay_data(webpage: &str, filter: &Regex, id: &str) -> Vec<Value> {
    filter
        .captures_iter(webpage)
        .map(|found| parse_json(&found[1], id).unwrap_or_else(|| json!({})))
        .collect()
}

/// `extract_relay_prefetched_data(filter, target_keys)`: the first
/// prefetched result `data` dict (holding one of the target keys).
fn relay_prefetched_data(
    webpage: &str,
    filter: &Regex,
    targets: Option<&[&str]>,
    id: &str,
) -> Option<Map<String, Value>> {
    for relay in all_relay_data(webpage, filter, id) {
        let Some(require) = relay.get("require") else {
            continue;
        };
        // `(None, (..., ..., ..., '__bbox', 'require'))`.
        let mut lists = vec![require];
        for a in children(require) {
            for b in children(a) {
                for c in children(b) {
                    if let Some(inner) = c.pointer("/__bbox/require") {
                        lists.push(inner);
                    }
                }
            }
        }
        for list in lists {
            for entry in children(list) {
                let prefetched = children(entry).first().is_some_and(|key| {
                    key.as_str()
                        .is_some_and(|key| key.starts_with("RelayPrefetchedStreamCache"))
                });
                if !prefetched {
                    continue;
                }
                for item in children(entry) {
                    for candidate in children(item) {
                        if let Some(data) = candidate
                            .pointer("/__bbox/result/data")
                            .and_then(Value::as_object)
                            .filter(|data| {
                                targets.is_none_or(|targets| {
                                    targets.iter().any(|key| data.contains_key(*key))
                                })
                            })
                        {
                            return Some(data.clone());
                        }
                    }
                }
            }
        }
    }
    None
}

/// The `post` results of `extract_metadata`: every server JS result's
/// `data` dict.
fn post_data(webpage: &str) -> Vec<Value> {
    let mut post = Vec::new();
    for found in SERVER_JS.captures_iter(webpage) {
        let Ok(data) = serde_json::from_str::<Value>(&found[1]) else {
            continue;
        };
        let Some(require) = data.get("require") else {
            continue;
        };
        for a in children(require) {
            for b in children(a) {
                for c in children(b) {
                    let Some(inner) = c.pointer("/__bbox/require") else {
                        continue;
                    };
                    for d in children(inner) {
                        for e in children(d) {
                            for f in children(e) {
                                if let Some(result) = f
                                    .pointer("/__bbox/result/data")
                                    .filter(|data| data.is_object())
                                {
                                    post.push(result.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    post
}

/// `_search_regex(patterns, webpage, group='content')` over the title
/// sources `extract_metadata` tries.
fn page_title(webpage: &str) -> Option<String> {
    static HEADER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<h2\s+[^>]*class="uiHeaderTitle"[^>]*>(?P<content>[^<]*)</h2>"#).unwrap()
    });
    static CAPTION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<span class="fbPhotosPhotoCaption".*?id="fbPhotoPageCaption"><span class="hasCaption">(?P<content>.*?)</span>"#).unwrap()
    });
    static TITLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<title>(?P<content>.+?)</title>").unwrap());
    let clean = |text: &str| Some(crate::html::clean_html(text)).filter(|text| !text.is_empty());
    HEADER
        .captures(webpage)
        .and_then(|found| clean(&found["content"]))
        .or_else(|| {
            CAPTION
                .captures(webpage)
                .and_then(|found| clean(&found["content"]))
        })
        .or_else(|| crate::html::html_search_meta(webpage, "og:title"))
        .or_else(|| crate::html::html_search_meta(webpage, "twitter:title"))
        .or_else(|| {
            TITLE
                .captures(webpage)
                .and_then(|found| clean(&found["content"]))
        })
}

/// `extract_metadata(webpage)`: the webpage's title, description,
/// uploader, timestamp, thumbnail and counts.
fn metadata(webpage: &str, video_id: &str) -> Map<String, Value> {
    let post = post_data(webpage);
    let media: Vec<&Value> = post
        .iter()
        .filter_map(|data| data.get("attachments"))
        .flat_map(children)
        .filter_map(|attachment| attachment.get("media"))
        .filter(|media| {
            str_or_none(media.get("id")).as_deref() == Some(video_id)
                && media.get("__typename").and_then(Value::as_str) == Some("Video")
        })
        .collect();
    let first_text = |path: &str| {
        media.iter().find_map(|media| {
            media
                .pointer(path)
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
    };
    let title = first_text("/title/text");
    // An empty media title or message (reels) falls back like `or`.
    let description = first_text("/creation_story/comet_sections/message/story/message/text")
        .filter(|text| !text.is_empty());
    // `_search_json_ld(webpage, video_id, default={})`.
    let mut info = super::json_ld::search(webpage);
    // `title or info_json_ld.get('title') or page_title`.
    let fallback_title = title
        .clone()
        .filter(|title| !title.is_empty())
        .or_else(|| {
            info.get("title")
                .and_then(Value::as_str)
                .filter(|title| !title.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| page_title(webpage));
    let description = description.or_else(|| {
        ["description", "og:description", "twitter:description"]
            .iter()
            .find_map(|name| crate::html::html_search_meta(webpage, name))
    });
    let named_owner = |owner: Option<&Value>| {
        owner
            .filter(|owner| {
                owner
                    .get("name")
                    .is_some_and(|name| !name.is_null() && name != "")
            })
            .cloned()
    };
    let uploader_data = media
        .iter()
        .find_map(|media| {
            media
                .get("owner")
                .filter(|owner| owner.is_object())
                .cloned()
        })
        .or_else(|| {
            post.iter().find_map(|data| {
                data.pointer("/video/creation_story/attachments")
                    .map(children)
                    .into_iter()
                    .flatten()
                    .find_map(|attachment| named_owner(attachment.pointer("/media/owner")))
            })
        })
        .or_else(|| {
            post.iter().find_map(|data| {
                children(data)
                    .into_iter()
                    .find_map(|value| named_owner(value.pointer("/video/owner")))
            })
        })
        .or_else(|| {
            post.iter().find_map(|data| {
                data.pointer("/node/actors")
                    .map(children)
                    .into_iter()
                    .flatten()
                    .find(|actor| actor.is_object())
                    .cloned()
            })
        })
        .or_else(|| {
            post.iter().find_map(|data| {
                data.pointer("/event/event_creator")
                    .filter(|v| v.is_object())
                    .cloned()
            })
        })
        .or_else(|| {
            post.iter().find_map(|data| {
                data.pointer("/video/creation_story/short_form_video_context/video_owner")
                    .filter(|v| v.is_object())
                    .cloned()
            })
        })
        .unwrap_or_else(|| json!({}));
    let uploader = uploader_data
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            crate::html_elements::element_html_by_id("fbPhotoPageAuthorName", webpage)
                .ok()
                .flatten()
                .map(|element| crate::html::clean_html(&element))
                .filter(|name| !name.is_empty())
        })
        .or_else(|| {
            OWNER_NAME
                .captures(webpage)
                .map(|found| found[1].to_owned())
                // `_search_regex`: the content as written (still escaped).
                .or_else(|| crate::html::og_search_raw(webpage, "title"))
                .or_else(|| {
                    super::search_warning(NAME, "uploader");
                    None
                })
        });
    let timestamp = UTIME
        .captures(webpage)
        .and_then(|found| found[1].parse::<i64>().ok());
    let thumbnail = ["og:image", "twitter:image"]
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name))
        .filter(|thumbnail| IMAGE_EXT.is_match(thumbnail));
    let view_count = VIEW_COUNT
        .iter()
        .find_map(|pattern| pattern.captures(webpage))
        .and_then(|found| super::youtube::dict::parse_count(&found[1]));
    let concurrent = post.iter().find_map(|data| {
        let mut sources: Vec<&Value> = data.get("video").into_iter().collect();
        for a in children(data) {
            for b in children(a) {
                if let Some(attachments) = b.get("attachments") {
                    for attachment in children(attachments) {
                        sources.extend(attachment.get("media"));
                    }
                }
            }
        }
        sources
            .into_iter()
            .find_map(|source| int(source.get("liveViewerCount")))
    });
    // `info_json_ld['title'] = ...`, then `merge_dicts(info_json_ld, info_dict)`.
    let title_text = FACEBOOK_SUFFIX
        .replace(fallback_title.as_deref().unwrap_or_default(), "")
        .into_owned();
    let title = Some(title_text)
        .filter(|title| !title.is_empty())
        .or_else(|| {
            description
                .as_deref()
                .map(|description| description.replace('\n', " "))
                .filter(|description| !description.is_empty())
        })
        .unwrap_or_else(|| format!("Facebook video #{video_id}"));
    info.insert("title".into(), title.into());
    let json_ld = info;
    let mut info = Map::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value.filter(|value| !value.is_null()) {
            info.insert(key.to_owned(), value);
        }
    };
    put("description", description.map(Value::from));
    put("uploader", uploader.map(Value::from));
    put("uploader_id", uploader_data.get("id").cloned());
    put("timestamp", timestamp.map(Value::from));
    put("thumbnail", thumbnail.map(Value::from));
    put("view_count", view_count.map(Value::from));
    put("concurrent_view_count", concurrent.map(Value::from));
    if let Some(feedback) = post.iter().find_map(|data| {
        data.get("url")
            .and_then(Value::as_str)
            .filter(|url| url.contains(video_id))
            .and(data.get("feedback"))
    }) {
        put(
            "like_count",
            feedback
                .pointer("/likers/count")
                .filter(|count| count.is_i64())
                .cloned(),
        );
        put(
            "comment_count",
            feedback
                .pointer("/total_comment_count")
                .filter(|count| count.is_i64())
                .cloned(),
        );
        put(
            "repost_count",
            feedback
                .pointer("/share_count_reduced")
                .and_then(Value::as_str)
                .and_then(super::youtube::dict::parse_count)
                .map(Value::from),
        );
    }
    merge_dicts(json_ld, info)
}

/// `extract_dash_manifest(vid_data, formats, mpd_url)`.
fn dash_manifest(
    data: &Value,
    formats: &mut Vec<crate::VideoFormat>,
    mpd_url: Option<&str>,
    id: &str,
) {
    let Some(manifest) = [
        "dash_manifest",
        "playlist",
        "dash_manifest_xml_string",
        "manifest_xml",
    ]
    .iter()
    .find_map(|key| data.get(*key).and_then(Value::as_str)) else {
        return;
    };
    // `urllib.parse.unquote_plus`.
    let text = percent_encoding::percent_decode_str(&manifest.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned();
    let base = data
        .get("dash_manifest_url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .or_else(|| mpd_url.map(str::to_owned));
    let Some(base) = base.and_then(|base| Url::parse(&base).ok()) else {
        crate::screen::warning(&format!(
            "[{NAME}] {id}: DASH manifests without a manifest URL are not supported natively"
        ));
        return;
    };
    match crate::dash::parse_with_subtitles(&text, &base) {
        Ok((parsed, subtitles)) => {
            // `_parse_mpd_formats(doc, mpd_url=...)`: no base URL besides
            // the document's own, and the subtitles ignored.
            if !subtitles.is_empty() {
                crate::screen::warning_once(&format!(
                    "[{NAME}] Ignoring subtitle tracks found in the DASH manifest; if any subtitle tracks are missing, {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
            }
            let directory = base.join(".").map(String::from).unwrap_or_default();
            for mut format in parsed {
                if format
                    .extra
                    .get("fragment_base_url")
                    .and_then(Value::as_str)
                    == Some(directory.as_str())
                {
                    format.extra.insert("fragment_base_url".into(), "".into());
                }
                formats.push(format);
            }
        }
        Err(error) => crate::screen::warning(&format!(
            "[{NAME}] {id}: {}",
            crate::upstream_error_text(&error)
        )),
    }
}

fn simple_format(
    format_id: Option<&str>,
    quality: i64,
    url: &str,
    id: &str,
) -> Result<crate::VideoFormat> {
    let mut fields = Vec::new();
    if let Some(format_id) = format_id {
        fields.push(("format_id", Value::from(format_id)));
    }
    fields.push(("quality", quality.into()));
    fields.push(("url", url.into()));
    dict_format(fields, id)
}

/// An extractor's format dict, its keys in order (`None` values kept as
/// keys).
fn dict_format(fields: Vec<(&str, Value)>, id: &str) -> Result<crate::VideoFormat> {
    let mut format = Map::new();
    let mut keys = Vec::new();
    for (key, value) in fields {
        keys.push(key);
        if !value.is_null() {
            format.insert(key.into(), value);
        }
    }
    if !format.contains_key("ext") {
        let url = format
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let ext = super::direct::determine_ext(url).unwrap_or_else(|| "unknown_video".into());
        format.insert("ext".into(), ext.into());
    }
    if !format.contains_key("format_id") {
        format.insert("format_id".into(), "".into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_rates(&mut format);
    Ok(format)
}

/// `process_formats(info)`: a non-browser User-Agent and chunked downloads.
fn process_formats(formats: &mut [crate::VideoFormat]) {
    for format in formats {
        crate::info_order::mark_format_update(format, "http_headers");
        crate::info_order::mark_format_update(format, "downloader_options");
        let mut headers = format.http_headers.take().unwrap_or_default();
        headers.insert("User-Agent".into(), "facebookexternalhit/1.1".into());
        format.http_headers = Some(headers);
        let options = format
            .extra
            .entry("downloader_options")
            .or_insert_with(|| json!({}));
        if let Some(options) = options.as_object_mut() {
            options.insert("http_chunk_size".into(), (250_i64 << 20).into());
        }
    }
}

/// A parsed video's dict and formats.
type Entry = (Map<String, Value>, Vec<crate::VideoFormat>);

/// `parse_graphql_video(video)`: the video's dict and formats, if any.
fn graphql_video(
    client: &Client,
    video: &Value,
    video_id: &str,
    webpage: &str,
) -> Result<Option<Entry>> {
    let v_id = str_or_none(video.get("videoId"))
        .or_else(|| str_or_none(video.get("id")))
        .unwrap_or_else(|| video_id.to_owned());
    let mut video = video.clone();
    if let Some(reel) = video
        .pointer("/creation_story/short_form_video_context/playback_video")
        .filter(|value| value.is_object())
        .cloned()
    {
        let mut story = video
            .get("creation_story")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let owner = story
            .pointer("/short_form_video_context/video_owner")
            .cloned()
            .unwrap_or(Value::Null);
        if let Some(object) = story.as_object_mut() {
            object.insert("owner".into(), owner);
            if let Some(reel) = reel.as_object() {
                object.extend(reel.clone());
            }
        }
        video = story;
    }
    let mut formats = Vec::new();
    let quality = |format_id: Option<&str>| match format_id {
        Some("sd") => 0 - 3,
        Some("hd") => 1 - 3,
        _ => -1 - 3,
    };
    let legacy = video
        .get("videoDeliveryLegacyFields")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| video.clone());
    for (key, format_id) in [
        ("playable_url", "sd"),
        ("playable_url_quality_hd", "hd"),
        ("playable_url_dash", ""),
        ("browser_native_hd_url", "hd"),
        ("browser_native_sd_url", "sd"),
    ] {
        let Some(url) = legacy
            .get(key)
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        if super::direct::determine_ext(url).as_deref() == Some("mpd") {
            if let Some(stream) = Url::parse(url)
                .ok()
                .and_then(|mpd| super::extract_mpd_optional(client, &mpd, NAME, video_id, false))
            {
                formats.extend(stream.formats);
            }
        } else {
            formats.push(simple_format(
                Some(format_id),
                quality(Some(format_id)),
                url,
                video_id,
            )?);
        }
    }
    dash_manifest(&legacy, &mut formats, None, video_id);
    let delivery = video
        .pointer("/videoDeliveryResponseFragment/videoDeliveryResponseResult")
        .cloned()
        .unwrap_or(Value::Null);
    let mpd_urls: Vec<String> = delivery
        .get("dash_manifest_urls")
        .map(children)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("manifest_url").and_then(Value::as_str))
        .filter_map(super::json_ld::url_or_none)
        .collect();
    let manifests: Vec<&Value> = delivery
        .get("dash_manifests")
        .map(children)
        .into_iter()
        .flatten()
        .filter(|manifest| {
            manifest
                .get("manifest_xml")
                .is_some_and(|xml| !xml.is_null() && xml != "")
        })
        .collect();
    for (index, manifest) in manifests.iter().enumerate() {
        dash_manifest(
            manifest,
            &mut formats,
            mpd_urls.get(index).map(String::as_str),
            video_id,
        );
    }
    if manifests.is_empty() {
        for mpd_url in &mpd_urls {
            if let Some(stream) = Url::parse(mpd_url)
                .ok()
                .and_then(|mpd| super::extract_mpd_optional(client, &mpd, NAME, video_id, false))
            {
                formats.extend(stream.formats);
            }
        }
    }
    for progressive in delivery
        .get("progressive_urls")
        .map(children)
        .into_iter()
        .flatten()
    {
        let Some(url) = progressive
            .get("progressive_url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let format_id = progressive
            .pointer("/metadata/quality")
            .and_then(Value::as_str)
            .map(str::to_lowercase);
        formats.push(simple_format(
            format_id.as_deref(),
            quality(format_id.as_deref()),
            url,
            video_id,
        )?);
    }
    for hls in delivery
        .get("hls_playlist_urls")
        .map(children)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("hls_playlist_url").and_then(Value::as_str))
        .filter_map(super::json_ld::url_or_none)
    {
        if let Some(stream) = Url::parse(&hls).ok().and_then(|manifest| {
            super::extract_m3u8_optional(client, &manifest, NAME, video_id, Some("hls"), false)
        }) {
            formats.extend(stream.formats);
        }
    }
    if formats.is_empty() {
        return Ok(None);
    }
    // Captions, by locale.
    let broadcast = video.get("is_video_broadcast").and_then(Value::as_bool) == Some(true);
    let mut automatic = Map::new();
    let mut subtitles = Map::new();
    let mut captions: Vec<&Value> = video
        .get("video_available_captions_locales")
        .and_then(Value::as_array)
        .map(|captions| captions.iter().collect())
        .unwrap_or_default();
    captions.sort_by(|a, b| {
        let locale = |caption: &Value| {
            caption
                .get("locale")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        locale(a).cmp(&locale(b))
    });
    for caption in captions {
        let Some(url) = caption
            .get("captions_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let lang = caption
            .get("localized_language")
            .and_then(Value::as_str)
            .filter(|lang| !lang.is_empty())
            .unwrap_or("und");
        let name = caption
            .get("localized_country")
            .and_then(Value::as_str)
            .filter(|country| !country.is_empty())
            .map_or_else(|| lang.to_owned(), |country| format!("{lang} ({country})"));
        let locale = caption
            .get("locale")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let target = if truthy(caption.get("localized_creation_method")) || broadcast {
            &mut automatic
        } else {
            &mut subtitles
        };
        if let Value::Array(tracks) = target.entry(locale).or_insert_with(|| json!([])) {
            tracks.push(json!({"url": url, "name": name, "ext": subtitle_ext(&url)}));
        }
    }
    if let Some(url) = video
        .get("captions_url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        && automatic.is_empty()
        && subtitles.is_empty()
    {
        let locale = ["og:locale", "twitter:locale"]
            .iter()
            .find_map(|name| crate::html::html_search_meta(webpage, name))
            .unwrap_or_else(|| "en_US".into());
        let target = if broadcast {
            &mut automatic
        } else {
            &mut subtitles
        };
        target.insert(locale, json!([{"url": url, "ext": subtitle_ext(&url)}]));
    }
    let mut info = Map::new();
    info.insert("id".into(), v_id.clone().into());
    // The formats themselves are returned apart (a placeholder keeps the key).
    info.insert("formats".into(), json!([]));
    info.insert(
        "thumbnail".into(),
        video
            .pointer("/thumbnailImage/uri")
            .or_else(|| video.pointer("/preferred_thumbnail/image/uri"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    info.insert(
        "uploader_id".into(),
        str_or_none(video.pointer("/owner/id")).map_or(Value::Null, Value::from),
    );
    info.insert(
        "timestamp".into(),
        int(video.get("publish_time"))
            .or_else(|| int(video.get("creation_time")))
            .map_or(Value::Null, Value::from),
    );
    info.insert(
        "duration".into(),
        float(video.get("playable_duration_in_ms"), 1000.0)
            .filter(|duration| *duration != 0.0)
            .or_else(|| float(video.get("length_in_second"), 1.0))
            .map_or(Value::Null, |duration| json!(duration)),
    );
    info.insert("automatic_captions".into(), Value::Object(automatic));
    info.insert("subtitles".into(), Value::Object(subtitles));
    let description = video
        .pointer("/savable_description/text")
        .cloned()
        .unwrap_or(Value::Null);
    match video
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    {
        Some(name) => {
            info.insert("title".into(), name.into());
            info.insert("description".into(), description);
        }
        None => {
            info.insert(
                "title".into(),
                description
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .map_or_else(|| format!("Facebook video #{v_id}"), str::to_owned)
                    .into(),
            );
        }
    }
    process_formats(&mut formats);
    Ok(Some((info, formats)))
}

/// Processing's subtitle `ext` (`determine_ext(url)`).
fn subtitle_ext(url: &str) -> String {
    super::direct::determine_ext(url).unwrap_or_else(|| "unknown_video".into())
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
    }
}

/// `merge_dicts(a, b)`: `a`'s non-`None` values, then `b`'s new ones (or
/// replacing `a`'s empty strings).
fn merge_dicts(first: Map<String, Value>, second: Map<String, Value>) -> Map<String, Value> {
    let mut merged = Map::new();
    for (key, value) in first.into_iter().chain(second) {
        let replace = match merged.get(&key) {
            None => !value.is_null(),
            Some(Value::String(existing)) => existing.is_empty() && value.is_string(),
            Some(_) => false,
        };
        if replace {
            merged.insert(key, value);
        }
    }
    merged
}

/// A video from upstream's dict (`dict;KEYS` order).
fn video(url: &Url, dict: Map<String, Value>, formats: Vec<crate::VideoFormat>) -> VideoInfo {
    video_as(url, dict, formats, (NAME, KEY), "url,name")
}

/// [`video`] for an extractor `(IE_NAME, ie_key)`, its subtitle tracks'
/// keys in order.
fn video_as(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    (name, key): (&str, &str),
    subtitle_keys: &str,
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
    info.webpage_url = url.clone();
    info.extractor = name.into();
    info.extractor_key = key.into();
    let mut keys = Vec::new();
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "formats" => {}
            "subtitles" | "automatic_captions" => {
                let parsed: crate::Subtitles = serde_json::from_value(value).unwrap_or_default();
                if key == "subtitles" {
                    info.subtitles = parsed;
                } else {
                    info.automatic_captions = parsed;
                }
            }
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    crate::info_order::mark_info(
        info,
        &format!("dict;{};subtitle={subtitle_keys}", keys.join(",")),
    )
}

/// `url_result(url)`, followed by the registry.
fn follow(target: &str) -> Result<ExtractResult> {
    let target = Url::parse(target)?;
    let mut info = super::direct::build_at(target.clone(), &target, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    Ok(ExtractResult::Video(info))
}

/// `url_result(url, FacebookIE, video_id, video_title)`: a playlist entry
/// extracted when processed.
fn url_entry(target: &Url, id: Option<String>, title: Option<String>) -> VideoInfo {
    let mut info = super::direct::build_at(target.clone(), target, String::new());
    info.formats.clear();
    info.direct = None;
    // `get_temp_id(url)` without an id.
    if let Some(id) = id.clone().or_else(|| {
        VALID_URL
            .captures(target.as_str())
            .map(|found| found["id"].to_owned())
    }) {
        info.id = id;
    }
    let mut entry = Map::new();
    entry.insert("ie_key".into(), KEY.into());
    if let Some(id) = id {
        entry.insert("id".into(), id.into());
    }
    if let Some(title) = title {
        entry.insert("title".into(), title.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.as_str().into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target.as_str(),
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    info
}

/// `playlist_result(entries, video_id)`.
fn id_playlist(url: &Url, id: &str, entries: Vec<VideoInfo>) -> ExtractResult {
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "playlist-dict;id".into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id: id.to_owned(),
        title: String::new(),
        webpage_url: url.clone(),
        extractor: NAME.into(),
        extractor_key: KEY.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// An unexpected upstream exception (`KeyError` and the like).
fn bug(id: &str, cause: &str) -> Error {
    fail(
        id,
        &format!(
            "An extractor error has occurred. (caused by {cause}); {}",
            crate::BUG_REPORTS_MESSAGE
        ),
    )
}

/// `_download_webpage`/`_download_json` of a form POST: the body text.
fn post_form(
    client: &Client,
    url: &Url,
    id: &str,
    what: &'static str,
    body: String,
) -> Result<String> {
    use std::io::Read;
    let mut response = super::direct::post(
        client,
        url,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body,
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: NAME,
            id: id.to_owned(),
            what,
            source: Box::new(error),
        },
        error => error,
    })?;
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The server JS (`handleServerJS`/`s.handle`) `VideoConfig` instances.
fn server_js_video_data(webpage: &str, id: &str) -> Vec<Value> {
    let Some(text) = search_first(&*SERVER_JS_DATA, webpage) else {
        return Vec::new();
    };
    match parse_json(text, id) {
        Some(data) if truthy(Some(&data)) => extract_video_data(data.get("instances")),
        _ => Vec::new(),
    }
}

/// The `bigPipe.onPageletArrive` pagelet's `jsmods` instances
/// (`js_to_json`).
fn pagelet_video_data(webpage: &str, id: &str) -> Vec<Value> {
    let Some(text) = search_first(&*PAGELET_DATA, webpage) else {
        return Vec::new();
    };
    jsmods_video_data(parse_json(&crate::js_to_json::js_to_json(text), id).as_ref())
}

/// The tahoe player's `for (;;);` payload (a secondary POST request).
fn tahoe_video_data(client: &Client, id: &str, webpage: &str) -> Result<Vec<Value>> {
    let url = Url::parse(&format!(
        "https://www.facebook.com/video/tahoe/async/{id}/?chain=true&isvideo=true&payloadtype=primary"
    ))?;
    let group = |pattern: &Regex, default: &str| {
        pattern
            .captures(webpage)
            .map_or_else(|| default.to_owned(), |found| found[1].to_owned())
    };
    let body = urlencode(&[
        ("__a".into(), "1".into()),
        ("__pc".into(), group(&PKG_COHORT, "PHASED:DEFAULT")),
        ("__rev".into(), group(&CLIENT_REVISION, "3944515")),
        ("fb_dtsg".into(), group(&DTSG_TOKEN, "")),
    ]);
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading webpage"));
    let text = post_form(client, &url, id, "webpage", body)?;
    let js = TAHOE_JS
        .captures(&text)
        .map_or_else(|| "{}".to_owned(), |found| found[1].to_owned());
    Ok(jsmods_video_data(parse_json(&js, id).as_ref()))
}

/// The watch party's (`livingRoom`) recap videos, through the GraphQL API.
fn watch_party(
    client: &Client,
    url: &Url,
    real_url: &Url,
    video_id: &str,
    webpage: &str,
) -> Result<ExtractResult> {
    let mut post: Vec<(String, String)> = vec![
        ("doc_id".into(), "3731964053542869".into()),
        (
            "variables".into(),
            format!(
                "{{\"livingRoomID\": {}}}",
                serde_json::to_string(video_id).unwrap_or_default()
            ),
        ),
    ];
    if let Some(prefetched) = relay_prefetched_data(webpage, &RELAY_LOGIN_DATA, None, video_id)
        && let Some(lsd) = prefetched
            .get("login_data")
            .and_then(|login| login.get("lsd"))
            .and_then(Value::as_object)
    {
        let (Some(name), Some(value)) = (lsd.get("name"), lsd.get("value")) else {
            return Err(bug(video_id, "KeyError('name')"));
        };
        let (name, value) = (py_str(name), py_str(value));
        match post.iter_mut().find(|(key, _)| *key == name) {
            Some(existing) => existing.1 = value,
            None => post.push((name, value)),
        }
    }
    // `extract_relay_data`: the first `RelayAPIConfigDefaults` definition.
    let mut config = json!({"graphURI": "/api/graphql/"});
    if let Some(relay) = all_relay_data(webpage, &RELAY_API_CONFIG, video_id)
        .into_iter()
        .find(|relay| truthy(Some(relay)))
    {
        for define in relay
            .get("define")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if define.get(0).and_then(Value::as_str) == Some("RelayAPIConfigDefaults") {
                config = define.get(2).cloned().unwrap_or(Value::Null);
            }
        }
    }
    let Some(graph_uri) = config.get("graphURI").and_then(Value::as_str) else {
        return Err(bug(video_id, "KeyError('graphURI')"));
    };
    let endpoint = real_url.join(graph_uri)?;
    crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading JSON metadata"));
    let text = post_form(
        client,
        &endpoint,
        video_id,
        "JSON metadata",
        urlencode(&post),
    )?;
    let response: Value = serde_json::from_str(&text).map_err(|error| {
        fail(
            video_id,
            &format!("Failed to parse JSON  (caused by {error})"),
        )
    })?;
    let Some(living_room) = response.pointer("/data/living_room") else {
        return Err(bug(video_id, "KeyError('living_room')"));
    };
    let mut entries = Vec::new();
    for edge in living_room
        .pointer("/recap/watched_content/edges")
        .map(children)
        .into_iter()
        .flatten()
    {
        let video = edge.pointer("/node/video");
        let Some(v_id) = video
            .and_then(|video| video.get("id"))
            .filter(|id| truthy(Some(id)))
        else {
            continue;
        };
        let v_id = py_str(v_id);
        let title = video
            .and_then(|video| video.get("name"))
            .filter(|name| !name.is_null())
            .map(py_str);
        let target = Url::parse(&format!("{VIDEO_PAGE_TEMPLATE}{v_id}"))?;
        entries.push(url_entry(&target, Some(v_id), title));
    }
    Ok(id_playlist(url, video_id, entries))
}

/// The server JS/pagelet/tahoe `videoData` of one video: its `src` and
/// DASH formats, English subtitles and the webpage metadata.
fn video_data_result(
    url: &Url,
    video_id: &str,
    webpage: &str,
    video_data: &Value,
) -> Result<ExtractResult> {
    let Value::Array(items) = video_data else {
        return Err(bug(
            video_id,
            "TypeError('string indices must be integers')",
        ));
    };
    let mut formats = Vec::new();
    let mut subtitles = Vec::new();
    for f in items {
        let Some(stream_type) = f.as_object().and_then(|f| f.get("stream_type")) else {
            return Err(bug(video_id, "KeyError('stream_type')"));
        };
        let format_id = py_str(stream_type);
        for quality in ["sd", "hd"] {
            for src_type in ["src", "src_no_ratelimit"] {
                let Some(src) = f
                    .get(format!("{quality}_{src_type}"))
                    .filter(|src| truthy(Some(src)))
                else {
                    continue;
                };
                // sd, hd formats w/o resolution info are deprioritized below DASH.
                let mut preference = if format_id == "progressive" { -10 } else { -3 };
                if quality == "hd" {
                    preference += 1;
                }
                formats.push(dict_format(
                    vec![
                        (
                            "format_id",
                            format!("{format_id}_{quality}_{src_type}").into(),
                        ),
                        ("url", src.clone()),
                        ("quality", preference.into()),
                        (
                            "height",
                            if quality == "hd" {
                                720.into()
                            } else {
                                Value::Null
                            },
                        ),
                    ],
                    video_id,
                )?);
            }
        }
        dash_manifest(f, &mut formats, None, video_id);
        if let Some(src) = f.get("subtitles_src").filter(|src| truthy(Some(src))) {
            let ext = subtitle_ext(src.as_str().unwrap_or_default());
            subtitles.push(json!({"url": src, "ext": ext}));
        }
    }
    process_formats(&mut formats);
    let mut dict = Map::new();
    dict.insert("id".into(), video_id.into());
    dict.insert("formats".into(), json!([]));
    let mut tracks = Map::new();
    if !subtitles.is_empty() {
        tracks.insert("en".into(), Value::Array(subtitles));
    }
    dict.insert("subtitles".into(), Value::Object(tracks));
    // `info_dict.update(extract_metadata(webpage))`.
    dict.extend(metadata(webpage, video_id));
    Ok(ExtractResult::Video(video_as(
        url,
        dict,
        formats,
        (NAME, KEY),
        "url",
    )))
}

/// The relay prefetched data's videos (`parse_graphql_video`), merged with
/// the webpage metadata.
fn relay_result(
    client: &Client,
    url: &Url,
    video_id: &str,
    webpage: &str,
    data: Map<String, Value>,
) -> Result<ExtractResult> {
    let data = Value::Object(data);
    let mut entries: Vec<Entry> = Vec::new();
    let parse_attachment = |attachment: &Value, key: &str, entries: &mut Vec<_>| -> Result<()> {
        let media = attachment.get(key).cloned().unwrap_or_else(|| json!({}));
        if media.get("__typename").and_then(Value::as_str) == Some("Video")
            && let Some(entry) = graphql_video(client, &media, video_id, webpage)?
        {
            entries.push(entry);
        }
        Ok(())
    };
    let nodes: Vec<Value> = match data
        .get("nodes")
        .filter(|v| truthy(Some(v)))
        .or_else(|| data.get("node"))
    {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![other.clone()],
    };
    let mut attachments = Vec::new();
    for node in &nodes {
        let Some(story) = node.pointer("/comet_sections/content/story") else {
            continue;
        };
        for source in [Some(story), story.get("attached_story")]
            .into_iter()
            .flatten()
        {
            for attachment in source
                .get("attachments")
                .map(children)
                .into_iter()
                .flatten()
            {
                for style in [
                    attachment.get("styles"),
                    attachment.get("style_type_renderer"),
                    attachment.pointer("/throwbackStyles/attachment_target_renderer"),
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(found) = style.get("attachment").filter(|value| value.is_object()) {
                        attachments.push(found.clone());
                    }
                }
            }
        }
    }
    for attachment in &attachments {
        let mut subs: Vec<Value> = attachment
            .pointer("/all_subattachments/nodes")
            .map(children)
            .into_iter()
            .flatten()
            .filter(|node| node.is_object())
            .cloned()
            .collect();
        if subs.is_empty() {
            subs = attachment
                .pointer("/target/attachments")
                .map(children)
                .into_iter()
                .flatten()
                .filter_map(|item| item.pointer("/styles/attachment"))
                .filter(|value| value.is_object())
                .cloned()
                .collect();
        }
        for sub in &subs {
            parse_attachment(sub, "media", &mut entries)?;
        }
        parse_attachment(attachment, "media", &mut entries)?;
    }
    for edge in data
        .pointer("/mediaset/currMedia/edges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        parse_attachment(edge, "node", &mut entries)?;
    }
    let cover = data
        .pointer("/event/cover_media_renderer/cover_video")
        .filter(|value| value.is_object())
        .or_else(|| data.get("video").filter(|value| value.is_object()))
        .cloned();
    if let Some(video_data) = cover {
        let story_attachments: Vec<Value> = video_data
            .pointer("/story/attachments")
            .or_else(|| video_data.pointer("/creation_story/attachments"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for attachment in &story_attachments {
            parse_attachment(attachment, "media", &mut entries)?;
        }
        if entries.is_empty()
            && let Some(entry) = graphql_video(client, &video_data, video_id, webpage)?
        {
            entries.push(entry);
        }
    }
    if entries.len() > 1 {
        let entries = entries
            .into_iter()
            .map(|(dict, formats)| video(url, dict, formats))
            .collect();
        return Ok(id_playlist(url, video_id, entries));
    }
    let (video_info, formats) = entries.pop().unwrap_or_else(|| {
        let mut info = Map::new();
        info.insert("id".into(), video_id.into());
        (info, Vec::new())
    });
    let mut webpage_info = metadata(webpage, video_id);
    if let Some(duration) = video_info
        .get("duration")
        .filter(|value| truthy(Some(value)))
    {
        webpage_info.insert("duration".into(), duration.clone());
    }
    if let Some(thumbnail) = video_info
        .get("thumbnail")
        .filter(|value| truthy(Some(value)))
    {
        webpage_info.insert("thumbnail".into(), thumbnail.clone());
    }
    let dict = merge_dicts(webpage_info, video_info);
    Ok(ExtractResult::Video(video(url, dict, formats)))
}

impl Extractor for Facebook {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(NAME, url);
        let real_url = if url.as_str().starts_with("facebook:") {
            Url::parse(&format!("{VIDEO_PAGE_TEMPLATE}{video_id}"))?
        } else {
            url.clone()
        };
        let page = Url::parse(
            &real_url
                .as_str()
                .replace("://m.facebook.com/", "://www.facebook.com/"),
        )?;
        let webpage = super::download_webpage(client, &page, NAME, &video_id)?;
        let mut video_data = server_js_video_data(&webpage, &video_id);
        if video_data.is_empty() {
            video_data = pagelet_video_data(&webpage, &video_id);
        }
        if video_data.is_empty()
            && let Some(data) = relay_prefetched_data(
                &webpage,
                &RELAY_VIDEO,
                Some(&["video", "event", "nodes", "node", "mediaset"]),
                &video_id,
            )
        {
            return relay_result(client, url, &video_id, &webpage, data).map(Some);
        }
        if video_data.is_empty() {
            static INTERSTITIAL: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(r#"class="[^"]*uiInterstitialContent[^"]*"><div>(.*?)</div>"#).unwrap()
            });
            if let Some(found) = INTERSTITIAL.captures(&webpage) {
                return Err(fail(
                    &video_id,
                    &format!(
                        "The video is not available, Facebook said: \"{}\"",
                        &found[1]
                    ),
                ));
            }
            if [
                ">You must log in to continue",
                "id=\"login_form\"",
                "id=\"loginbutton\"",
            ]
            .iter()
            .any(|marker| webpage.contains(marker))
            {
                return Err(fail(
                    &video_id,
                    "This video is only available for registered users. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies",
                ));
            }
            if real_url.as_str().contains("/watchparty/") {
                return watch_party(client, url, &real_url, &video_id, &webpage).map(Some);
            }
            video_data = tahoe_video_data(client, &video_id, &webpage)?;
        }
        if video_data.is_empty() {
            return Err(fail(
                &video_id,
                &format!("Cannot parse data; {}", crate::BUG_REPORTS_MESSAGE),
            ));
        }
        if video_data.len() > 1 {
            let mut entries = Vec::new();
            for data in &video_data {
                let first = match data {
                    Value::Array(items) => items.first(),
                    _ => None,
                };
                let Some(video_url) = first
                    .and_then(|first| first.get("video_url"))
                    .filter(|video_url| truthy(Some(video_url)))
                else {
                    continue;
                };
                let target = real_url.join(&py_str(video_url))?;
                let id = str_or_none(first.and_then(|first| first.get("video_id")));
                entries.push(url_entry(&target, id, None));
            }
            return Ok(Some(id_playlist(url, &video_id, entries)));
        }
        video_data_result(url, &video_id, &webpage, &video_data[0]).map(Some)
    }
}

impl Extractor for FacebookReel {
    fn key(&self) -> &'static str {
        "facebook:reel"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = REEL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        announce("facebook:reel", url);
        follow(&format!(
            "https://m.facebook.com/watch/?v={}&_rdr",
            &found["id"]
        ))
        .map(Some)
    }
}

impl Extractor for FacebookPluginsVideo {
    fn key(&self) -> &'static str {
        "FacebookPluginsVideo"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLUGINS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        announce("FacebookPluginsVideo", url);
        // `urllib.parse.unquote(self._match_id(url))`.
        let target = percent_encoding::percent_decode_str(&found["id"])
            .decode_utf8_lossy()
            .into_owned();
        follow(&target).map(Some)
    }
}

impl Extractor for FacebookRedirectUrl {
    fn key(&self) -> &'static str {
        REDIRECT_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !REDIRECT_URL.is_match(url.as_str()) {
            return Ok(None);
        }
        announce(REDIRECT_NAME, url);
        // `url_or_none(parse_qs(url).get('u', [None])[-1])`.
        let target = url
            .query_pairs()
            .filter(|(key, value)| key == "u" && !value.is_empty())
            .map(|(_, value)| value.into_owned())
            .last()
            .and_then(|target| super::json_ld::url_or_none(&target));
        match target {
            Some(target) => follow(&target).map(Some),
            None => Err(Error::Reported(format!(
                "[{REDIRECT_NAME}] Invalid facebook redirect URL"
            ))),
        }
    }
}

/// `FacebookAdsIE._FORMATS_MAP`: key, format id and note.
const AD_FORMATS: [(&str, &str, Option<&str>); 4] = [
    (
        "watermarked_video_sd_url",
        "sd-wmk",
        Some("SD, watermarked"),
    ),
    ("video_sd_url", "sd", None),
    (
        "watermarked_video_hd_url",
        "hd-wmk",
        Some("HD, watermarked"),
    ),
    ("video_hd_url", "hd", None),
];

/// The ad's `snapshot` (current and old page layouts).
fn ad_snapshot(post_data: &[Value]) -> Option<&Map<String, Value>> {
    for doc in post_data {
        for a in doc.get("require").map(children).into_iter().flatten() {
            for b in children(a) {
                for c in children(b) {
                    let Some(inner) = c.pointer("/__bbox/require") else {
                        continue;
                    };
                    for d in children(inner) {
                        for e in children(d) {
                            for f in children(e) {
                                for path in [
                                    "/__bbox/result/data/ad_library_main/deeplink_ad_archive_result/deeplink_ad_archive/snapshot",
                                    "/entryPointRoot/otherProps/deeplinkAdCard/snapshot",
                                ] {
                                    if let Some(snapshot) =
                                        f.pointer(path).and_then(Value::as_object)
                                    {
                                        return Some(snapshot);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// The ad body's markup text (`__m` reference), unless a template.
fn ad_markup(post_data: &[Value], markup_id: Option<&str>) -> Option<String> {
    let markup_id = markup_id?;
    for doc in post_data {
        for a in doc.get("require").map(children).into_iter().flatten() {
            for b in children(a) {
                for c in children(b) {
                    for item in c
                        .pointer("/__bbox/markup")
                        .map(children)
                        .into_iter()
                        .flatten()
                    {
                        if !item
                            .get(0)
                            .and_then(Value::as_str)
                            .is_some_and(|id| id.starts_with(markup_id))
                        {
                            continue;
                        }
                        for part in children(item) {
                            if let Some(html) = part.get("__html").and_then(Value::as_str) {
                                let text = crate::html::clean_html(html);
                                if !text.is_empty() && !text.starts_with("{{product.") {
                                    return Some(text);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// `FacebookAdsIE._extract_formats(video_dict)`.
fn ad_formats(entry: &Map<String, Value>, id: &str) -> Result<Vec<crate::VideoFormat>> {
    let mut formats = Vec::new();
    for (key, value) in entry {
        let Some(quality) = AD_FORMATS.iter().position(|(name, ..)| name == key) else {
            continue;
        };
        if value
            .as_str()
            .and_then(super::json_ld::url_or_none)
            .is_none()
        {
            continue;
        }
        let (_, format_id, note) = AD_FORMATS[quality];
        formats.push(dict_format(
            vec![
                ("format_id", format_id.into()),
                ("format_note", note.map_or(Value::Null, Value::from)),
                ("url", value.clone()),
                ("ext", "mp4".into()),
                ("quality", quality.into()),
            ],
            id,
        )?);
    }
    Ok(formats)
}

impl Extractor for FacebookAds {
    fn key(&self) -> &'static str {
        ADS_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ADS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(ADS_NAME, url);
        let ads_fail = |message: &str| {
            Error::Reported(format!(
                "[{ADS_NAME}] {video_id}: {message}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let webpage = super::download_webpage(client, url, ADS_NAME, &video_id)?;
        let post_data: Vec<Value> = SERVER_JS
            .captures_iter(&webpage)
            .filter_map(|found| serde_json::from_str(&found[1]).ok())
            .collect();
        let Some(data) = ad_snapshot(&post_data) else {
            return Err(ads_fail("Unable to extract ad data"));
        };
        let title = match data.get("title").and_then(Value::as_str) {
            Some(title) if !title.is_empty() && title != "{{product.name}}" => title.to_owned(),
            // `join_nonempty('display_format', 'page_name', delim=' by ')`.
            _ => ["display_format", "page_name"]
                .iter()
                .filter_map(|key| data.get(*key).filter(|value| truthy(Some(value))))
                .map(py_str)
                .collect::<Vec<_>>()
                .join(" by "),
        };
        let markup = ad_markup(
            &post_data,
            data.get("body")
                .and_then(|body| body.get("__m"))
                .and_then(Value::as_str),
        );
        let not_template = |value: Option<&Value>| {
            value
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty() && !text.starts_with("{{product."))
                .map(str::to_owned)
        };
        let mut first = Map::new();
        first.insert("title".into(), title.clone().into());
        first.insert(
            "description".into(),
            markup.map_or(Value::Null, Value::from),
        );
        let mut second = Map::new();
        if let Some(description) = not_template(data.get("body").and_then(|body| body.get("text")))
            .or_else(|| not_template(data.get("link_description")))
        {
            second.insert("description".into(), description.into());
        }
        if let Some(uploader) = data.get("page_name").filter(|name| name.is_string()) {
            second.insert("uploader".into(), uploader.clone());
        }
        if let Some(uploader_id) = str_or_none(data.get("page_id")) {
            second.insert("uploader_id".into(), uploader_id.into());
        }
        if let Some(uploader_url) = data
            .get("page_profile_uri")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            second.insert("uploader_url".into(), uploader_url.into());
        }
        if let Some(likes) = int(data.get("page_like_count")) {
            second.insert("like_count".into(), likes.into());
        }
        let mut info = merge_dicts(first, second);
        let mut entries: Vec<(Map<String, Value>, Vec<crate::VideoFormat>)> = Vec::new();
        for key in ["videos", "cards"] {
            for entry in data.get(key).map(children).into_iter().flatten() {
                let Some(entry) = entry.as_object() else {
                    continue;
                };
                let playable = AD_FORMATS.iter().any(|(name, ..)| {
                    entry
                        .get(*name)
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                        .is_some()
                });
                if !playable {
                    continue;
                }
                let mut dict = Map::new();
                dict.insert(
                    "id".into(),
                    format!("{video_id}_{}", entries.len() + 1).into(),
                );
                dict.insert(
                    "title".into(),
                    entry
                        .get("title")
                        .filter(|title| truthy(Some(title)))
                        .cloned()
                        .unwrap_or_else(|| title.clone().into()),
                );
                // `traverse_obj(entry, 'body', 'link_description', expected_type=str)`.
                let description = ["body", "link_description"]
                    .iter()
                    .find_map(|key| entry.get(*key).and_then(Value::as_str))
                    .filter(|text| !text.is_empty())
                    .map(Value::from)
                    .or_else(|| info.get("description").cloned())
                    .unwrap_or(Value::Null);
                dict.insert("description".into(), description);
                dict.insert(
                    "thumbnail".into(),
                    entry
                        .get("video_preview_image_url")
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                        .map_or(Value::Null, Value::from),
                );
                dict.insert("formats".into(), json!([]));
                entries.push((dict, ad_formats(entry, &video_id)?));
            }
        }
        if entries.len() > 1 {
            let first_title = entries[0].0.get("title").cloned().unwrap_or(Value::Null);
            info.insert("title".into(), first_title.clone());
            let keys: Vec<String> = info.keys().cloned().collect();
            let entries = entries
                .into_iter()
                .map(|(dict, formats)| video_as(url, dict, formats, (ADS_NAME, ADS_KEY), "url"))
                .collect();
            let mut extra = info;
            extra.shift_remove("title");
            extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                format!("playlist-dict;{},entries;after=id", keys.join(",")).into(),
            );
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id: video_id,
                title: first_title.as_str().unwrap_or_default().to_owned(),
                webpage_url: url.clone(),
                extractor: ADS_NAME.into(),
                extractor_key: ADS_KEY.into(),
                nested: Vec::new(),
                pending: None,
                entries,
                extra,
            })));
        }
        let mut formats = Vec::new();
        if let Some((entry, entry_formats)) = entries.pop() {
            // `info_dict.update(entries[0])`.
            info.extend(entry);
            formats = entry_formats;
        }
        info.insert("id".into(), video_id.into());
        Ok(Some(ExtractResult::Video(video_as(
            url,
            info,
            formats,
            (ADS_NAME, ADS_KEY),
            "url",
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.facebook.com/radiokicksfm/videos/3676516585958356/"
        ));
        assert!(suitable(
            "https://www.facebook.com/watch/?v=647537299265662"
        ));
        assert!(suitable(
            "https://m.facebook.com/story.php?story_fbid=pfbid0abc&id=1"
        ));
        assert!(suitable("facebook:544765982287235"));
        assert!(!suitable("https://www.facebook.com/radiokicksfm/"));
        assert!(reel_suitable(
            "https://www.facebook.com/reel/1195289147628387"
        ));
        assert!(plugins_suitable(
            "https://www.facebook.com/plugins/video.php?href=https%3A%2F%2Fwww.facebook.com%2Fx%2Fvideos%2F1%2F"
        ));
        assert!(redirect_suitable(
            "https://www.facebook.com/flx/warn/?h=TAQHsoToz&u=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3DpO8h3EaFRdo&s=1"
        ));
        assert!(!redirect_suitable("https://www.facebook.com/flx/warning"));
        assert!(ads_suitable(
            "https://www.facebook.com/ads/library/?id=899206155126718"
        ));
        assert!(ads_suitable(
            "https://es-la.facebook.com/ads/library/?active_status=all&id=901230958115569"
        ));
        assert!(!ads_suitable("https://www.facebook.com/ads/library/?q=1"));
    }

    #[test]
    fn post_data_is_encoded_like_urlencode() {
        let pairs = [
            ("variables".to_owned(), "{\"a\": \"b~c*\"}".to_owned()),
            ("x y".to_owned(), "_.-".to_owned()),
        ];
        assert_eq!(
            urlencode(&pairs),
            "variables=%7B%22a%22%3A+%22b~c%2A%22%7D&x+y=_.-"
        );
    }

    #[test]
    fn pagelet_patterns_find_video_configs() {
        let page = r#"<script>bigPipe.onPageletArrive({id:"pagelet_group_mall",jsmods:{instances:[["a",["VideoConfig"],[{video_id:"1",videoData:[{stream_type:"other"}]}],1]]}});}),"onPageletArrive pagelet_group_mall"</script>"#;
        assert_eq!(
            pagelet_video_data(page, "1"),
            vec![json!([{"stream_type": "other"}])]
        );
    }

    #[test]
    fn merge_dicts_keeps_first_values_and_fills_empty_strings() {
        let first = json!({"a": "", "b": 1, "c": null})
            .as_object()
            .unwrap()
            .clone();
        let second = json!({"a": "x", "b": 2, "c": 3, "d": 4})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            Value::Object(merge_dicts(first, second)),
            json!({"a": "x", "b": 1, "c": 3, "d": 4})
        );
    }
}
