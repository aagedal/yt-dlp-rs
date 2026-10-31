//! Upstream `MassengeschmackTVIE` (`yt_dlp/extractor/massengeschmacktv.py`):
//! massengeschmack.tv episodes from the page's `MEDIA` sources (HLS
//! playlists or direct files) and its download links (resolution and file
//! size), with the clip title and `POSTER` thumbnail.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MassengeschmackTv;

const NAME: &str = "massengeschmack.tv";
const KEY: &str = "MassengeschmackTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?massengeschmack\.tv/play/(?P<id>[^?&#]+)")
        .unwrap()
});

static MEDIA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)MEDIA\s*=\s*(\[.+?\]);").unwrap());

static DOWNLOADS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"<a[^>]+?href="(?P<url>(?:https:)?//[^"]+)".*?"#,
        r"<strong>(?P<format_id>.+?)</strong>.*?",
        r"<small>(?:(?P<width>\d+)x(?P<height>\d+))?\s+?\((?P<filesize>[\d,]+\s*[GM]iB)\)</small>",
    ))
    .unwrap()
});

static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<span[^>]+\bid=["']clip-title["'][^>]*>([^<]+)"#).unwrap());

static POSTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"POSTER\s*=\s*"([^"]+)"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!(
        "massengeschmack.tv {what} are not supported natively"
    ))
}

/// `determine_ext(url)` with its `unknown_video` default.
fn determine_ext(url: &str) -> String {
    super::direct::determine_ext(url).unwrap_or_else(|| "unknown_video".into())
}

/// A format URL as `sanitize_url` makes it (`//` URLs as `http:`).
fn format_url(url: &str) -> Result<Url> {
    Url::parse(&super::proto_relative_http(url))
        .map_err(|_| unsupported("media URLs that are not URLs"))
}

impl Extractor for MassengeschmackTv {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let episode = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &episode)?;
        let media = MEDIA.captures(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {episode}: Unable to extract media; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let sources: Value = serde_json::from_str(&crate::js_to_json::js_to_json(&media[1]))
            .map_err(|error| {
                Error::Reported(format!(
                    "[{NAME}] {episode}: Failed to parse JSON  (caused by {error})"
                ))
            })?;
        let Value::Array(sources) = sources else {
            return Err(unsupported("media lists that are not lists"));
        };
        let mut formats = Vec::new();
        for source in &sources {
            let Value::Object(source) = source else {
                return Err(unsupported("media sources that are not objects"));
            };
            let furl = match source.get("src") {
                None | Some(Value::Null) => continue,
                Some(Value::String(furl)) if furl.is_empty() => continue,
                // `_proto_relative_url`: `http_scheme()`, `https:`.
                Some(Value::String(furl)) => super::proto_relative(furl),
                Some(_) => return Err(unsupported("media URLs that are not strings")),
            };
            // `determine_ext(furl) or mimetype2ext(...)`: never empty.
            let ext = determine_ext(&furl);
            if ext == "m3u8" {
                let manifest = format_url(&furl)?;
                if let Some(stream) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &episode,
                    Some("hls"),
                    false,
                    false,
                ) {
                    formats.extend(stream.formats);
                }
            } else {
                formats.push(dict_format(
                    vec![
                        ("url", format_url(&furl)?.as_str().into()),
                        ("format_id", ext.into()),
                    ],
                    NAME,
                    &episode,
                )?);
            }
        }
        for download in DOWNLOADS.captures_iter(&webpage) {
            let format_id = download["format_id"].to_owned();
            let int = |name: &str| {
                download
                    .name(name)
                    .and_then(|value| value.as_str().parse::<i64>().ok())
                    .map_or(Value::Null, Value::from)
            };
            let vcodec = if format_id.starts_with("Audio") {
                Value::from("none")
            } else {
                Value::Null
            };
            formats.push(dict_format(
                vec![
                    ("url", format_url(&download["url"])?.as_str().into()),
                    ("format_id", format_id.into()),
                    ("width", int("width")),
                    ("height", int("height")),
                    (
                        "filesize",
                        super::xboxclips::parse_filesize(&download["filesize"])
                            .map_or(Value::Null, Value::from),
                    ),
                    ("vcodec", vcodec),
                ],
                NAME,
                &episode,
            )?);
        }
        let title = match TITLE.captures(&webpage) {
            Some(title) => crate::html::clean_html(&title[1]).into(),
            None => {
                super::search_warning(NAME, "title");
                Value::Null
            }
        };
        let thumbnail = match POSTER.captures(&webpage) {
            Some(poster) => poster[1].to_owned().into(),
            None => {
                super::search_warning(NAME, "thumbnail");
                Value::Null
            }
        };
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", episode.into()),
                ("title", title),
                ("formats", Value::Null),
                ("thumbnail", thumbnail),
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
        assert!(suitable("https://massengeschmack.tv/play/fktv202"));
        assert!(!suitable("https://massengeschmack.tv/clip/fktv202"));
    }

    #[test]
    fn download_links_follow_upstream_pattern() {
        let page = r#"<a class="x" href="//cache.test/a.mp4"><strong>1080p</strong> <small>1920x1080 (1,5 GiB)</small></a>"#;
        let found = DOWNLOADS.captures(page).unwrap();
        assert_eq!(&found["url"], "//cache.test/a.mp4");
        assert_eq!(&found["filesize"], "1,5 GiB");
    }
}
