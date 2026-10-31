//! Upstream `YappyIE` and `YappyProfileIE` (`yt_dlp/extractor/yappy.py`):
//! Yappy videos from the page's JSON-LD and Next.js data (else the video
//! API): the video (and its unwatermarked copy) and audio links with the
//! creator, counts and categories; profiles as a paged list of video
//! `url` results.
use super::{
    Extractor, dict_format, dict_info, download_json, download_json_note, download_webpage,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Yappy;
pub struct YappyProfile;

const NAME: &str = "Yappy";
const PROFILE_NAME: &str = "YappyProfile";
const PAGE_SIZE: usize = 15;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://yappy\.media/video/(?P<id>\w+)").unwrap()
});
static PROFILE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://yappy\.media/profile/(?P<id>\w+)").unwrap());

/// `YappyIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `YappyProfileIE._match_id`.
pub fn profile_suitable(url: &str) -> bool {
    PROFILE_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| super::nuxt::truthy(value))
}

fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        Some(Value::Bool(value)) => i64::from(*value).into(),
        _ => Value::Null,
    }
}

/// `_html_search_meta(names, webpage)` (non-fatal: warns with the first
/// name).
fn meta(webpage: &str, names: &[&str]) -> Option<String> {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name));
    if found.is_none() {
        crate::screen::warning(&format!(
            "[{NAME}] unable to extract {}; {}",
            names[0],
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    found
}

impl Extractor for Yappy {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        // `_WORKING = False`.
        super::broken_warning();
        start(NAME, url);
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `_search_json_ld(webpage, video_id)` (fatal, also without any
        // usable JSON-LD fields).
        let json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract JSON-LD; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let nextjs_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        let page_props = nextjs_data.pointer("/props/pageProps");
        let embedded = ["data", "OpenGraphParameters"]
            .iter()
            .find_map(|key| {
                page_props
                    .and_then(|props| props.get(*key))
                    .filter(|value| !value.is_null())
            })
            .filter(|value| super::nuxt::truthy(value))
            .cloned();
        let media_data = match embedded {
            Some(data) => data,
            None => {
                let api = Url::parse(&format!("https://yappy.media/api/video/{video_id}"))?;
                download_json(client, &api, NAME, &video_id)?
            }
        };
        let media_url = media_data
            .get("link")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .unwrap_or_default();
        let has_watermark = media_url.ends_with("-wm.mp4");
        let mut formats = Vec::new();
        if !media_url.is_empty() {
            formats.push(dict_format(
                vec![
                    ("url", Value::from(media_url.clone())),
                    ("ext", "mp4".into()),
                    (
                        "format_note",
                        if has_watermark {
                            "Watermarked".into()
                        } else {
                            Value::Null
                        },
                    ),
                    (
                        "preference",
                        if has_watermark {
                            (-10).into()
                        } else {
                            Value::Null
                        },
                    ),
                ],
                NAME,
                &video_id,
            )?);
        }
        if has_watermark {
            formats.push(dict_format(
                vec![
                    ("url", media_url.replace("-wm.mp4", ".mp4").into()),
                    ("ext", "mp4".into()),
                ],
                NAME,
                &video_id,
            )?);
        }
        if let Some(audio) = truthy(media_data.pointer("/audio/link")) {
            formats.push(dict_format(
                vec![
                    ("url", audio.clone()),
                    ("ext", "mp3".into()),
                    ("acodec", "mp3".into()),
                    ("vcodec", "none".into()),
                ],
                NAME,
                &video_id,
            )?);
        }
        let ld = |key: &str| truthy(json_ld.get(key)).cloned();
        let title = match ld("description") {
            Some(title) => title,
            None => match meta(&webpage, &["og:title"]).filter(|title| !title.is_empty()) {
                Some(title) => title.into(),
                None => {
                    let title = crate::html::html_extract_title(&webpage);
                    if title.is_none() {
                        crate::screen::warning(&format!(
                            "[{NAME}] unable to extract title; {}",
                            crate::BUG_REPORTS_MESSAGE
                        ));
                    }
                    title.map_or(Value::Null, Value::from)
                }
            },
        };
        let thumbnail = match truthy(media_data.get("thumbnail")) {
            Some(thumbnail) => thumbnail.clone(),
            None => meta(&webpage, &["og:image", "og:image:secure_url"])
                .map_or(Value::Null, Value::from),
        };
        let description = match truthy(media_data.get("description"))
            .cloned()
            .or_else(|| ld("description"))
        {
            Some(description) => description,
            None => {
                meta(&webpage, &["description", "og:description"]).map_or(Value::Null, Value::from)
            }
        };
        let timestamp = truthy(media_data.get("publishedAt"))
            .or_else(|| truthy(json_ld.get("timestamp")))
            .and_then(|value| match value {
                Value::String(date) => {
                    crate::dates::unified_timestamp(date, false).map(Value::from)
                }
                Value::Number(number) => Some(Value::Number(number.clone())),
                _ => None,
            })
            .unwrap_or(Value::Null);
        let view_count =
            int_or_none(truthy(media_data.get("viewsCount")).or_else(|| json_ld.get("view_count")));
        let categories: Vec<Value> = match media_data.get("categories") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.get("name").filter(|name| !name.is_null()).cloned())
                .collect(),
            _ => Vec::new(),
        };
        let creator = media_data.get("creator");
        let uploader_id = creator
            .and_then(|creator| {
                ["uuid", "nickname"]
                    .iter()
                    .find_map(|key| creator.get(*key).filter(|value| !value.is_null()))
            })
            .cloned()
            .unwrap_or(Value::Null);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("formats", Value::Null),
                ("thumbnail", thumbnail),
                ("description", description),
                ("timestamp", timestamp),
                ("view_count", view_count),
                ("like_count", int_or_none(media_data.get("likesCount"))),
                (
                    "uploader",
                    creator
                        .and_then(|creator| creator.get("firstName"))
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                ("uploader_id", uploader_id),
                (
                    "categories",
                    if categories.is_empty() {
                        Value::Null
                    } else {
                        Value::Array(categories)
                    },
                ),
                ("repost_count", int_or_none(media_data.get("sharingCount"))),
            ],
            formats,
        )))
    }
}

/// `url_result(f'https://yappy.media/video/{uuid}', YappyIE, uuid, description)`.
fn video_entry(uuid: &str, description: Option<&Value>) -> Result<VideoInfo> {
    let target = format!("https://yappy.media/video/{uuid}");
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = uuid.to_owned();
    let mut entry = Map::new();
    entry.insert("ie_key".into(), NAME.into());
    entry.insert("id".into(), uuid.into());
    if let Some(description) = description.filter(|value| !value.is_null()) {
        entry.insert("title".into(), description.clone());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.clone().into());
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

/// Whether a playlist's pending state is a Yappy profile's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == PROFILE_NAME
}

/// `OnDemandPagedList(fetch_page, 15)` read by upstream's `PlaylistEntries`.
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let profile_id = state["profile_id"].as_str().unwrap_or_default().to_owned();
    let mut fetch = |page: usize| -> Result<Vec<VideoInfo>> {
        let page = page + 1;
        let api = Url::parse(&format!(
            "https://yappy.media/api/video/list/{profile_id}?page={page}"
        ))?;
        let videos = download_json_note(
            client,
            &api,
            PROFILE_NAME,
            &profile_id,
            &format!("Downloading profile page {page} JSON"),
        )?;
        let mut entries = Vec::new();
        if let Some(Value::Array(results)) = videos.get("results") {
            for video in results {
                let Some(uuid) = truthy(video.get("uuid")) else {
                    continue;
                };
                let uuid = match uuid {
                    Value::String(uuid) => uuid.clone(),
                    Value::Number(number) => number.to_string(),
                    _ => continue,
                };
                entries.push(video_entry(&uuid, video.get("description"))?);
            }
        }
        Ok(entries)
    };
    let placeholder = || video_entry("None", None);
    super::paged::fetch_requested(state, limit, entries, PAGE_SIZE, &mut fetch, &placeholder)
}

impl Extractor for YappyProfile {
    fn key(&self) -> &'static str {
        PROFILE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PROFILE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let profile_id = found["id"].to_owned();
        start(PROFILE_NAME, url);
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;id{}", crate::info_order::GENERATOR).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: profile_id.clone(),
            title: String::new(),
            webpage_url: url.clone(),
            extractor: PROFILE_NAME.into(),
            extractor_key: PROFILE_NAME.into(),
            nested: Vec::new(),
            pending: Some(crate::PendingEntries {
                state: json!({
                    "kind": PROFILE_NAME,
                    "profile_id": profile_id,
                    "random_access": true,
                }),
            }),
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
            "https://yappy.media/video/47f3b4a1ad2e41b38a1d8f8e0c1bda88"
        ));
        assert!(profile_suitable(
            "https://yappy.media/profile/59a0c8c485e5410b9c43474bf4c6a373"
        ));
        assert!(!suitable("https://yappy.media/profile/x"));
    }
}
