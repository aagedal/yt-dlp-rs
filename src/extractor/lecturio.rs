//! Upstream `LecturioIE` (`yt_dlp/extractor/lecturio.py`): a lecture's media
//! files and captions from the lecture's HTML5 API JSON. Logging in and the
//! course and `lecturio.de` course extractors are not supported natively.
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Lecturio;

const NAME: &str = "Lecturio";
const API_BASE: &str = "https://app.lecturio.com/api/en/latest/html5/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https://(?:app\.lecturio\.com/(?:[^/?#]+/(?P<nt>[^/?#&]+)\.lecture|(?:\#/)?lecture/c/\d+/(?P<id>\d+))|(?:www\.)?lecturio\.de/(?:[^/?#]+/)+(?P<nt_de>[^/?#&]+)\.vortrag)",
    )
    .unwrap()
});
static LABEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)p\s*\(([^)]+)\)").unwrap());
static CC_LANG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/([a-z]{2})_").unwrap());
static CC_ORIGINAL_LANG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/[a-z]{2}_([a-z]{2})_").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

// `_CC_LANGS` is keyed by language names, so upstream's `.get(code, code)`
// always keeps the caption's language code.

/// Python's truthiness of a JSON value, for `or` defaults.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

/// `KeyError(key)` escaping `_real_extract` before the video ID is known.
fn key_error(key: &str) -> Error {
    Error::Reported(format!(
        "An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Lecturio {what} are not supported natively"))
}

/// `video['title'].strip()` plus the rest of `_real_extract`'s formats and
/// captions, as the info dict.
fn result_from_video(
    url: &Url,
    video: &Value,
    lecture_id: Option<String>,
    display_id: &str,
) -> Result<VideoInfo> {
    let title = video
        .get("title")
        .ok_or_else(|| key_error("title"))?
        .as_str()
        .ok_or_else(|| unsupported("titles that are not strings"))?
        .trim()
        .to_owned();
    let mut lecture_id = lecture_id;
    if lecture_id.is_none() {
        let pid = video
            .get("productId")
            .filter(|value| truthy(Some(value)))
            .or_else(|| video.get("uid").filter(|value| truthy(Some(value))));
        if let Some(pid) = pid {
            let pid = pid
                .as_str()
                .ok_or_else(|| unsupported("product IDs that are not strings"))?;
            let parts: Vec<&str> = pid.split('_').collect();
            if let [_, id] = parts.as_slice() {
                lecture_id = Some((*id).to_owned());
            }
        }
    }

    let content = video.get("content").ok_or_else(|| key_error("content"))?;
    let media = content.get("media").ok_or_else(|| key_error("media"))?;
    let media = media
        .as_array()
        .ok_or_else(|| unsupported("media lists that are not lists"))?;
    let mut formats = Vec::new();
    for format in media {
        if !format.is_object() {
            continue;
        }
        let Some(file) = format
            .get("file")
            .and_then(Value::as_str)
            .filter(|file| !file.is_empty())
        else {
            continue;
        };
        if super::direct::determine_ext(file).as_deref() == Some("smil") {
            // SMIL contains only broken RTMP formats anyway.
            continue;
        }
        let Some(file_url) = super::json_ld::url_or_none(file) else {
            continue;
        };
        let label = format
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_owned);
        // `int_or_none(fileSize, invscale=1000)`.
        let filesize = super::streamable::int_or_none(format.get("fileSize"))
            .as_i64()
            .map(|size| size * 1000);
        let mut fields: Vec<(&str, Value)> = vec![
            ("url", file_url.into()),
            ("format_id", label.clone().map_or(Value::Null, Value::from)),
            ("filesize", filesize.map_or(Value::Null, Value::from)),
        ];
        // `re.match(r'(\d+)p\s*\(([^)]+)\)', label)` renames the format.
        if let Some(found) = label.as_deref().and_then(|label| LABEL.captures(label)) {
            fields[1] = ("format_id", found[2].into());
            fields.push((
                "height",
                found[1].parse::<i64>().map_or(Value::Null, Value::from),
            ));
        }
        formats.push(dict_format(fields, NAME, display_id)?);
    }

    let mut subtitles = Map::new();
    let mut automatic_captions = Map::new();
    if let Some(captions) = video.get("captions").and_then(Value::as_array) {
        for caption in captions {
            let Some(cc_url) = caption
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
            else {
                continue;
            };
            let cc_label = caption.get("translatedCode").and_then(Value::as_str);
            let language_code = caption
                .get("languageCode")
                .and_then(Value::as_str)
                .filter(|code| !code.is_empty());
            let lang = match language_code {
                Some(code) => code.to_owned(),
                None => CC_LANG.captures(cc_url).map_or_else(
                    || {
                        cc_label
                            .and_then(|label| label.split_whitespace().next())
                            .unwrap_or("en")
                            .to_owned()
                    },
                    |found| found[1].to_owned(),
                ),
            };
            let original_lang = CC_ORIGINAL_LANG
                .captures(cc_url)
                .map(|found| found[1].to_owned());
            let auto = cc_label.is_some_and(|label| label.contains("auto-translated"))
                || original_lang.is_some();
            let target = if auto {
                &mut automatic_captions
            } else {
                &mut subtitles
            };
            let entry = target
                .entry(lang)
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(items) = entry {
                // yt-dlp's subtitle normalization adds the file's extension.
                let ext =
                    super::direct::determine_ext(cc_url).unwrap_or_else(|| "unknown_video".into());
                items.push(serde_json::json!({"url": cc_url, "ext": ext}));
            }
        }
    }

    let id = lecture_id.unwrap_or_else(|| display_id.to_owned());
    let info = dict_info(
        url,
        (NAME, NAME),
        vec![
            ("id", id.into()),
            ("title", title.into()),
            ("formats", Value::Null),
            ("subtitles", Value::Object(subtitles)),
            ("automatic_captions", Value::Object(automatic_captions)),
        ],
        formats,
    );
    Ok(info)
}

impl Extractor for Lecturio {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let nt = found
            .name("nt")
            .or_else(|| found.name("nt_de"))
            .map(|found| found.as_str().to_owned());
        let lecture_id = found.name("id").map(|found| found.as_str().to_owned());
        let display_id = nt
            .clone()
            .or_else(|| lecture_id.clone())
            .unwrap_or_default();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api_path = match &lecture_id {
            Some(id) => format!("lectures/{id}"),
            None => format!("lecture/{display_id}.json"),
        };
        let api = Url::parse(&format!("{API_BASE}{api_path}"))?;
        let video = download_json(client, &api, NAME, &display_id)?;
        result_from_video(url, &video, lecture_id, &display_id).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://app.lecturio.com/medical-courses/important-concepts-and-terms-introduction-to-microbiology.lecture#tab/videos"
        ));
        assert!(suitable("https://app.lecturio.com/#/lecture/c/6434/39634"));
        assert!(suitable(
            "https://www.lecturio.de/jura/oeffentliches-recht-staatsexamen.vortrag"
        ));
        assert!(!suitable("https://app.lecturio.com/#/course/c/6434"));
        assert!(!suitable(
            "https://app.lecturio.com/medical-courses/microbiology-introduction.course#/"
        ));
    }

    #[test]
    fn label_renames_quality_formats() {
        let found = LABEL.captures("360p (LD)").unwrap();
        assert_eq!((&found[1], &found[2]), ("360", "LD"));
        assert!(LABEL.captures("HLS").is_none());
    }
}
