//! Upstream `GameSpotIE` (`yt_dlp/extractor/gamespot.py`): GameSpot video,
//! article and review pages from the page's `data-video` JSON: its adaptive
//! HLS stream (each format followed by an `http-` MP4 copy) and DASH
//! manifest, the unquoted title, the `guid` and the page's description and
//! OpenGraph thumbnail.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct GameSpot;

const NAME: &str = "GameSpot";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?gamespot\.com/(?:video|article|review)s/(?:[^/]+/\d+-|embed/)(?P<id>\d+)",
    )
    .unwrap()
});
// `data-video=(["\'])({.*?})\1`.
static DATA_VIDEO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-video=(?:"(?P<dq>\{.*?\})"|'(?P<sq>\{.*?\})')"#).unwrap());

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
    Error::UnsupportedHtml5(format!("GameSpot {what} are not supported natively"))
}

/// `self._parse_json(self._html_search_regex(..., 'video data', group=2),
/// page_id)`.
fn data_video(webpage: &str, page_id: &str) -> Result<Value> {
    let Some(found) = DATA_VIDEO.captures(webpage) else {
        return Err(Error::Reported(format!(
            "[{NAME}] {page_id}: Unable to extract video data; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let raw = found.name("dq").or_else(|| found.name("sq")).unwrap();
    let text = crate::html::clean_html(raw.as_str());
    crate::json::from_str(&text).map_err(|_| {
        let cause =
            crate::json::python_decode_error_text(&text).map_or_else(String::new, |error| {
                format!(
                    " (caused by JSONDecodeError({}))",
                    crate::batch::py_repr(&error)
                )
            });
        Error::Reported(format!(
            "[{NAME}] {page_id}: Failed to parse JSON{cause}; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })
}

/// A stream URL from `streams.get(KEY)` (falsy values are absent).
fn stream_url(streams: &serde_json::Map<String, Value>, key: &str) -> Result<Option<Url>> {
    match streams.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(url)) if url.is_empty() => Ok(None),
        Some(Value::String(url)) => Url::parse(url)
            .map(Some)
            .map_err(|_| unsupported("stream URLs that are not absolute URLs")),
        Some(_) => Err(unsupported("stream URLs that are not strings")),
    }
}

impl Extractor for GameSpot {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let page_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &page_id)?;
        let data_video = data_video(&webpage, &page_id)?;
        let Value::Object(data_video) = data_video else {
            return Err(unsupported("video data that are not objects"));
        };
        // `urllib.parse.unquote(data_video['title'])`.
        let title = match data_video.get("title") {
            Some(Value::String(title)) => percent_encoding::percent_decode_str(title)
                .decode_utf8_lossy()
                .into_owned(),
            Some(_) => return Err(unsupported("titles that are not strings")),
            None => return Err(key_error(&page_id, "title")),
        };
        let streams = match data_video.get("videoStreams") {
            Some(Value::Object(streams)) => streams,
            Some(_) => return Err(unsupported("stream lists that are not objects")),
            None => return Err(key_error(&page_id, "videoStreams")),
        };
        let mut formats = Vec::new();
        if let Some(manifest) = stream_url(streams, "adaptive_stream")?
            && let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &page_id,
                Some("hls"),
                false,
                false,
            )
        {
            for format in stream.formats {
                // `del http_f['manifest_url']`: a media playlist's format
                // has none.
                if !format.extra.contains_key("manifest_url") {
                    return Err(key_error(&page_id, "manifest_url"));
                }
                let mut http = format.clone();
                http.extra.shift_remove("manifest_url");
                http.format_id = http.format_id.replace("hls-", "http-");
                http.protocol = Some("http".into());
                http.url = Url::parse(&format.url.as_str().replace(".m3u8", ".mp4"))?;
                http.fragments = Vec::new();
                formats.push(format);
                formats.push(http);
            }
        }
        if let Some(manifest) = stream_url(streams, "adaptive_dash")?
            && let Some(stream) = super::extract_mpd_optional_note(
                client,
                &manifest,
                NAME,
                &page_id,
                Some("dash"),
                "Downloading MPD manifest",
            )
        {
            formats.extend(stream.formats);
        }
        let id = match data_video.get("guid") {
            Some(Value::String(guid)) if !guid.is_empty() => guid.clone(),
            None | Some(Value::Null) | Some(Value::String(_)) => page_id.clone(),
            Some(_) => return Err(unsupported("GUIDs that are not strings")),
        };
        let description = crate::html::html_search_meta_clean(&webpage, "description");
        if description.is_none() {
            super::search_warning(NAME, "description");
        }
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            super::search_warning(NAME, "thumbnail URL");
        }
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", id.into()),
                ("display_id", page_id.into()),
                ("title", title.into()),
                ("formats", Value::Null),
                ("description", optional(description)),
                ("thumbnail", optional(thumbnail)),
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
            "http://www.gamespot.com/videos/arma-3-community-guide-sitrep-i/2300-6410818/"
        ));
        assert!(suitable("https://www.gamespot.com/videos/embed/6439218/"));
        assert!(suitable(
            "https://www.gamespot.com/reviews/gears-of-war-review/1900-6161188/"
        ));
        assert!(!suitable("https://www.gamespot.com/videos/"));
    }
}
