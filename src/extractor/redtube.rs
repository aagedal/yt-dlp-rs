//! Upstream `RedTubeIE` (`yt_dlp/extractor/redtube.py`): the JSON-LD metadata,
//! the page's `sources` and `mediaDefinition` formats (direct MP4s, and HLS
//! masters reached through the `/media/...` JSON), the `Published on` date,
//! the OpenGraph duration and the view count. The title fallback patterns use
//! alternatives in place of upstream's backreferences.
use super::{
    Extractor, dict_format, dict_info, download_json, download_webpage,
    extract_m3u8_master_optional, search_warning,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct RedTube;

const NAME: &str = "RedTube";
const BASE: &str = "https://www.redtube.com";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:\w+\.)?redtube\.com(?:\.br)?/|embed\.redtube\.com/\?.*?\bid=)(?P<id>[0-9]+)",
    )
    .unwrap()
});
static TITLE_HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<h\d[^>]+class="(?:video_title_text|videoTitle|video_title)[^"]*">([^<\n]+)</h\d"#,
    )
    .unwrap()
});
static TITLE_SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:videoTitle|title)\s*:\s*(?:"([^"\n]+)"|'([^'\n]+)')"#).unwrap()
});
static SOURCES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"sources\s*:\s*(\{.+?\})").unwrap());
static MEDIA_DEFINITIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"mediaDefinition["']?\s*:\s*(\[.+?\}\s*\])"#).unwrap());
static VIDEO_DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"videoDuration\s*:\s*(\d+)").unwrap());
static UPLOAD_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<span[^>]+>(?:ADDED|Published on) ([^<]+)<").unwrap());
static VIEW_COUNTS: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r"<div[^>]*>Views</div>\s*<div[^>]*>\s*([\d,.]+)").unwrap(),
        Regex::new(r"<span[^>]*>VIEWS</span>\s*</td>\s*<td>\s*([\d,.]+)").unwrap(),
        Regex::new(r#"<span[^>]+\bclass=["']video_view_count[^>]*>\s*([\d,.]+)"#).unwrap(),
    ]
});
static SOURCE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<source src="(.+?)" type="video/mp4">"#).unwrap());

/// (markers, message) of the removed and private video pages (`any(p in
/// webpage)`).
const ERRORS: [(&[&str], &str); 2] = [
    (
        &["video-deleted-info", ">This video has been removed"],
        "has been removed",
    ),
    (
        &[
            "private_video_text",
            ">This video is private",
            ">Send a friend request to its owner to be able to view it",
        ],
        "is private",
    ),
];

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("RedTube {what} are not supported natively"))
}

fn search_error(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_download_json(url, video_id, fatal=False)`: HTTP errors are warnings.
fn download_json_optional(client: &Client, url: &Url, id: &str) -> Result<Option<Value>> {
    match download_json(client, url, NAME, id) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// `_parse_json(search_regex(pattern, ..., default=default), video_id,
/// fatal=False)`: the first capture's JSON, `default` when absent.
fn json_from_page(webpage: &str, pattern: &Regex, default: &str, id: &str) -> Result<Value> {
    let text = pattern
        .captures(webpage)
        .map_or(default.to_owned(), |captures| captures[1].to_owned());
    serde_json::from_str(&text)
        .map_err(|_| unsupported(&format!("JSON that does not parse ({id})")))
}

/// Python's `int_or_none` on a quality or format-ID string.
fn int_text(text: &str) -> Value {
    text.parse::<i64>().map_or(Value::Null, Value::from)
}

/// Upstream `str_to_int`: separators removed, then `int_or_none`.
fn str_to_int(text: &str) -> Option<i64> {
    text.chars()
        .filter(|character| !matches!(character, ',' | '.' | '+'))
        .collect::<String>()
        .parse()
        .ok()
}

/// Upstream `merge_dicts` of two dicts: a key keeps its first non-`None`
/// value, and an empty string is replaced.
fn merge_into(info: &mut Vec<(String, Value)>, key: &str, value: Value) {
    match info.iter_mut().find(|(name, _)| name == key) {
        Some((_, old)) => {
            if value.is_string() && old.as_str() == Some("") {
                *old = value;
            }
        }
        None if !value.is_null() => info.push((key.to_owned(), value)),
        None => {}
    }
}

/// `info[key] = value` (keeps an existing key's position).
fn set_key(info: &mut Vec<(String, Value)>, key: &str, value: Value) {
    match info.iter_mut().find(|(name, _)| name == key) {
        Some((_, old)) => *old = value,
        None => info.push((key.to_owned(), value)),
    }
}

impl Extractor for RedTube {
    fn key(&self) -> &'static str {
        NAME
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
        let page = Url::parse(&format!("{BASE}/{video_id}"))?;
        let webpage = download_webpage(client, &page, NAME, &video_id)?;

        for (markers, message) in ERRORS {
            if markers.iter().any(|marker| webpage.contains(marker)) {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Video {video_id} {message}"
                )));
            }
        }

        // `_search_json_ld(webpage, video_id, default={})`.
        let mut info: Vec<(String, Value)> = super::json_ld::search(&webpage).into_iter().collect();
        if !info
            .iter()
            .any(|(key, value)| key == "title" && super::nuxt::truthy(value))
        {
            let title = TITLE_HEADING
                .captures(&webpage)
                .map(|captures| captures[1].to_owned())
                .or_else(|| {
                    TITLE_SCRIPT.captures(&webpage).and_then(|captures| {
                        captures
                            .get(1)
                            .or_else(|| captures.get(2))
                            .map(|text| text.as_str().to_owned())
                    })
                })
                .map(|text| crate::html::clean_html(&text))
                .filter(|text| !text.is_empty())
                .or_else(|| crate::html::og_search_property(&webpage, "title"));
            if title.is_none() {
                search_warning(NAME, "OpenGraph title");
            }
            set_key(&mut info, "title", title.map_or(Value::Null, Value::from));
        }

        let mut formats = Vec::new();
        let sources = json_from_page(&webpage, &SOURCES, "{}", &video_id)?;
        if let Value::Object(sources) = sources {
            for (format_id, format_url) in sources {
                if !super::nuxt::truthy(&format_url) {
                    continue;
                }
                let Some(format_url) = format_url.as_str() else {
                    return Err(unsupported("source URLs that are not strings"));
                };
                formats.push(dict_format(
                    vec![
                        ("url", format_url.into()),
                        ("format_id", format_id.as_str().into()),
                        ("height", int_text(&format_id)),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }

        let medias = json_from_page(&webpage, &MEDIA_DEFINITIONS, "{}", &video_id)?;
        for media in medias.as_array().into_iter().flatten() {
            let Some(media_url) = media.get("videoUrl").and_then(Value::as_str) else {
                continue;
            };
            let format_url = Url::parse(BASE)?.join(media_url)?;
            let format_id = media.get("format").and_then(Value::as_str);
            let quality = media
                .get("quality")
                .filter(|value| super::nuxt::truthy(value));
            let more_media: Vec<Value> =
                if format_id == Some("hls") || (format_id == Some("mp4") && quality.is_none()) {
                    match download_json_optional(client, &format_url, &video_id)? {
                        Some(Value::Array(items)) => items,
                        _ => Vec::new(),
                    }
                } else {
                    vec![media.clone()]
                };
            for media in more_media {
                let Some(media_url) = media
                    .get("videoUrl")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                else {
                    continue;
                };
                let format_id = media.get("format").and_then(Value::as_str);
                if format_id == Some("hls") || format_url_ext_is_m3u8(&media_url) {
                    let manifest = Url::parse(&media_url)?;
                    if let Some(stream) = extract_m3u8_master_optional(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some(format_id.unwrap_or("hls")),
                        false,
                        false,
                    ) {
                        formats.extend(stream.formats);
                    }
                    continue;
                }
                let quality = media.get("quality").and_then(Value::as_str);
                formats.push(dict_format(
                    vec![
                        ("url", media_url.as_str().into()),
                        ("ext", "mp4".into()),
                        ("format_id", quality.map_or(Value::Null, Value::from)),
                        ("height", quality.map_or(Value::Null, int_text)),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        if formats.is_empty() {
            let Some(found) = SOURCE_TAG.captures(&webpage) else {
                return Err(search_error(&video_id, "video URL"));
            };
            formats.push(dict_format(
                vec![("url", found[1].into()), ("ext", "mp4".into())],
                NAME,
                &video_id,
            )?);
        }

        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            search_warning(NAME, "thumbnail URL");
        }
        let upload_date = UPLOAD_DATE
            .captures(&webpage)
            .and_then(|captures| crate::dates::unified_strdate(&captures[1], true));
        let duration = crate::html::og_search_property(&webpage, "video:duration")
            .or_else(|| {
                VIDEO_DURATION
                    .captures(&webpage)
                    .map(|captures| captures[1].to_owned())
            })
            .and_then(|text| text.parse::<i64>().ok());
        let view_count = VIEW_COUNTS
            .iter()
            .find_map(|pattern| pattern.captures(&webpage))
            .and_then(|captures| str_to_int(&captures[1]));

        // `merge_dicts(info, {...})`: the JSON-LD values come first.
        let extra: [(&str, Value); 8] = [
            ("id", video_id.as_str().into()),
            ("ext", "mp4".into()),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            ("upload_date", upload_date.map_or(Value::Null, Value::from)),
            ("duration", duration.map_or(Value::Null, Value::from)),
            ("view_count", view_count.map_or(Value::Null, Value::from)),
            ("age_limit", 18.into()),
            ("formats", Value::Null),
        ];
        for (key, value) in extra {
            merge_into(&mut info, key, value);
        }

        let fields: Vec<(&str, Value)> = info
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

/// Upstream `determine_ext(format_url) == 'm3u8'`.
fn format_url_ext_is_m3u8(url: &str) -> bool {
    super::direct::determine_ext(url).as_deref() == Some("m3u8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.redtube.com/38864951"));
        assert!(suitable(
            "http://embed.redtube.com/?bgcolor=000000&id=1443286"
        ));
        assert!(suitable("http://it.redtube.com/66418"));
        assert!(suitable("https://www.redtube.com.br/103224331"));
        assert!(!suitable("https://www.redtube.com/"));
    }

    #[test]
    fn view_counts_and_merges() {
        assert_eq!(str_to_int("602,808"), Some(602_808));
        let mut info = vec![("title".to_owned(), Value::from("a"))];
        merge_into(&mut info, "title", Value::from("b"));
        merge_into(&mut info, "thumbnail", Value::Null);
        merge_into(&mut info, "age_limit", Value::from(18));
        assert_eq!(info.len(), 2);
        assert_eq!(info[0].1, Value::from("a"));
    }
}
