//! Upstream `YouJizzIE` (`yt_dlp/extractor/youjizz.py`): YouJizz videos from
//! the page's `encodings` JSON (HLS playlists and progressive files), else
//! its first HTML5 media entry, with the `<title>`, runtime, uploader and
//! RTA age limit.
use super::{Extractor, dict_format, dict_info, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct YouJizz;

const NAME: &str = "YouJizz";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:\w+\.)?youjizz\.com/videos/(?:[^/#?]*-(?P<id>\d+)\.html|embed/(?P<embed_id>\d+))",
    )
    .unwrap()
});
static ENCODINGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[Ee]ncodings\s*=\s*(\[.+?\]);\n").unwrap());
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)[pP]").unwrap());
static RUNTIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<strong>Runtime:</strong>([^<]+)").unwrap());
static UPLOADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<strong>Uploaded By:.*?<a[^>]*>([^<]+)").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("YouJizz {what} are not supported natively"))
}

impl Extractor for YouJizz {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found
            .name("id")
            .or_else(|| found.name("embed_id"))
            .map(|id| id.as_str().to_owned())
            .unwrap_or_default();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let mut webpage = download_webpage(client, url, NAME, &video_id)?;
        let title = crate::html::html_extract_title(&webpage);
        if title.is_none() {
            search_warning(NAME, "title");
        }
        let encodings = match ENCODINGS.captures(&webpage) {
            Some(found) => serde_json::from_str::<Value>(&found[1])
                .map_err(|_| unsupported("encodings that are not JSON"))?,
            None => Value::Array(Vec::new()),
        };
        let Value::Array(encodings) = encodings else {
            return Err(unsupported("encodings that are not lists"));
        };
        let mut formats = Vec::new();
        for encoding in &encodings {
            let Value::Object(encoding) = encoding else {
                continue;
            };
            let Some(format_url) = encoding
                .get("filename")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            if super::direct::determine_ext(&format_url).as_deref() == Some("m3u8") {
                if let Ok(manifest) = Url::parse(&super::proto_relative_http(&format_url))
                    && let Some(stream) = super::extract_m3u8_master_optional(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some("hls"),
                        false,
                        false,
                    )
                {
                    formats.extend(stream.formats);
                }
                continue;
            }
            // `encoding.get('name') or encoding.get('quality')`.
            let format_id = ["name", "quality"]
                .iter()
                .filter_map(|key| encoding.get(*key))
                .find(|value| super::nuxt::truthy(value));
            let Some(Value::String(format_id)) = format_id else {
                return Err(unsupported("encodings without a name"));
            };
            let height = HEIGHT
                .captures(format_id)
                .and_then(|found| found[1].parse::<i64>().ok());
            formats.push(dict_format(
                vec![
                    ("url", super::proto_relative_http(&format_url).into()),
                    ("format_id", format_id.as_str().into()),
                    ("height", height.map_or(Value::Null, Value::from)),
                ],
                NAME,
                &video_id,
            )?);
        }
        let mut fields: Vec<(&str, Value)> = vec![("formats", Value::Null)];
        let mut subtitles = None;
        if formats.is_empty() {
            // YouJizz's HTML5 player has invalid HTML.
            webpage = webpage.replace("\"controls", "\" controls");
            let entry = super::html5::parse_html5_media_entries(
                client, url, &webpage, &video_id, NAME, None, None,
            )?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
            formats = entry.formats;
            subtitles = Some(entry.subtitles);
            fields.push(("subtitles", Value::Null));
            if let Some(thumbnail) = entry.thumbnail {
                fields.push(("thumbnail", thumbnail.into()));
            }
        }
        let duration = RUNTIME
            .captures(&webpage)
            .and_then(|found| crate::parse_duration(&found[1]))
            .map(|duration| match duration {
                crate::info::PyNumber::Int(value) => Value::from(value),
                crate::info::PyNumber::Float(value) => Value::from(value),
            });
        let uploader = UPLOADER.captures(&webpage).map(|found| found[1].to_owned());
        fields.extend([
            ("id", video_id.into()),
            ("title", title.map_or(Value::Null, Value::from)),
            (
                "age_limit",
                crate::html::rta_search(&webpage).map_or(Value::Null, Value::from),
            ),
            ("duration", duration.unwrap_or(Value::Null)),
            ("uploader", uploader.map_or(Value::Null, Value::from)),
        ]);
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        if let Some(subtitles) = subtitles {
            info.subtitles = subtitles;
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.youjizz.com/videos/zeichentrick-1-2189178.html"
        ));
        assert!(suitable("http://www.youjizz.com/videos/-2189178.html"));
        assert!(suitable("https://www.youjizz.com/videos/embed/31991001"));
        assert!(!suitable("https://www.youjizz.com/videos/2189178.html"));
    }
}
