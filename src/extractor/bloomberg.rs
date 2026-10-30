//! Upstream `BloombergIE` (`yt_dlp/extractor/bloomberg.py`): Bloomberg pages
//! from the page's `bmmrId`, `videoId`, `data-bmmrid` or `BPlayer(...)`
//! video ID and its multimedia embed API: TS streams as HLS (`hls`), others
//! as HDS (`hds`), with the OpenGraph title (without `: Video`),
//! description and thumbnail.
use super::{Extractor, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Bloomberg;

const NAME: &str = "Bloomberg";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?bloomberg\.com/(?:[^/]+/)*(?P<id>[^/?#]+)")
        .unwrap()
});
// `(["\'])(?P<id>(?:(?!\1).)+)\1` for each quote.
static VIDEO_ID: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        r#"["']bmmrId["']\s*:\s*"#,
        r#"videoId\s*:\s*"#,
        r#"data-bmmrid="#,
    ]
    .map(|prefix| {
        Regex::new(&format!(
            r#"{prefix}(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')"#
        ))
        .unwrap()
    })
});
static BPLAYER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"BPlayer\(null,\s*(\{[^;]+\})\);").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Bloomberg {what} are not supported natively"))
}

/// `_og_search_property(prop, webpage)` (non-fatal: a warning when absent).
fn og_search(webpage: &str, prop: &str, name: &str) -> Value {
    crate::html::og_search_property(webpage, prop).map_or_else(
        || {
            super::search_warning(NAME, name);
            Value::Null
        },
        Value::from,
    )
}

/// The page's video ID: the first pattern that matches, else the
/// `BPlayer(null, {...});` data's `id`.
fn video_id(webpage: &str, name: &str) -> Result<String> {
    for regex in VIDEO_ID.iter() {
        if let Some(found) = regex.captures(webpage) {
            let id = found.name("dq").or_else(|| found.name("sq")).unwrap();
            return Ok(id.as_str().to_owned());
        }
    }
    let Some(found) = BPLAYER.captures(webpage) else {
        return Err(Error::Reported(format!(
            "[{NAME}] {name}: Unable to extract id; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let text = &found[1];
    let data: Value = crate::json::from_str(text).map_err(|_| {
        let cause = crate::json::python_decode_error_text(text).map_or_else(String::new, |error| {
            format!(
                " (caused by JSONDecodeError({}))",
                crate::batch::py_repr(&error)
            )
        });
        Error::Reported(format!(
            "[{NAME}] {name}: Failed to parse JSON{cause}; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    let Value::Object(data) = data else {
        return Err(unsupported("player data that are not objects"));
    };
    match data.get("id") {
        Some(Value::String(id)) => Ok(id.clone()),
        Some(_) => Err(unsupported("video IDs that are not strings")),
        None => Err(key_error(name, "id")),
    }
}

/// `re.sub(': Video$', '', title)` (`$` also matches before a final newline).
fn strip_video_suffix(title: &str) -> String {
    if let Some(rest) = title.strip_suffix(": Video") {
        rest.to_owned()
    } else if let Some(rest) = title.strip_suffix(": Video\n") {
        format!("{rest}\n")
    } else {
        title.to_owned()
    }
}

impl Extractor for Bloomberg {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let name = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &name)?;
        let video_id = video_id(&webpage, &name)?;
        // `re.sub` of a missing (`None`) title fails like upstream.
        let title = match og_search(&webpage, "title", "OpenGraph title") {
            Value::String(title) => strip_video_suffix(&title),
            _ => {
                return Err(Error::Reported(
                    "expected string or bytes-like object, got 'NoneType'".into(),
                ));
            }
        };
        let embed = Url::parse(&format!(
            "http://www.bloomberg.com/multimedia/api/embed?id={video_id}"
        ))?;
        let embed_info = download_json(client, &embed, NAME, &video_id)?;
        let streams = match embed_info.get("streams") {
            Some(Value::Array(streams)) => streams,
            Some(_) => return Err(unsupported("stream lists that are not arrays")),
            None => return Err(key_error(&name, "streams")),
        };
        let mut formats = Vec::new();
        for stream in streams {
            let Value::Object(stream) = stream else {
                return Err(unsupported("streams that are not objects"));
            };
            let stream_url = match stream.get("url") {
                None | Some(Value::Null) => continue,
                Some(Value::String(stream_url)) if stream_url.is_empty() => continue,
                Some(Value::String(stream_url)) => stream_url,
                Some(_) => return Err(unsupported("stream URLs that are not strings")),
            };
            let muxing_format = stream
                .get("muxing_format")
                .ok_or_else(|| key_error(&name, "muxing_format"))?;
            let manifest = Url::parse(stream_url)
                .map_err(|_| unsupported("stream URLs that are not absolute URLs"))?;
            if muxing_format.as_str() == Some("TS") {
                if let Some(stream) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("hls"),
                    false,
                    false,
                ) {
                    formats.extend(stream.formats);
                }
            } else {
                formats.extend(super::extract_f4m_optional(
                    client,
                    NAME,
                    &manifest,
                    &video_id,
                    Some("hds"),
                )?);
            }
        }
        let description = og_search(&webpage, "description", "OpenGraph description");
        let thumbnail = og_search(&webpage, "image", "thumbnail URL");
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("formats", Value::Null),
                ("description", description),
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
        let id = |url: &str| VALID_URL.captures(url).map(|found| found["id"].to_owned());
        assert_eq!(
            id("http://www.bloomberg.com/features/2016-hello-world-new-zealand/").as_deref(),
            Some("2016-hello-world-new-zealand")
        );
        assert_eq!(
            id("https://www.bloomberg.com/news/videos/2021-09-14/a-video?x=1").as_deref(),
            Some("a-video")
        );
        assert!(!suitable("https://bloomberg.com/"));
    }

    #[test]
    fn video_suffix_is_removed_at_the_end() {
        assert_eq!(strip_video_suffix("A: Video"), "A");
        assert_eq!(strip_video_suffix("A: Video\n"), "A\n");
        assert_eq!(strip_video_suffix("A: Videos"), "A: Videos");
    }
}
