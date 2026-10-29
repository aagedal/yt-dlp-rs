//! Upstream `SinaIE` (`yt_dlp/extractor/sina.py`): the video ID (from the
//! URL or the page's `video_id`), the `h5play` JSON and its MP4 qualities.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Sina;

const NAME: &str = "Sina";
const QUALITIES: [&str; 5] = ["cif", "sd", "hd", "fhd", "ffd"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^https?://(?:[^/?#]+\.)?video\.sina\.com\.cn/(?:",
        r"(?:view/|.*\#)(?P<id>\d+)|",
        r".+?/(?P<pseudo_id>[^/?#]+)(?:\.s?html)|",
        r"api/sinawebApi/outplay\.php/(?P<token>.+?)\.swf)",
    ))
    .unwrap()
});
static VIDEO_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"video_id\s*:\s*'(\d+)'").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Sina {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// Python's `str()` of a JSON value in an error message.
fn python_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        other => other.to_string(),
    }
}

/// `video_data['code'] != 1` (Python's `True == 1`).
fn code_is_one(code: &Value) -> bool {
    match code {
        Value::Number(number) => number.as_f64() == Some(1.0),
        Value::Bool(value) => *value,
        _ => false,
    }
}

/// The page's video ID for a pseudo-ID URL (`errtitle` is the site's error).
fn page_video_id(client: &Client, url: &Url, pseudo_id: &str) -> Result<String> {
    let webpage = download_webpage(client, url, NAME, pseudo_id)?;
    let errors =
        crate::html_elements::elements_text_and_html_by_attribute("class", "errtitle", &webpage)
            .map_err(|_| unsupported("error elements that do not parse"))?;
    if let Some((error, _)) = errors.into_iter().next()
        && !error.is_empty()
    {
        return Err(Error::Reported(format!(
            "[{NAME}] {NAME} said: {}",
            crate::html::clean_html(&error)
        )));
    }
    VIDEO_ID
        .captures(&webpage)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] Unable to extract video id; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

impl Extractor for Sina {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = match (found.name("id"), found.name("pseudo_id")) {
            (Some(id), _) => id.as_str().to_owned(),
            _ if found.name("token").is_some() => {
                // Upstream follows a HEAD request's redirect to the video URL.
                return Err(unsupported("outplay.php token URLs"));
            }
            (None, Some(pseudo_id)) => page_video_id(client, url, pseudo_id.as_str())?,
            (None, None) => return Err(unsupported("URLs without a video ID")),
        };

        let api = super::update_url_query(
            &Url::parse("http://s.video.sina.com.cn/video/h5play")?,
            &[("video_id", &video_id)],
        );
        let video_data = download_json(client, &api, NAME, &video_id)?;
        let code = video_data
            .get("code")
            .ok_or_else(|| key_error(&video_id, "code"))?;
        if !code_is_one(code) {
            let message = video_data
                .get("message")
                .ok_or_else(|| key_error(&video_id, "message"))?;
            // `get_temp_id(url)`: only the URL's own ID prefixes the error.
            let prefix = found
                .name("id")
                .map_or(String::new(), |id| format!(" {}:", id.as_str()));
            return Err(Error::Reported(format!(
                "[{NAME}]{prefix} {NAME} said: {}",
                python_str(message)
            )));
        }
        let video_data = video_data
            .get("data")
            .ok_or_else(|| key_error(&video_id, "data"))?;
        let title = video_data
            .get("title")
            .ok_or_else(|| key_error(&video_id, "title"))?
            .clone();
        let description = match video_data.get("description") {
            Some(Value::String(text)) if !text.is_empty() => Value::from(text.trim()),
            Some(Value::String(_)) | None | Some(Value::Null) => video_data
                .get("description")
                .cloned()
                .unwrap_or(Value::Null),
            Some(_) => return Err(unsupported("descriptions that are not strings")),
        };

        let mut formats = Vec::new();
        let qualities = video_data
            .get("videos")
            .and_then(|videos| videos.get("mp4"))
            .and_then(Value::as_object);
        for (quality_id, quality) in qualities.into_iter().flatten() {
            let file_api = quality.get("file_api").filter(|v| super::nuxt::truthy(v));
            let file_id = quality.get("file_id").filter(|v| super::nuxt::truthy(v));
            let (Some(Value::String(file_api)), Some(file_id)) = (file_api, file_id) else {
                if file_api.is_some() && file_id.is_some() {
                    return Err(unsupported("file APIs that are not strings"));
                }
                continue;
            };
            let file_id = match file_id {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                _ => return Err(unsupported("file IDs that are not strings")),
            };
            let file_api = Url::parse(file_api).map_err(|_| unsupported("file APIs"))?;
            let format_url = super::update_url_query(&file_api, &[("vid", &file_id)]);
            let preference = QUALITIES
                .iter()
                .position(|name| name == quality_id)
                .map_or(-1, |index| index as i64);
            formats.push(dict_format(
                vec![
                    ("format_id", quality_id.clone().into()),
                    ("url", format_url.as_str().into()),
                    ("quality", preference.into()),
                    ("ext", "mp4".into()),
                ],
                NAME,
                &video_id,
            )?);
        }

        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("description", description),
                (
                    "thumbnail",
                    video_data.get("image").cloned().unwrap_or(Value::Null),
                ),
                (
                    "duration",
                    super::streamable::int_or_none(video_data.get("length")),
                ),
                (
                    "timestamp",
                    super::streamable::int_or_none(video_data.get("create_time")),
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
        assert!(suitable(
            "http://video.sina.com.cn/news/spj/topvideoes20160504/?opsubject_id=top1#250576622"
        ));
        assert!(suitable("http://video.sina.com.cn/view/250587748.html"));
        assert!(suitable(
            "http://video.sina.com.cn/v/b/101314253-1290078633.html"
        ));
        assert!(!suitable("http://video.sina.com.cn/"));
    }
}
