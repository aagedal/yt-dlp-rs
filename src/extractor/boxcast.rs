//! Upstream `BoxCastVideoIE` (`yt_dlp/extractor/boxcast.py`): BoxCast
//! broadcasts from the page's `BOXCAST_PRELOAD` (else the broadcasts API):
//! a recorded view's HLS playlist with the broadcast's metadata and the
//! page's meta-tag fallbacks; BoxCast embed iframes are embeds.
use super::{Extractor, dict_info, download_json, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct BoxCastVideo;

pub(crate) const NAME: &str = "BoxCastVideo";
pub(crate) const KEY: &str = "BoxCastVideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://boxcast\.tv/(?:view-embed/|channel/\w+\?(?:[^#]+&)?b=|video-portal/(?:\w+/){2})(?P<id>[\w-]+)",
    )
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<iframe[^>]+src=["'](?P<url>https?://boxcast\.tv/view-embed/[\w-]+)"#).unwrap()
});
static PRELOAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s*BOXCAST_PRELOAD\s*=\s*").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Ok(embed) = url.join(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

/// `_html_search_meta(names, webpage)`: the first name present, else a
/// warning naming the first.
fn search_meta(webpage: &str, names: &[&str]) -> Option<String> {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name));
    if found.is_none() {
        super::search_warning(NAME, names[0]);
    }
    found
}

fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| super::nuxt::truthy(value))
}

impl Extractor for BoxCastVideo {
    fn key(&self) -> &'static str {
        KEY
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
        let preload = super::search_json(&PRELOAD, &webpage, true)
            .flatten()
            .unwrap_or(Value::Null);
        let broadcast = match truthy(preload.pointer("/broadcast/data")) {
            Some(broadcast) => broadcast.clone(),
            None => download_json(
                client,
                &Url::parse(&format!("https://api.boxcast.com/broadcasts/{display_id}"))?,
                NAME,
                &display_id,
            )?,
        };
        let view = match truthy(preload.pointer("/view/data")) {
            Some(view) => view.clone(),
            // `_download_json(..., fatal=False) or {}`.
            None => match download_json(
                client,
                &Url::parse(&format!(
                    "https://api.boxcast.com/broadcasts/{display_id}/view"
                ))?,
                NAME,
                &display_id,
            ) {
                Ok(view) => view,
                Err(Error::ExtractorRequest { source, .. }) => {
                    crate::screen::warning(&format!(
                        "[{NAME}] {display_id}: Unable to download JSON metadata: {}",
                        crate::upstream_error_text(&source)
                    ));
                    Value::Null
                }
                Err(error) => return Err(error),
            },
        };
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        if view.get("status").and_then(Value::as_str) == Some("recorded") {
            let manifest = match view.get("playlist") {
                Some(Value::String(playlist)) => Url::parse(playlist).map_err(|_| {
                    Error::UnsupportedHtml5(
                        "BoxCast playlists that are not URLs are not supported natively".into(),
                    )
                })?,
                _ => {
                    return Err(Error::Reported(format!(
                        "{display_id}: An extractor error has occurred. (caused by KeyError('playlist')); {}",
                        crate::BUG_REPORTS_MESSAGE
                    )));
                }
            };
            let mut stream =
                extract_m3u8_fatal_master(client, &manifest, NAME, &display_id, None, true)?;
            super::hls::without_ext(&mut stream);
            formats = stream.formats;
            subtitles = stream.subtitles;
        }
        let id = match broadcast.get("id") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "BoxCast IDs that are not strings are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "{display_id}: An extractor error has occurred. (caused by KeyError('id')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let or_meta = |key: &str, names: &[&str]| {
            truthy(broadcast.get(key))
                .cloned()
                .or_else(|| search_meta(&webpage, names).map(Value::from))
        };
        let title = or_meta("name", &["og:title", "twitter:title"]);
        let description = or_meta("description", &["og:description", "twitter:description"])
            .filter(super::nuxt::truthy);
        let thumbnail = or_meta("preview", &["og:image", "twitter:image"]);
        let get = |key: &str| broadcast.get(key).cloned().unwrap_or(Value::Null);
        let timestamp = broadcast
            .get("streamed_at")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, false))
            .map_or(Value::Null, Value::from);
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", id.into()),
                ("title", title.unwrap_or(Value::Null)),
                ("description", description.unwrap_or(Value::Null)),
                ("thumbnail", thumbnail.unwrap_or(Value::Null)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("release_timestamp", timestamp),
                ("uploader", get("account_name")),
                ("uploader_id", get("account_id")),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://boxcast.tv/view-embed/in-the-midst-of-darkness-light-prevails-an-interfaith-vigil-for-peace-ys0ryhjsjgw2abbgdhx3"
        ));
        assert!(suitable(
            "https://boxcast.tv/channel/z03fqwaeaby5lnaawox2?b=ssihlw5gvfij2by8tkev"
        ));
        assert!(suitable(
            "https://boxcast.tv/video-portal/vctwevwntun3o0ikq7af/rvyblnn0fxbfjx5nwxhl/otbpltj2kzkveo2qz3ad"
        ));
        assert!(!suitable("https://boxcast.tv/channel/x"));
    }
}
