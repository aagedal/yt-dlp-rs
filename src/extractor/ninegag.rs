//! Upstream `NineGagIE` (`yt_dlp/extractor/ninegag.py`): the `v1/post` API
//! (requested with upstream's impersonation warning), animated posts' image
//! variants as video formats (with per-codec URLs) and still images as
//! thumbnails, creator, counts, section and tags.
use super::clyp::key_error;
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NineGag;

const NAME: &str = "9gag";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?9gag\.com/gag/(?P<id>[^/?&#]+)").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("9GAG {what} are not supported natively"))
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| super::nuxt::truthy(value))
}

/// `value == 0` / `value == 1` with Python's numeric equality (`True == 1`).
fn equals(value: Option<&Value>, wanted: i64) -> bool {
    match value {
        Some(Value::Bool(flag)) => i64::from(*flag) == wanted,
        Some(Value::Number(number)) => number.as_f64() == Some(wanted as f64),
        _ => false,
    }
}

/// `traverse_obj(post, (first, second))`.
fn traverse<'a>(post: &'a Map<String, Value>, first: &str, second: &str) -> Option<&'a Value> {
    post.get(first)?
        .as_object()?
        .get(second)
        .filter(|value| !value.is_null())
}

impl Extractor for NineGag {
    fn key(&self) -> &'static str {
        "NineGag"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let post_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = super::update_url_query(
            &Url::parse("https://9gag.com/v1/post")?,
            &[("id", &post_id)],
        );
        let data = {
            let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
                crate::screen::warning_once(&format!("[{NAME}] {message}"));
            });
            download_json(client, &api, NAME, &post_id)?
        };
        let post = data
            .get("data")
            .ok_or_else(|| key_error(&post_id, "data"))?
            .get("post")
            .ok_or_else(|| key_error(&post_id, "post"))?
            .as_object()
            .ok_or_else(|| unsupported("posts that are not objects"))?;
        if post.get("type").and_then(Value::as_str) != Some("Animated") {
            crate::no_formats::raise_expected(
                NAME,
                None,
                Some(&post_id),
                "The given url does not contain a video",
            )?;
        }
        let mut duration = Value::Null;
        let mut formats = Vec::new();
        let mut thumbnails = Vec::new();
        let images = match post.get("images") {
            Some(images) if super::nuxt::truthy(images) => images
                .as_object()
                .ok_or_else(|| unsupported("image maps that are not objects"))?
                .clone(),
            _ => Map::new(),
        };
        for (key, image) in &images {
            let image = image
                .as_object()
                .ok_or_else(|| unsupported("images that are not objects"))?;
            let Some(image_url) = image
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let ext = super::direct::determine_ext(&image_url);
            // `key.strip('image')` strips those characters from both ends.
            let image_id = key.trim_matches(|c| "image".contains(c)).to_owned();
            let width = int_or_none(image.get("width"));
            let height = int_or_none(image.get("height"));
            match ext.as_deref() {
                Some(ext @ ("jpg" | "png")) => {
                    if let Some(webp_url) = truthy(image.get("webpUrl")) {
                        thumbnails.push(json!({
                            "url": webp_url,
                            "width": width,
                            "height": height,
                            "id": format!("{image_id}-webp"),
                        }));
                    }
                    thumbnails.push(json!({
                        "url": image_url,
                        "width": width,
                        "height": height,
                        "id": image_id,
                        "ext": ext,
                    }));
                }
                Some(ext @ ("webm" | "mp4")) => {
                    if !super::nuxt::truthy(&duration) {
                        duration = int_or_none(image.get("duration"));
                    }
                    let acodec = if equals(image.get("hasAudio"), 0) {
                        Value::from("none")
                    } else {
                        Value::Null
                    };
                    for vcodec in ["vp8", "vp9", "h265"] {
                        let Some(codec_url) = truthy(image.get(&format!("{vcodec}Url"))) else {
                            continue;
                        };
                        formats.push(dict_format(
                            vec![
                                ("url", codec_url.clone()),
                                ("width", width.clone()),
                                ("height", height.clone()),
                                ("acodec", acodec.clone()),
                                ("format_id", format!("{image_id}-{vcodec}").into()),
                                ("vcodec", vcodec.into()),
                            ],
                            NAME,
                            &post_id,
                        )?);
                    }
                    formats.push(dict_format(
                        vec![
                            ("url", image_url.clone().into()),
                            ("width", width),
                            ("height", height),
                            ("acodec", acodec),
                            ("ext", ext.into()),
                            ("format_id", image_id.into()),
                        ],
                        NAME,
                        &post_id,
                    )?);
                }
                _ => {}
            }
        }
        let section = traverse(post, "postSection", "name")
            .filter(|section| super::nuxt::truthy(section))
            .cloned();
        let tags = match truthy(post.get("tags")) {
            None => Value::Null,
            Some(Value::Array(tags)) => Value::Array(
                tags.iter()
                    .filter_map(|tag| truthy(tag.get("key")).cloned())
                    .collect(),
            ),
            Some(_) => return Err(unsupported("tag lists that are not arrays")),
        };
        let title = match post.get("title") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(title)) => crate::html::unescape_html(title).into(),
            Some(_) => return Err(unsupported("titles that are not strings")),
        };
        let uploader_url = traverse(post, "creator", "profileUrl")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map_or(Value::Null, Value::from);
        let mut info = dict_info(
            url,
            (NAME, "NineGag"),
            vec![
                ("id", post_id.clone().into()),
                ("title", title),
                ("timestamp", int_or_none(post.get("creationTs"))),
                ("duration", duration),
                (
                    "uploader",
                    traverse(post, "creator", "fullName")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "uploader_id",
                    traverse(post, "creator", "username")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                ("uploader_url", uploader_url),
                ("formats", Value::Null),
                ("thumbnails", Value::Array(thumbnails)),
                ("like_count", int_or_none(post.get("upVoteCount"))),
                ("dislike_count", int_or_none(post.get("downVoteCount"))),
                ("comment_count", int_or_none(post.get("commentsCount"))),
                (
                    "age_limit",
                    if equals(post.get("nsfw"), 1) {
                        18.into()
                    } else {
                        Value::Null
                    },
                ),
                (
                    "categories",
                    section.map_or(Value::Null, |section| Value::Array(vec![section])),
                ),
                ("tags", tags),
            ],
            formats,
        );
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=url,width,height,id,ext");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://9gag.com/gag/ae5Ag7B"));
        assert!(suitable("https://www.9gag.com/gag/aYKBYm9?ref=x"));
        assert!(!suitable("https://9gag.com/hot"));
    }
}
