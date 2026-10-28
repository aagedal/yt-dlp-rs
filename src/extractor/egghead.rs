//! Upstream `EggheadCourseIE` and `EggheadLessonIE`
//! (`yt_dlp/extractor/egghead.py`): egghead.io courses as playlists of
//! their lessons' `url` results (with the non-fatal course metadata), and
//! lessons from the lesson API (HLS, DASH and direct media URLs).
use super::{Extractor, dict_format, dict_info, download_json_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct EggheadCourse;
pub struct EggheadLesson;

const COURSE_NAME: &str = "egghead:course";
const COURSE_KEY: &str = "EggheadCourse";
const LESSON_NAME: &str = "egghead:lesson";
const LESSON_KEY: &str = "EggheadLesson";
const API: &str = "https://app.egghead.io/api/v1/";

static COURSE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:app\.)?egghead\.io/(?:course|playlist)s/(?P<id>[^/?#&]+)").unwrap()
});
static LESSON_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:app\.)?egghead\.io/(?:api/v1/)?lessons/(?P<id>[^/?#&]+)").unwrap()
});

/// `EggheadCourseIE._match_id`.
pub fn course_suitable(url: &str) -> bool {
    COURSE_URL.is_match(url)
}

/// `EggheadLessonIE._match_id`.
pub fn lesson_suitable(url: &str) -> bool {
    LESSON_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("egghead.io {what} are not supported natively"))
}

/// `_call_api(path, video_id, resource)`.
fn call_api(
    client: &Client,
    name: &'static str,
    path: &str,
    video_id: &str,
    resource: &str,
) -> Result<Value> {
    let api = Url::parse(&format!("{API}{path}"))?;
    download_json_note(
        client,
        &api,
        name,
        video_id,
        &format!("Downloading {resource} JSON"),
    )
}

/// Python `str(value)` of a JSON ID (`None` stays `None`).
fn py_str(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.into()),
        _ => None,
    }
}

fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

/// `url_result(lesson_url, ie='EggheadLesson', video_id=lesson_id)`.
fn lesson_entry(target: &str, lesson_id: Option<Value>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // Without an ID, a flat entry's ID is EggheadLessonIE's temporary ID.
    info.id = LESSON_URL
        .captures(target)
        .map(|found| found["id"].to_owned())
        .unwrap_or_default();
    let mut entry = Map::new();
    entry.insert("ie_key".into(), LESSON_KEY.into());
    if let Some(id) = lesson_id {
        // A falsy ID also gets the temporary one when flat.
        if super::nuxt::truthy(&id) {
            info.id = py_str(Some(&id)).unwrap_or_default();
        }
        entry.insert("id".into(), id);
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": LESSON_KEY,
            "extractor": LESSON_NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for EggheadCourse {
    fn key(&self) -> &'static str {
        COURSE_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = COURSE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        start(COURSE_NAME, url);
        let series_path = format!("series/{playlist_id}");
        let lessons = call_api(
            client,
            COURSE_NAME,
            &format!("{series_path}/lessons"),
            &playlist_id,
            "course lessons",
        )?;
        let mut entries = Vec::new();
        let lessons: Vec<&Value> = match &lessons {
            Value::Array(items) => items.iter().collect(),
            _ => return Err(unsupported("lesson lists that are not lists")),
        };
        for lesson in lessons {
            let Some(lesson_url) = lesson
                .get("http_url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            // `str(id)` when truthy; a falsy ID stays as it is.
            let lesson_id = match lesson.get("id") {
                None | Some(Value::Null) => None,
                Some(id) if super::nuxt::truthy(id) => py_str(Some(id)).map(Value::from),
                Some(id) => Some(id.clone()),
            };
            entries.push(lesson_entry(&lesson_url, lesson_id)?);
        }
        // `_call_api(series_path, playlist_id, 'course', False) or {}`.
        let course = match call_api(client, COURSE_NAME, &series_path, &playlist_id, "course") {
            Ok(course) => course,
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{COURSE_NAME}] {playlist_id}: Unable to download JSON metadata: {}",
                    crate::upstream_error_text(&source)
                ));
                Value::Null
            }
            Err(Error::Reported(message)) if message.contains(": Failed to parse JSON ") => {
                crate::screen::warning(&message);
                Value::Null
            }
            Err(error) => return Err(error),
        };
        let course_id = course
            .get("id")
            .filter(|id| super::nuxt::truthy(id))
            .and_then(|id| py_str(Some(id)));
        let title = match course.get("title") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => Some(title.clone()).filter(|title| !title.is_empty()),
            Some(_) => return Err(unsupported("course titles that are not strings")),
        };
        // `playlist_result(entries, id, title, description)`.
        let mut keys = Vec::new();
        let mut extra = Map::new();
        if course_id.is_some() {
            keys.push("id");
        } else {
            extra.insert("id".into(), Value::Null);
        }
        if title.is_some() {
            keys.push("title");
        }
        if let Some(description) = course.get("description").filter(|value| !value.is_null()) {
            keys.push("description");
            extra.insert("description".into(), description.clone());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: course_id.unwrap_or_default(),
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: COURSE_NAME.into(),
            extractor_key: COURSE_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

impl Extractor for EggheadLesson {
    fn key(&self) -> &'static str {
        LESSON_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = LESSON_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(LESSON_NAME, url);
        let lesson = call_api(
            client,
            LESSON_NAME,
            &format!("lessons/{display_id}"),
            &display_id,
            "lesson",
        )?;
        let lesson_id = match lesson.get("id") {
            None => return Err(key_error(&display_id, "id")),
            Some(Value::Null) => "None".to_owned(),
            Some(id) => {
                py_str(Some(id)).ok_or_else(|| unsupported("lesson IDs that are not scalars"))?
            }
        };
        let title = lesson
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&display_id, "title"))?;
        let media_urls = match lesson.get("media_urls") {
            Some(Value::Object(urls)) => urls.clone(),
            Some(_) => return Err(unsupported("media URL maps that are not objects")),
            None => return Err(key_error(&display_id, "media_urls")),
        };
        let mut formats = Vec::new();
        for format_url in media_urls.values() {
            let Some(format_url) = format_url.as_str().and_then(super::json_ld::url_or_none) else {
                continue;
            };
            let manifest = Url::parse(&format_url)?;
            match super::direct::determine_ext(&format_url).as_deref() {
                Some("m3u8") => {
                    if let Some(stream) = super::extract_m3u8_master_optional(
                        client,
                        &manifest,
                        LESSON_NAME,
                        &lesson_id,
                        Some("hls"),
                        false,
                        false,
                    ) {
                        formats.extend(stream.formats);
                    }
                }
                Some("mpd") => {
                    if let Some(mut stream) = super::extract_mpd_optional_note(
                        client,
                        &manifest,
                        LESSON_NAME,
                        &lesson_id,
                        Some("dash"),
                        "Downloading MPD manifest",
                    ) {
                        super::ignore_subtitles(&mut stream, LESSON_NAME, "DASH");
                        formats.extend(stream.formats);
                    }
                }
                _ => formats.push(dict_format(
                    vec![("url", format_url.into())],
                    LESSON_NAME,
                    &lesson_id,
                )?),
            }
        }
        let get = |key: &str| lesson.get(key).cloned().unwrap_or(Value::Null);
        let opt = |value: Option<i64>| value.map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (LESSON_NAME, LESSON_KEY),
            vec![
                ("id", lesson_id.into()),
                ("display_id", display_id.into()),
                ("title", title),
                ("description", get("summary")),
                ("thumbnail", get("thumb_nail")),
                (
                    "timestamp",
                    lesson
                        .get("published_at")
                        .and_then(Value::as_str)
                        .and_then(|date| crate::dates::unified_timestamp(date, false))
                        .map_or(Value::Null, Value::from),
                ),
                ("duration", opt(int_or_none(lesson.get("duration")))),
                ("view_count", opt(int_or_none(lesson.get("plays_count")))),
                (
                    "tags",
                    lesson
                        .get("tag_list")
                        .filter(|tags| tags.is_array())
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "series",
                    lesson
                        .pointer("/series/title")
                        .filter(|title| title.is_string())
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(course_suitable(
            "https://egghead.io/courses/professor-frisby-introduces-composable-functional-javascript"
        ));
        assert!(course_suitable("https://app.egghead.io/playlists/x"));
        assert!(lesson_suitable(
            "https://egghead.io/api/v1/lessons/react-add-redux-to-a-react-application"
        ));
        assert!(!lesson_suitable("https://egghead.io/courses/x"));
    }
}
