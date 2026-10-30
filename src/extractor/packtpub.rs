//! Upstream `PacktPubIE` and `PacktPubCourseIE` (`yt_dlp/extractor/packtpub.py`):
//! a video's CDN MP4 URL from the products API as a url-only result, and a
//! course's table of contents as `url_transparent` video entries. Logging in
//! is not supported natively, so a locked video reports the login requirement.
use super::{Extractor, download_json, login};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub struct PacktPub;
pub struct PacktPubCourse;

const NAME: &str = "PacktPub";
const COURSE_NAME: &str = "PacktPubCourse";
const STATIC_PRODUCTS_BASE: &str = "https://static.packt-cdn.com/products/";
const PRODUCTS_API: &str = "https://services.packtpub.com/products-v1/products/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www\.)?packtpub\.com/mapt|subscription\.packtpub\.com)/video/[^/]+/(?P<course_id>\d+)/(?P<chapter_id>[^/]+)/(?P<id>[^/]+)(?:/(?P<display_id>[^/?&#]+))?",
    )
    .unwrap()
});
static COURSE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<url>https?://(?:(?:www\.)?packtpub\.com/mapt|subscription\.packtpub\.com)/video/[^/]+/(?P<id>\d+))",
    )
    .unwrap()
});

/// `PacktPubIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `PacktPubCourseIE.suitable`: the course pattern, unless a video matches.
pub fn course_suitable(url: &str) -> bool {
    !VALID_URL.is_match(url) && COURSE_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("PacktPub {what} are not supported natively"))
}

/// `_download_json(url, video_id, 'Downloading JSON video')` where a 400
/// status is `raise_login_required('This video is locked')`.
fn video_data(client: &Client, url: &Url, id: &str) -> Result<Value> {
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON video"));
    let mut response = super::direct::request_expecting(client, url, &[], &[400])?;
    if response.status().as_u16() == 400 {
        return Err(Error::Reported(format!(
            "[{NAME}] {id}: {}",
            login::login_required_message("This video is locked", Some("packtpub"), None)
        )));
    }
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// A string `str` value that Python's `strip_or_none` keeps.
fn strip_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(|text| text.trim().to_owned())
}

/// `str_or_none` of a chapter or section ID: `None` for a null value.
fn id_text(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(other) => Some(other.to_string()),
    }
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for PacktPub {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let course_id = found["course_id"].to_owned();
        let chapter_id = found["chapter_id"].to_owned();
        let video_id = found["id"].to_owned();
        let display_id = found
            .name("display_id")
            .map(|found| found.as_str().to_owned());
        announce(NAME, url);
        let api = Url::parse(&format!(
            "{PRODUCTS_API}{course_id}/{chapter_id}/{video_id}"
        ))?;
        let data = video_data(client, &api, &video_id)?;
        let video_url = data.get("data").and_then(Value::as_str).ok_or_else(|| {
            Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('data')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let media =
            Url::parse(video_url).map_err(|_| unsupported("video URLs that are not URLs"))?;
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp4".into()),
            "",
        );
        info.id = video_id.clone();
        // `display_id or video_id`, undecoded, as upstream's title.
        info.title = display_id.unwrap_or(video_id);
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        Ok(Some(crate::info_order::mark_info(
            info,
            "untyped-dict;id,url,title",
        )))
    }
}

/// `url_result`-like `url_transparent` entry for one course video: the
/// section's fields overlaid on the video's own extraction.
fn course_entry(target: &str, fields: Vec<(&str, Value)>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut raw = Map::new();
    let mut overlay = Map::new();
    raw.insert("_type".into(), "url_transparent".into());
    raw.insert("url".into(), target.into());
    let mut ie_key_inserted = false;
    for (key, value) in fields {
        // Upstream's entry: the video fields, then `ie_key`, then the chapter.
        if key == "chapter" && !ie_key_inserted {
            raw.insert("ie_key".into(), NAME.into());
            ie_key_inserted = true;
        }
        if key == "title"
            && let Some(title) = value.as_str()
        {
            info.title = title.to_owned();
        }
        if !value.is_null() {
            overlay.insert(key.into(), value.clone());
        }
        raw.insert(key.into(), value);
    }
    if !ie_key_inserted {
        raw.insert("ie_key".into(), NAME.into());
    }
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

impl Extractor for PacktPubCourse {
    fn key(&self) -> &'static str {
        "PacktPubCourse"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = COURSE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        if !course_suitable(url.as_str()) {
            return Ok(None);
        }
        let base = found["url"].to_owned();
        let course_id = found["id"].to_owned();
        announce(COURSE_NAME, url);
        let course = download_json(
            client,
            &Url::parse(&format!("{STATIC_PRODUCTS_BASE}{course_id}/toc"))?,
            COURSE_NAME,
            &course_id,
        )?;
        // `_download_json(..., fatal=False) or {}`.
        let metadata = match download_json(
            client,
            &Url::parse(&format!("{STATIC_PRODUCTS_BASE}{course_id}/summary"))?,
            COURSE_NAME,
            &course_id,
        ) {
            Ok(metadata) => metadata,
            Err(error) => {
                crate::screen::warning(&super::request_error_text(
                    COURSE_NAME,
                    &course_id,
                    "JSON metadata",
                    &error,
                ));
                json!({})
            }
        };

        let mut entries = Vec::new();
        if let Some(chapters) = course.get("chapters").and_then(Value::as_array) {
            for (index, chapter) in chapters.iter().enumerate() {
                let Some(chapter_id) = id_text(chapter.get("id")) else {
                    continue;
                };
                let Some(sections) = chapter.get("sections").and_then(Value::as_array) else {
                    continue;
                };
                let chapter_title = chapter.get("title").cloned().unwrap_or(Value::Null);
                for section in sections {
                    let Some(section_id) = id_text(section.get("id")) else {
                        continue;
                    };
                    if section.get("contentType").and_then(Value::as_str) != Some("video") {
                        continue;
                    }
                    let target = format!("{base}/{chapter_id}/{section_id}");
                    let description = section
                        .get("summary")
                        .and_then(Value::as_str)
                        .map_or(Value::Null, |summary| {
                            crate::html::clean_html(summary).into()
                        });
                    let timestamp = metadata
                        .get("publicationDate")
                        .and_then(Value::as_str)
                        .and_then(|date| crate::dates::unified_timestamp(date, true))
                        .map_or(Value::Null, Value::from);
                    let title =
                        strip_or_none(section.get("title")).map_or(Value::Null, Value::from);
                    entries.push(course_entry(
                        &target,
                        vec![
                            ("title", title),
                            ("description", description),
                            (
                                "thumbnail",
                                metadata.get("coverImage").cloned().unwrap_or(Value::Null),
                            ),
                            ("timestamp", timestamp),
                            ("chapter", chapter_title.clone()),
                            ("chapter_number", Value::from(index + 1)),
                            ("chapter_id", chapter_id.clone().into()),
                        ],
                    )?);
                }
            }
        }

        // `playlist_result(entries, course_id, title, description)`.
        let title = metadata
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .map(str::to_owned);
        let description = metadata
            .get("about")
            .and_then(Value::as_str)
            .map(crate::html::clean_html);
        let mut keys = vec!["id"];
        if title.is_some() {
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
            id: course_id,
            title: title.clone().unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: COURSE_NAME.into(),
            extractor_key: COURSE_NAME.into(),
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
        assert!(suitable(
            "https://www.packtpub.com/mapt/video/web-development/9781787122215/20528/20530/Project+Intro"
        ));
        assert!(suitable(
            "https://subscription.packtpub.com/video/programming/9781838988906/p1/video1_1/business-card-project"
        ));
        assert!(!course_suitable(
            "https://www.packtpub.com/mapt/video/web-development/9781787122215/20528/20530/Project+Intro"
        ));
        assert!(course_suitable(
            "https://www.packtpub.com/mapt/video/web-development/9781787122215"
        ));
        assert!(course_suitable(
            "https://subscription.packtpub.com/video/web_development/9781787122215"
        ));
    }
}
