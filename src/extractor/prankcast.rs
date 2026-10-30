//! Upstream `PrankCastIE` and `PrankCastPostIE` (`yt_dlp/extractor/prankcast.py`):
//! a showreel's or post's next.js page data, its recording URL as a direct
//! MP3 and the broadcast's metadata.
use super::clyp::key_error;
use super::{Extractor, dict_format, dict_info, download_webpage, search_nextjs_data};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct PrankCast;
pub struct PrankCastPost;

const NAME: &str = "PrankCast";
const POST_NAME: &str = "PrankCastPost";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?prankcast\.com/[^/?#]+/showreel/(?P<id>\d+)-(?P<display_id>[^/?#]+)",
    )
    .unwrap()
});
static POST_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?prankcast\.com/[^/?#]+/posts/(?P<id>\d+)-(?P<display_id>[^/?#]+)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_valid_url` for `PrankCastPostIE`.
pub fn post_suitable(url: &str) -> bool {
    POST_VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("PrankCast {what} are not supported natively"))
}

/// `data[key]` along a path, a missing key being a `KeyError`.
fn path<'a>(value: &'a Value, keys: &[&str], id: &str) -> Result<&'a Value> {
    keys.iter().try_fold(value, |value, key| {
        value.get(*key).ok_or_else(|| key_error(id, key))
    })
}

/// A string field (`None` when absent, null or not a string).
fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A non-empty string field (Python's `x.get(k) or fallback`).
fn truthy_text(value: &Value, key: &str) -> Option<String> {
    text(value, key).filter(|text| !text.is_empty())
}

/// `_parse_json(x or '{}')` of a JSON-encoded string field.
fn parse_json_field(value: &Value, key: &str) -> Result<Value> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(Value::Object(Default::default())),
        Some(Value::String(text)) if text.is_empty() => Ok(Value::Object(Default::default())),
        Some(Value::String(text)) => {
            serde_json::from_str(text).map_err(|_| unsupported("JSON fields that do not parse"))
        }
        Some(_) => Err(unsupported("JSON fields that are not strings")),
    }
}

/// `str(x)` for an ID-like field (`None` stays null).
fn str_or_none(value: Option<&Value>) -> Value {
    match value {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(text)) => text.clone().into(),
        Some(other) => other.to_string().into(),
    }
}

/// The `og:title`, warning when absent (`_og_search_title(webpage)`).
fn og_title(webpage: &str) -> Option<String> {
    let title = crate::html::og_search_property(webpage, "title");
    if title.is_none() {
        crate::screen::warning(&format!(
            "[{NAME}] unable to extract OpenGraph title; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    title
}

/// The `timestamp` from `parse_iso8601(...)`: `start` and `end` are the
/// showreel's ISO 8601 date strings.
fn iso_timestamp(text: Option<&str>) -> Option<i64> {
    text.and_then(crate::dates::parse_iso8601)
}

impl Extractor for PrankCast {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let next = search_nextjs_data(&webpage, NAME, &video_id)?;
        let json_info = path(
            &next,
            &["props", "pageProps", "ssr_data_showreel"],
            &video_id,
        )?;

        let uploader = json_info.get("user_name").cloned().unwrap_or(Value::Null);
        let guests = parse_json_field(json_info, "guests_json")?;
        let start_date = iso_timestamp(json_info.get("start_date").and_then(Value::as_str));
        let title = truthy_text(json_info, "broadcast_title")
            .map(Value::from)
            .or_else(|| og_title(&webpage).map(Value::from))
            .unwrap_or(Value::Null);
        let broadcast_url = path(json_info, &["broadcast_url"], &video_id)?
            .as_str()
            .ok_or_else(|| unsupported("broadcast URLs that are not strings"))?;
        let recording_hash = path(json_info, &["recording_hash"], &video_id)?
            .as_str()
            .ok_or_else(|| unsupported("recording hashes that are not strings"))?;
        let media_url = format!("{broadcast_url}{recording_hash}.mp3");

        // `try_call(lambda: parse_iso8601(end_date) - start_date)`.
        let duration = match (
            iso_timestamp(json_info.get("end_date").and_then(Value::as_str)),
            start_date,
        ) {
            (Some(end), Some(start)) => Value::from(end - start),
            _ => Value::Null,
        };
        // `list(filter(None, [uploader, *guest names]))`.
        let mut cast: Vec<Value> = Vec::new();
        for name in std::iter::once(uploader.clone()).chain(
            guests
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|guest| guest.get("name").cloned()),
        ) {
            match &name {
                Value::String(text) if text.is_empty() => {}
                Value::String(_) => cast.push(name),
                Value::Null => {}
                _ => return Err(unsupported("guest names that are not strings")),
            }
        }
        let tags = json_info
            .get("broadcast_tags")
            .and_then(Value::as_str)
            .map_or(Value::Null, |tags| {
                tags.split(',').map(Value::from).collect::<Vec<_>>().into()
            });

        Ok(Some(direct_info(
            url,
            (NAME, NAME),
            &media_url,
            vec![
                ("id", video_id.clone().into()),
                ("title", title),
                ("display_id", display_id.into()),
                ("url", media_url.clone().into()),
                ("timestamp", start_date.map_or(Value::Null, Value::from)),
                ("uploader", uploader),
                ("channel_id", str_or_none(json_info.get("user_id"))),
                ("duration", duration),
                ("cast", cast.into()),
                (
                    "description",
                    json_info
                        .get("broadcast_description")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "categories",
                    vec![
                        json_info
                            .get("broadcast_category")
                            .cloned()
                            .unwrap_or(Value::Null),
                    ]
                    .into(),
                ),
                ("tags", tags),
            ],
        )?))
    }
}

impl Extractor for PrankCastPost {
    fn key(&self) -> &'static str {
        POST_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = POST_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(POST_NAME);
        crate::screen::screen(&format!(
            "[{POST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, POST_NAME, &video_id)?;
        let next = search_nextjs_data(&webpage, POST_NAME, &video_id)?;
        let post = path(&next, &["props", "pageProps", "ssr_data_posts"], &video_id)?;
        let contents = parse_json_field_text(post, "post_contents_json", &video_id)?;
        let content = contents
            .as_array()
            .and_then(|items| items.first())
            .cloned()
            .ok_or_else(|| unsupported("posts without contents"))?;

        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("display_id", display_id.into()),
            ("title", og_title(&webpage).map_or(Value::Null, Value::from)),
        ];
        // The post's `traverse_obj` keys: a present value overrides the
        // page's `og:title`, and `None` values are left out.
        let post_title = post.get("post_title").and_then(Value::as_str);
        if let Some(title) = post_title {
            set_field(&mut fields, "title", title.into());
        }
        if let Some(body) = post.get("post_body").and_then(Value::as_str) {
            fields.push(("description", body.into()));
        }
        if let Some(tags) = post.get("post_tags").and_then(Value::as_str) {
            let tags: Vec<Value> = tags
                .split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(Value::from)
                .collect();
            if !tags.is_empty() {
                fields.push(("tags", tags.into()));
            }
        }
        if let Some(channel) = post.get("user_id").and_then(Value::as_i64) {
            fields.push(("channel_id", channel.to_string().into()));
        }
        if let Some(uploader) = post.get("user_name").and_then(Value::as_str) {
            fields.push(("uploader", uploader.into()));
        }

        let media_url = ["secure_url", "url"]
            .iter()
            .filter_map(|key| text(&content, key))
            .find(|candidate| candidate.starts_with("http"));
        let timestamp = [
            iso_timestamp_space(text(&content, "start_date").as_deref()),
            iso_timestamp_space(text(&content, "crdate").as_deref()),
            iso_timestamp(text(&content, "created_at").as_deref()),
        ]
        .into_iter()
        .flatten()
        .next();
        if let Some(url) = &media_url {
            fields.push(("url", url.clone().into()));
        }
        if let Some(timestamp) = timestamp {
            fields.push(("timestamp", timestamp.into()));
        }
        if let Some(duration) = content.get("duration").and_then(Value::as_f64) {
            fields.push(("duration", duration.into()));
        }
        if let Some(category) = truthy_text(&content, "category") {
            fields.push(("categories", vec![category].into()));
        }
        let mut cast: Vec<Value> = Vec::new();
        if let Some(uploader) = post.get("user_name").and_then(Value::as_str)
            && !uploader.is_empty()
        {
            cast.push(uploader.into());
        }
        let guests = parse_json_field_text(&content, "guests_json", &video_id)
            .unwrap_or_else(|_| Value::Null);
        for guest in guests.as_array().into_iter().flatten() {
            if let Some(name) = guest.get("name").and_then(Value::as_str)
                && !name.is_empty()
            {
                cast.push(name.into());
            }
        }
        if !cast.is_empty() {
            fields.push(("cast", cast.into()));
        }

        match &media_url {
            Some(media_url) => Ok(Some(direct_info(
                url,
                (POST_NAME, POST_NAME),
                media_url,
                fields,
            )?)),
            None => {
                // Without a media URL the dict has no format at all.
                fields.push(("formats", Value::Null));
                Ok(Some(dict_info(
                    url,
                    (POST_NAME, POST_NAME),
                    fields,
                    Vec::new(),
                )))
            }
        }
    }
}

/// A direct media dict (`{'url': media, ...}`): upstream processes the URL as
/// the only format, so the info dict carries that format's fields and no
/// `formats` list. Keys keep the dict's order.
fn direct_info(
    page: &Url,
    (extractor, key): (&'static str, &'static str),
    media: &str,
    fields: Vec<(&str, Value)>,
) -> Result<VideoInfo> {
    let mut format = dict_format(
        vec![
            ("format_id", "".into()),
            ("url", media.into()),
            ("ext", "mp3".into()),
        ],
        extractor,
        "",
    )?;
    crate::info_order::mark_format(&mut format, "url-only");
    let fields: Vec<(&str, Value)> = fields
        .into_iter()
        .filter(|(name, _)| *name != "formats")
        .collect();
    let keys: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    let shape = format!("untyped-dict;{}", keys.join(","));
    let info = dict_info(page, (extractor, key), fields, vec![format]);
    Ok(crate::info_order::mark_info(info, &shape))
}

/// A JSON-encoded string field, parsed (`_parse_json`).
fn parse_json_field_text(value: &Value, key: &str, id: &str) -> Result<Value> {
    match value.get(key).and_then(Value::as_str) {
        Some(text) => serde_json::from_str(text).map_err(|_| {
            Error::Reported(format!(
                "[{POST_NAME}] {id}: Failed to parse JSON; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        }),
        None => Err(key_error(id, key)),
    }
}

/// `parse_iso8601(date, delimiter=' ')`.
fn iso_timestamp_space(text: Option<&str>) -> Option<i64> {
    text.and_then(crate::dates::parse_iso8601_space)
}

/// Replaces `key`'s value in place, or appends it (dict key order).
fn set_field(fields: &mut Vec<(&str, Value)>, key: &'static str, value: Value) {
    match fields.iter_mut().find(|(name, _)| *name == key) {
        Some((_, slot)) => *slot = value,
        None => fields.push((key, value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://prankcast.com/Devonanustart/showreel/1561-Beverly-is-back-like-a-heart-attack-"
        ));
        assert!(!suitable("https://prankcast.com/Devonanustart/"));
        assert!(post_suitable(
            "https://prankcast.com/devonanustart/posts/6214-happy-national-rachel-day-"
        ));
        assert!(!post_suitable("https://prankcast.com/devonanustart/posts/"));
    }
}
