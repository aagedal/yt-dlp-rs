//! Upstream `YouPornIE` (`yt_dlp/extractor/youporn.py`): a watch page's
//! `playervars` media definitions (the `/media/hls/` and `/media/mp4/` JSON
//! lists with their HLS masters and MP4 URLs), its title, counts and JSON-LD.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct YouPorn;

const NAME: &str = "YouPorn";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"https?://(?:www\.)?youporn\.com/(?:watch|embed)/(?P<id>\d+)(?:/(?P<display_id>[^/?#&]+))?/?(?:[#?]|$)",
    )
    .unwrap()
});
static WATCHABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div\s[^>]*\bid\s*=\s*(?:'watch-container'|"watch-container"|watch-container(?:[^-\w]|$))"#)
        .unwrap()
});
static PLAYER_VARS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bplayervars\s*:").unwrap());
static WATCH_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div[^>]+class=["']watchVideoTitle[^>]+>(.+?)</div>"#).unwrap()
});
static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div[^>]+\bid=["']description["'][^>]*>(.+?)</div>"#).unwrap()
});
static THUMBNAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:imageurl\s*=|poster\s*:)\s*["'](?P<thumbnail>[^"']+)["']"#).unwrap()
});
static UPLOADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div[^>]+class=["']submitByLink["'][^>]*>(.+?)</div>"#).unwrap()
});
static UPLOAD_DATE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"UPLOADED:\s*<span>([^<]+)",
        r"Date\s+[Aa]dded:\s*<span>([^<]+)",
        r#"(?s)<div[^>]+class=["']videoInfo(?:Date|Time)\b[^>]*>(.+?)</div>"#,
        r"(?s)<label\b[^>]*>Uploaded[^<]*</label>\s*<span\b[^>]*>(.+?)</span>",
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});
static VIEWS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(<div [^>]*\bdata-value\s*=[^>]+>)\s*<label>Views:</label>").unwrap()
});
static COMMENTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r">All [Cc]omments? \(([\d,.]+)\)").unwrap());
static CATEGORIES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)Categories:.*?</[^>]+>(.+?)</div>").unwrap());
static TAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)Tags:.*?</div>\s*<div[^>]+class=["']tagBoxContent["'][^>]*>(.+?)</div>"#)
        .unwrap()
});
static TAG_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<a[^>]+href=[^>]+>([^<]+)").unwrap());
static BITRATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?P<height>\d{3,4})[pP]_(?P<bitrate>\d+)[kK]_\d+").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_html_search_regex(pattern, webpage, name, default=None)`: the cleaned
/// first group.
fn html_search(pattern: &Regex, html: &str) -> Option<String> {
    pattern
        .captures(html)
        .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
}

/// `_get_format_data`: the JSON list behind a `mediaDefinitions` entry of
/// `stream_type` (`[]` when its JSON cannot be fetched).
fn format_data(
    definitions: &[Value],
    stream_type: &str,
    video_id: &str,
    client: &Client,
) -> Vec<Value> {
    let info_url = definitions.iter().find_map(|definition| {
        (definition.get("format").and_then(Value::as_str) == Some(stream_type))
            .then(|| {
                definition
                    .get("videoUrl")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
            })
            .flatten()
    });
    let Some(info_url) = info_url else {
        return Vec::new();
    };
    let Ok(url) = Url::parse(&info_url) else {
        return Vec::new();
    };
    let Ok(info) = download_json(client, &url, NAME, video_id) else {
        return Vec::new();
    };
    info.as_array()
        .into_iter()
        .flatten()
        .filter(|item| {
            item.get("format").and_then(Value::as_str) == Some(stream_type)
                && item
                    .get("videoUrl")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .is_some()
        })
        .cloned()
        .collect()
}

impl Extractor for YouPorn {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found.name("display_id").map(|m| m.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        crate::cookies::set_cookie(".youporn.com", "age_verified", "1");
        let watch_url = Url::parse(&format!("https://www.youporn.com/watch/{video_id}"))?;
        let webpage = download_webpage(client, &watch_url, NAME, &video_id)?;

        if !WATCHABLE.is_match(&webpage) {
            // `re.split(r'\s{2}', clean_html(mainContent))[0]`.
            let message = crate::html_elements::element_html_by_id("mainContent", &webpage)
                .ok()
                .flatten()
                .map(|element| crate::html::clean_html(&element))
                .unwrap_or_default();
            let message = message.split("  ").next().unwrap_or_default().to_owned();
            return Err(Error::Reported(if message.is_empty() {
                "Video unavailable".into()
            } else {
                format!("{NAME} says: {message}")
            }));
        }

        let player_vars = super::search_json(&PLAYER_VARS, &webpage, true)
            .flatten()
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract player vars; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let definitions = player_vars
            .get("mediaDefinitions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut formats = Vec::new();
        // Only the HLS entries without a boolean `defaultQuality` are masters
        // (the single-resolution ones are duplicates); fall back to all.
        let hls = format_data(&definitions, "hls", &video_id, client);
        let masters: Vec<&Value> = hls
            .iter()
            .filter(|item| !item.get("defaultQuality").is_some_and(Value::is_boolean))
            .collect();
        let masters: Vec<&Value> = if masters.is_empty() {
            hls.iter().collect()
        } else {
            masters
        };
        for item in masters {
            let Some(hls_url) = item
                .get("videoUrl")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let manifest = Url::parse(&hls_url)?;
            if let Some(info) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("hls"),
                false,
                false,
            ) {
                formats.extend(info.formats);
            }
        }

        for definition in format_data(&definitions, "mp4", &video_id, client) {
            let format_url = definition
                .get("videoUrl")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .unwrap_or_default();
            let mut fields: Vec<(&str, Value)> = vec![("url", format_url.clone().into())];
            if let Some(filesize) = definition.get("videoSize") {
                let size = super::streamable::int_or_none(Some(filesize));
                if !size.is_null() {
                    fields.push(("filesize", size));
                }
            }
            let mut height = super::streamable::int_or_none(definition.get("quality"));
            if let Some(found) = BITRATE.captures(&format_url) {
                if height.is_null() {
                    height = found["height"]
                        .parse::<i64>()
                        .map_or(Value::Null, Value::from);
                }
                let bitrate: i64 = found["bitrate"].parse().unwrap_or_default();
                fields.push((
                    "format_id",
                    format!("{}p-{bitrate}k", height.as_i64().unwrap_or_default()).into(),
                ));
                fields.push(("tbr", bitrate.into()));
            }
            fields.push(("height", height));
            let mut format = dict_format(fields, NAME, &video_id)?;
            // `tbr` is an int here (`int_or_none`), printed as one.
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }

        let title = html_search(&WATCH_TITLE, &webpage)
            .or_else(|| crate::html::og_search_property(&webpage, "title"))
            .or_else(|| crate::html::html_search_meta(&webpage, "title"));
        let description = html_search(&DESCRIPTION, &webpage)
            .or_else(|| crate::html::og_search_property(&webpage, "description"));
        let thumbnail = THUMBNAIL
            .captures(&webpage)
            .map(|found| found["thumbnail"].to_owned());
        let duration = super::streamable::int_or_none(player_vars.get("duration"));
        let uploader = html_search(&UPLOADER, &webpage);
        let upload_date = UPLOAD_DATE
            .iter()
            .find_map(|pattern| html_search(pattern, &webpage));
        let upload_date = upload_date.and_then(|date| crate::dates::unified_strdate(&date, false));
        if upload_date.is_none() {
            super::search_warning(NAME, "upload date");
        }
        let age_limit = crate::html::rta_search(&webpage);
        let view_count = VIEWS
            .captures(&webpage)
            .and_then(|found| extract_data_value(&found[1]));
        let comment_count = COMMENTS
            .captures(&webpage)
            .and_then(|found| super::youtube::dict::parse_count(&found[1]).map(Value::from));
        let tags_of = |pattern: &Regex| -> Vec<Value> {
            pattern
                .captures(&webpage)
                .map(|found| {
                    TAG_LINK
                        .captures_iter(&found[1])
                        .map(|link| Value::from(link[1].to_owned()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let categories = tags_of(&CATEGORIES);
        let tags = tags_of(&TAGS);

        // `_search_json_ld(webpage, video_id, expected_type='VideoObject')`.
        let mut data: Map<String, Value> = super::json_ld::search(&webpage);
        data.shift_remove("url");

        let mut info = data;
        let literal: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("display_id", display_id.into()),
            ("title", title.into()),
            ("description", description.into()),
            ("thumbnail", thumbnail.into()),
            ("duration", duration),
            ("uploader", uploader.into()),
            ("upload_date", upload_date.into()),
            ("view_count", view_count.into()),
            ("comment_count", comment_count.into()),
            ("categories", Value::Array(categories)),
            ("tags", Value::Array(tags)),
            (
                "age_limit",
                age_limit.map(Value::from).unwrap_or(Value::Null),
            ),
        ];
        for (key, value) in literal {
            merge_value(&mut info, key, value);
        }
        // Remove SEO spam "description".
        let title_text = info
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if info
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(|text| text.starts_with(&format!("Watch {title_text} online")))
        {
            info.shift_remove("description");
        }

        let mut fields: Vec<(&str, Value)> = info
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        fields.push(("formats", Value::Null));
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

/// `merge_dicts`: a value fills a missing, null or empty key.
fn merge_value(info: &mut Map<String, Value>, key: &str, value: Value) {
    if value.is_null() {
        return;
    }
    let empty = match info.get(key) {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty(),
        _ => false,
    };
    if empty {
        info.insert(key.to_owned(), value);
    }
}

/// `int/parse_count` of a `data-value` attribute.
fn extract_data_value(element: &str) -> Option<Value> {
    let attrs = crate::html_elements::extract_attributes(element);
    let text = attrs.get("data-value")?.as_str()?.to_owned();
    super::youtube::dict::parse_count(&text).map(Value::from)
}
