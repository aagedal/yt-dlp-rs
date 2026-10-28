//! Upstream `RudoVideoIE` (`yt_dlp/extractor/rudovideo.py`): Rudo videos,
//! podcasts and live streams from the page's `streamURL` (else its first
//! `<source>`), with an access token from the token array when present,
//! as HLS, MP3 or a direct file; YouTube players are followed, and Rudo
//! iframes on other pages are embeds.
use super::{Extractor, dict_format, dict_info, download_json_note, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct RudoVideo;

pub(crate) const NAME: &str = "RudoVideo";
pub(crate) const KEY: &str = "RudoVideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://rudo\.video/(?P<type>vod|podcast|live)/(?P<id>[^/?&#]+)").unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+src=['"](?P<url>(?:https?:)//rudo\.video/(?:vod|podcast|live)/[^'"]+)"#,
    )
    .unwrap()
});
static STREAM_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+streamURL\s*=\s*['"]([^?'"]+)"#).unwrap());
static SOURCE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<source[^>]+src=['"]([^'"]+)"#).unwrap());
static YOUTUBE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"file:\s*['"]((?:https?:)//(?:www\.)?youtube\.com[^'"]+)"#).unwrap()
});
static TOKEN_ARRAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<script>var\s+_\$_[a-zA-Z0-9]+\s*=\s*(\[.+\])").unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+titleVideo\s*=\s*['"]([^'"]+)"#).unwrap());
static AUTHOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+videoAuthor\s*=\s*['"]([^?'"]+)"#).unwrap());
static POSTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+posterIMG\s*=\s*['"]([^?'"]+)"#).unwrap());

/// `_match_valid_url`.
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
        let raw = crate::html::unescape_html(&captures["url"]);
        let Ok(embed) = url.join(&raw) else {
            continue;
        };
        if !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn first(regex: &Regex, webpage: &str) -> Option<String> {
    regex.captures(webpage).map(|found| found[1].to_owned())
}

fn reported(video_id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {video_id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_search_json(..., contains_pattern=r'\[(?s:.+)\]', default=None,
/// transform_source=js_to_json)`: the token array's first JSON value.
fn token_array(webpage: &str) -> Option<Value> {
    let candidate = TOKEN_ARRAY.captures(webpage)?.get(1)?.as_str();
    let text = crate::js_to_json::js_to_json(candidate);
    serde_json::Deserializer::from_str(&crate::json::replace_lone_surrogates(&text))
        .into_iter::<Value>()
        .next()
        .and_then(std::result::Result::ok)
}

impl Extractor for RudoVideo {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let is_live = &found["type"] == "live";
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        if webpage.contains("Streaming is not available in your area") {
            return Err(crate::geo::restricted(
                &format!(
                    "[{NAME}] {video_id}: This video is not available from your location due to geo restriction"
                ),
                &[],
            ));
        }
        let media_url = first(&STREAM_URL, &webpage)
            .filter(|url| !url.is_empty())
            .or_else(|| first(&SOURCE_URL, &webpage))
            .filter(|url| !url.is_empty());
        let Some(mut media_url) = media_url else {
            if let Some(youtube_url) = first(&YOUTUBE_URL, &webpage) {
                // `url_result(youtube_url, 'Youtube')`.
                let mut follow = super::direct::build_at(url.clone(), url, String::new());
                follow.formats.clear();
                follow.direct = None;
                follow
                    .extra
                    .insert(crate::FOLLOW_URL.into(), youtube_url.into());
                return Ok(Some(follow));
            }
            return Err(reported(&video_id, "Unable to extract stream url"));
        };
        if let Some(tokens) = token_array(&webpage).filter(super::nuxt::truthy) {
            // `traverse_obj(token_array, (..., {url_or_none}), get_all=False)`.
            let items: Vec<&Value> = match &tokens {
                Value::Array(items) => items.iter().collect(),
                Value::Object(items) => items.values().collect(),
                _ => Vec::new(),
            };
            let token_url = items
                .into_iter()
                .filter_map(Value::as_str)
                .find_map(super::json_ld::url_or_none)
                .ok_or_else(|| reported(&video_id, "Invalid access token array"))?;
            let token_url = Url::parse(&token_url)?;
            let response = download_json_note(
                client,
                &token_url,
                NAME,
                &video_id,
                "Downloading access token",
            )?;
            let key_error = |key: &str| {
                Error::Reported(format!(
                    "{video_id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            };
            let token = response
                .get("data")
                .ok_or_else(|| key_error("data"))?
                .get("authToken")
                .ok_or_else(|| key_error("authToken"))?;
            let token = match token {
                Value::String(token) => token.clone(),
                Value::Number(token) => token.to_string(),
                _ => {
                    return Err(Error::UnsupportedHtml5(
                        "Rudo access tokens that are not strings are not supported natively".into(),
                    ));
                }
            };
            let parsed = Url::parse(&media_url).map_err(|_| {
                Error::UnsupportedHtml5(
                    "Rudo stream URLs that are not URLs are not supported natively".into(),
                )
            })?;
            media_url =
                super::update_url_query(&parsed, &[("auth-token", token.as_str())]).to_string();
        }
        let ext = super::direct::determine_ext(&media_url);
        let formats = if ext.as_deref() == Some("m3u8") {
            let manifest = Url::parse(&media_url).map_err(|_| {
                Error::UnsupportedHtml5(
                    "Rudo playlist URLs that are not URLs are not supported natively".into(),
                )
            })?;
            let mut stream = if is_live {
                super::extract_m3u8_fatal_live(client, &manifest, NAME, &video_id, None, false)?
            } else {
                super::extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, false)?
            };
            super::hls::without_ext(&mut stream);
            stream.formats
        } else {
            let mut fields = vec![("url", Value::from(media_url.clone()))];
            if ext.as_deref() == Some("mp3") {
                fields.push(("vcodec", "none".into()));
            }
            vec![dict_format(fields, NAME, &video_id)?]
        };
        let search = |value: Option<String>, name: &str| {
            if value.is_none() {
                super::search_warning(NAME, name);
            }
            value
        };
        let title = first(&TITLE, &webpage).or_else(|| {
            search(
                crate::html::og_search_property(&webpage, "title"),
                "OpenGraph title",
            )
        });
        let thumbnail = first(&POSTER, &webpage).or_else(|| {
            search(
                crate::html::og_search_property(&webpage, "image"),
                "thumbnail URL",
            )
        });
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", optional(title)),
                ("creator", optional(first(&AUTHOR, &webpage))),
                ("thumbnail", optional(thumbnail)),
                ("formats", Value::Null),
                ("is_live", is_live.into()),
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
        assert!(suitable("https://rudo.video/vod/bN5AaJ"));
        assert!(suitable("https://rudo.video/live/bbtv"));
        assert!(!suitable("https://rudo.video/other/x"));
        assert_eq!(
            temp_id("https://rudo.video/podcast/b42ZUznHX0").as_deref(),
            Some("b42ZUznHX0")
        );
    }
}
