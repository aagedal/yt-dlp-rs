//! Upstream `ImdbIE` and `ImdbListIE` (`yt_dlp/extractor/imdb.py`): a
//! video page's Next.js `videoPlaybackData` (or the legacy
//! `VIDEO_PLAYBACK_DATA` API) with HLS and progressive encodings, and a
//! list page's video player links as a playlist.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Imdb;
pub struct ImdbList;

const NAME: &str = "Imdb";
const IE_NAME: &str = "imdb";
const LIST_NAME: &str = "ImdbList";
const LIST_IE_NAME: &str = "imdb:list";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www|m)\.imdb\.com/(?:video|title|list).*?[/-]vi(?P<id>\d+)").unwrap()
});
// Upstream's `(?!/videoplayer/vi\d+)` is checked in `list_id`.
static LIST_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?imdb\.com/list/ls(?P<id>\d{9})").unwrap());
static VIDEOPLAYER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/videoplayer/vi\d").unwrap());
static LIST_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<h1[^>]+class="[^"]*header[^"]*"[^>]*>(.*?)</h1>"#).unwrap()
});
static LIST_DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div[^>]+class="[^"]*list-description[^"]*"[^>]*><p>(.*?)</p>"#).unwrap()
});

const QUALITIES: [&str; 4] = ["SD", "480p", "720p", "1080p"];

/// `ImdbIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `ImdbListIE._match_valid_url`.
pub fn list_suitable(url: &str) -> bool {
    list_id(url).is_some()
}

fn list_id(url: &str) -> Option<String> {
    let found = LIST_URL.captures(url)?;
    let rest = &url[found.get(0)?.end()..];
    (!VIDEOPLAYER.is_match(rest)).then(|| found["id"].to_owned())
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("IMDb {what} are not supported natively"))
}

/// `traverse_obj(value, path)` over dict keys.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in keys {
        current = current.as_object()?.get(*key)?;
    }
    (!current.is_null()).then_some(current)
}

impl Extractor for Imdb {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(IE_NAME);
        crate::screen::screen(&format!(
            "[{IE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = Url::parse(&format!("https://www.imdb.com/video/vi{video_id}"))?;
        let webpage = download_webpage(client, &page, IE_NAME, &video_id)?;
        let info = super::search_nextjs_data(&webpage, IE_NAME, &video_id)?;
        let empty = Value::Object(Map::new());
        let video_info =
            path(&info, &["props", "pageProps", "videoPlaybackData", "video"]).unwrap_or(&empty);
        if !video_info.is_object() {
            return Err(unsupported("playback data that are not objects"));
        }
        // `_html_search_meta(('og:title', 'twitter:title'))` takes the first
        // `<meta>` naming either; this prefers `og:title`.
        let title = path(video_info, &["name", "value"])
            .or_else(|| path(video_info, &["primaryTitle", "titleText", "text"]))
            .filter(|title| super::nuxt::truthy(title))
            .cloned()
            .or_else(|| {
                crate::html::html_search_meta(&webpage, "og:title")
                    .or_else(|| crate::html::html_search_meta(&webpage, "twitter:title"))
                    .filter(|title| !title.is_empty())
                    .or_else(|| crate::html::html_extract_title(&webpage))
                    .map(Value::from)
            })
            .unwrap_or(Value::Null);
        let data = match video_info
            .get("playbackURLs")
            .filter(|data| super::nuxt::truthy(data))
        {
            Some(data) => data.clone(),
            None => {
                let key = base64::engine::general_purpose::STANDARD.encode(
                    crate::output::python_json_dumps(&json!({
                        "type": "VIDEO_PLAYER",
                        "subType": "FORCE_LEGACY",
                        "id": format!("vi{video_id}"),
                    })),
                );
                let api = super::update_url_query(
                    &Url::parse("https://www.imdb.com/ve/data/VIDEO_PLAYBACK_DATA")?,
                    &[("key", &key)],
                );
                let legacy = download_json(client, &api, IE_NAME, &video_id)?;
                // `try_get(..., lambda x: x[0]['videoLegacyEncodings'])`.
                legacy
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|item| item.as_object())
                    .and_then(|item| item.get("videoLegacyEncodings"))
                    .cloned()
                    .unwrap_or(Value::Null)
            }
        };
        let encodings = match &data {
            Value::Array(encodings) => encodings,
            // Iterating `None`: upstream's unwrapped `TypeError`.
            Value::Null => {
                return Err(Error::Reported("'NoneType' object is not iterable".into()));
            }
            _ => return Err(unsupported("playback URL lists that are not arrays")),
        };
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        for encoding in encodings {
            let Some(encoding) = encoding.as_object().filter(|encoding| !encoding.is_empty())
            else {
                continue;
            };
            let Some(video_url) = encoding
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let ext = encoding
                .get("mimeType")
                .and_then(Value::as_str)
                .map(|mime| super::direct::mimetype2ext_or(mime, None))
                .filter(|ext| !ext.is_empty())
                .or_else(|| super::direct::determine_ext(&video_url));
            if ext.as_deref() == Some("m3u8") {
                let manifest = Url::parse(&video_url)?;
                if let Some(mut stream) = super::extract_m3u8_optional(
                    client,
                    &manifest,
                    IE_NAME,
                    &video_id,
                    Some("hls"),
                    true,
                ) {
                    for format in &mut stream.formats {
                        format.extra.insert("preference".into(), 1.into());
                    }
                    formats.append(&mut stream.formats);
                    super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                }
                continue;
            }
            let format_id = path(&Value::Object(encoding.clone()), &["displayName", "value"])
                .or_else(|| encoding.get("definition").filter(|value| !value.is_null()))
                .cloned()
                .unwrap_or(Value::Null);
            let quality = format_id
                .as_str()
                .and_then(|id| QUALITIES.iter().position(|known| *known == id))
                .map_or(-1, |index| index as i64);
            formats.push(dict_format(
                vec![
                    ("format_id", format_id),
                    ("url", video_url.into()),
                    ("ext", ext.map_or(Value::Null, Value::from)),
                    ("quality", quality.into()),
                ],
                IE_NAME,
                &video_id,
            )?);
        }
        let try_value = |keys: &[&str]| path(video_info, keys).cloned().unwrap_or(Value::Null);
        let thumbnail = path(video_info, &["thumbnail", "url"])
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map_or(Value::Null, Value::from);
        let mut result = dict_info(
            url,
            (IE_NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", title),
                (
                    "alt_title",
                    info.get("videoSubTitle").cloned().unwrap_or(Value::Null),
                ),
                ("formats", Value::Null),
                ("description", try_value(&["description", "value"])),
                ("thumbnail", thumbnail),
                (
                    "duration",
                    super::streamable::int_or_none(path(video_info, &["runtime", "value"])),
                ),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        result.subtitles = subtitles;
        Ok(Some(result))
    }
}

impl Extractor for ImdbList {
    fn key(&self) -> &'static str {
        LIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(id) = list_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(LIST_IE_NAME);
        crate::screen::screen(&format!(
            "[{LIST_IE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, LIST_IE_NAME, &id)?;
        let links = Regex::new(&format!(r#"href="(/list/ls{id}/videoplayer/vi[^"]+)""#))
            .expect("digits-only list ids");
        let mut entries = Vec::new();
        for found in links.captures_iter(&webpage) {
            let target = format!("http://www.imdb.com{}", &found[1]);
            let Ok(entry_url) = Url::parse(&target) else {
                return Err(unsupported("list links that are not valid URLs"));
            };
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
                    "extractor": IE_NAME,
                    "entry": {"ie_key": NAME, "_type": "url", "url": target},
                }),
            );
            entries.push(entry);
        }
        let search = |regex: &Regex, name: &str| -> Result<String> {
            regex
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{LIST_IE_NAME}] {id}: Unable to extract {name}; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })
        };
        let title = search(&LIST_TITLE, "list title")?;
        let description = search(&LIST_DESCRIPTION, "list description")?;
        let mut extra = Map::new();
        let keys = if title.is_empty() {
            "id,description"
        } else {
            "id,title,description"
        };
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        extra.insert("description".into(), description.into());
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id,
            title,
            webpage_url: url.clone(),
            extractor: LIST_IE_NAME.into(),
            extractor_key: LIST_NAME.into(),
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
        assert!(suitable("https://www.imdb.com/video/imdb/vi2524815897"));
        assert!(suitable(
            "https://m.imdb.com/title/tt0000001/videoplayer/vi2524815897"
        ));
        assert!(suitable(
            "https://www.imdb.com/list/ls009921623/videoplayer/vi260482329"
        ));
        assert!(!suitable("https://www.imdb.com/list/ls009921623/"));
        assert!(list_suitable("https://www.imdb.com/list/ls009921623/"));
        assert!(!list_suitable(
            "https://www.imdb.com/list/ls009921623/videoplayer/vi260482329"
        ));
    }
}
