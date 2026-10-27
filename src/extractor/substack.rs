//! Upstream `SubstackIE` (`yt_dlp/extractor/substack.py`): a post page's
//! `window._preloads = JSON.parse("...")` data, with podcast posts' audio
//! URL (its extension resolved by a HEAD request when the URL has none),
//! video posts' non-fatal HLS and MP4 upload sources, the custom-domain
//! canonical URL and the post metadata.
//!
//! Not ported: `_extract_embed_urls` (custom-domain pages in the generic
//! extractor).
use super::{Extractor, dict_format, dict_info, download_webpage, extract_m3u8_master_optional};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Substack;

const NAME: &str = "Substack";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://[\w-]+\.substack\.com/p/(?P<id>[\w-]+)").unwrap());
/// `_search_json(r'window\._preloads\s*=\s*JSON\.parse\(', ...,
/// contains_pattern=r'"{(?s:.+)}"')`.
static PRELOADS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"window\._preloads\s*=\s*JSON\.parse\(\s*(?P<json>"\{(?s:.+)\}")"#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Substack {what} are not supported natively"))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `value[key]`.
fn require<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| key_error(id, key))
}

/// `traverse_obj(value, path)`: `None` along a missing path.
fn path(value: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .try_fold(value, |value, key| value.get(*key))
        .cloned()
        .unwrap_or(Value::Null)
}

/// Python's `str(value)` for the scalar IDs Substack uses.
fn py_str(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Null => Ok("None".into()),
        Value::Bool(flag) => Ok(if *flag { "True" } else { "False" }.into()),
        _ => Err(unsupported("post IDs that are not scalars")),
    }
}

/// The page's preloads: the quoted JSON literal (`js_to_json`), whose
/// string value is itself the JSON data.
fn preloads(webpage: &str, id: &str) -> Result<Value> {
    let Some(found) = PRELOADS.captures(webpage) else {
        return Err(Error::Reported(format!(
            "[{NAME}] {id}: Unable to extract json string; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let literal = crate::js_to_json::js_to_json(&found["json"]);
    let decoded = serde_json::Deserializer::from_str(&literal)
        .into_iter::<Value>()
        .next()
        .and_then(std::result::Result::ok);
    let Some(Value::String(text)) = decoded else {
        return Err(unsupported("preloads that are not JSON string literals"));
    };
    crate::json::from_str(&text).map_err(|_| unsupported("preloads that are not valid JSON"))
}

impl Extractor for Substack {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let info = preloads(&webpage, &display_id)?;

        let mut canonical = url.clone();
        if let Some(domain) = path(&info, &["domainInfo", "customDomain"])
            .as_str()
            .filter(|domain| !domain.is_empty())
        {
            canonical = Url::parse(&format!(
                "{}://{domain}{}",
                url.scheme(),
                &url[url::Position::BeforePath..]
            ))?;
        }

        let post = require(&info, "post", &display_id)?;
        let post_type = require(post, "type", &display_id)?;
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        match post_type.as_str() {
            Some("podcast") => {
                let podcast_url = require(post, "podcast_url", &display_id)?
                    .as_str()
                    .ok_or_else(|| unsupported("podcast URLs that are not strings"))?
                    .to_owned();
                let mut fields = vec![("url", Value::from(podcast_url.as_str()))];
                if super::direct::determine_ext(&podcast_url).is_none() {
                    // `_request_webpage(HEADRequest(url), ..., 'Resolving
                    // podcast file extension', 'Podcast URL is invalid')`.
                    crate::screen::screen(&format!(
                        "[{NAME}] {display_id}: Resolving podcast file extension"
                    ));
                    let media = Url::parse(&podcast_url)?;
                    crate::screen::before_extraction_request();
                    let response = crate::geo::add_header(client.head(media), &[])
                        .send()
                        .inspect(|_| crate::screen::after_request())
                        .map_err(Error::Http)
                        .and_then(crate::retry::check_response)
                        .map_err(|error| match error {
                            Error::Http(_) | Error::HttpRetryAfter { .. } => {
                                Error::Reported(format!(
                                    "[{NAME}] {display_id}: Podcast URL is invalid: {}",
                                    crate::upstream_error_text(&error)
                                ))
                            }
                            error => error,
                        })?;
                    let ext = super::direct::determine_ext(response.url().as_str())
                        .unwrap_or_else(|| "unknown_video".into());
                    fields.push(("ext", ext.into()));
                }
                formats.push(dict_format(fields, NAME, &display_id)?);
            }
            Some("video") => {
                let upload = require(
                    require(post, "videoUpload", &display_id)?,
                    "id",
                    &display_id,
                )?;
                let video_id = py_str(upload)?;
                for kind in ["hls", "mp4"] {
                    let video_url = canonical
                        .join(&format!("/api/v1/video/upload/{video_id}/src?type={kind}"))?;
                    if kind == "hls" {
                        if let Some(stream) = extract_m3u8_master_optional(
                            client, &video_url, NAME, &video_id, None, true, false,
                        ) {
                            formats.extend(stream.formats);
                            super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                        }
                    } else {
                        formats.push(dict_format(
                            vec![("url", video_url.as_str().into()), ("ext", "mp4".into())],
                            NAME,
                            &video_id,
                        )?);
                    }
                }
            }
            _ => {
                // `raise_no_formats(msg)`: not `expected`, so it raises even
                // with `--ignore-no-formats-error`.
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Page type \"{}\" is not supported; {}",
                    py_str(post_type).unwrap_or_else(|_| post_type.to_string()),
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        }

        let uploader_id = match path(post, &["publication_id"]) {
            Value::Null => Value::Null,
            value => py_str(&value)?.into(),
        };
        let fields = vec![
            (
                "id",
                Value::from(py_str(require(post, "id", &display_id)?)?),
            ),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("title", path(post, &["title"])),
            ("description", path(post, &["description"])),
            ("thumbnail", path(post, &["cover_image"])),
            ("uploader", path(&info, &["pub", "name"])),
            ("uploader_id", uploader_id),
            ("webpage_url", canonical.as_str().into()),
        ];
        let mut result = dict_info(url, (NAME, NAME), fields, formats);
        result.subtitles = subtitles;
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://haleynahman.substack.com/p/i-made-a-vlog?s=r"
        ));
        assert!(suitable("https://persuasion1.substack.com/p/summers"));
        assert!(!suitable("https://substack.com/p/x"));
        assert!(!suitable("https://www.mollymovieclub.com/p/interstellar"));
    }

    #[test]
    fn preloads_are_a_json_string_literal() {
        let page = r#"<script>window._preloads = JSON.parse("{\"post\":{\"id\":1,\"type\":\"podcast\"}}")</script>"#;
        assert_eq!(
            preloads(page, "x").unwrap(),
            serde_json::json!({"post": {"id": 1, "type": "podcast"}})
        );
    }
}
