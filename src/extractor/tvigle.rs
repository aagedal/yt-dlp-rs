//! Upstream `TvigleIE` (`yt_dlp/extractor/tvigle.py`): Tvigle videos (the
//! page's player ID, else the cloud URL's) from the cloud `play/video` API:
//! its HLS, DASH and progressive files, the API's geo and other errors, and
//! the item metadata; cloud player iframes on other pages are followed.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Tvigle;

pub(crate) const NAME: &str = "tvigle";
pub(crate) const KEY: &str = "Tvigle";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:tvigle\.ru/(?:[^/]+/)+(?P<display_id>[^/]+)/$|cloud\.tvigle\.ru/video/(?P<id>\d+))",
    )
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+?src=(?:"(?P<dq>(?:https?:)?//cloud\.tvigle\.ru/video/[^"\n]+?)"|'(?P<sq>(?:https?:)?//cloud\.tvigle\.ru/video/[^'\n]+?)')"#,
    )
    .unwrap()
});
static PLAYER_IDS: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r#"<div[^>]+class=["']player["'][^>]+id=["'](\d+)"#).unwrap(),
        Regex::new(r#"cloudId\s*=\s*["'](\d+)"#).unwrap(),
        Regex::new(r#"class="video-preview current_playing" id="(\d+)""#).unwrap(),
    ]
});
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)[pP]$").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`: the cloud ID (`None` for page URLs).
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL
        .captures(url)
        .and_then(|found| found.name("id").map(|id| id.as_str().to_owned()))
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Some(raw) = captures.name("dq").or_else(|| captures.name("sq")) else {
            continue;
        };
        let Ok(embed) = url.join(&crate::html::unescape_html(raw.as_str())) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Tvigle {what} are not supported natively"))
}

/// An error raised without a video ID: prefixed with the temporary ID.
fn prefixed(temp: Option<&str>, message: &str) -> String {
    match temp {
        Some(id) => format!("[{NAME}] {id}: {message}"),
        None => format!("[{NAME}] {message}"),
    }
}

fn key_error(temp: Option<&str>, key: &str) -> Error {
    let message = format!(
        "An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    );
    Error::Reported(match temp {
        Some(id) => format!("{id}: {message}"),
        None => message,
    })
}

fn url_or_none(value: &Value) -> Option<String> {
    value.as_str().and_then(super::json_ld::url_or_none)
}

impl Extractor for Tvigle {
    fn key(&self) -> &'static str {
        KEY
    }

    /// `_GEO_BYPASS = False`, `_GEO_COUNTRIES = ['RU']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&["RU"])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let temp = found.name("id").map(|id| id.as_str().to_owned());
        let display_id = found.name("display_id").map(|id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = match &temp {
            Some(id) => id.clone(),
            None => {
                let page_id = display_id.clone().unwrap_or_default();
                let webpage = download_webpage(client, url, NAME, &page_id)?;
                PLAYER_IDS
                    .iter()
                    .find_map(|pattern| pattern.captures(&webpage).map(|found| found[1].to_owned()))
                    .ok_or_else(|| {
                        Error::Reported(prefixed(
                            None,
                            &format!("Unable to extract video id; {}", crate::BUG_REPORTS_MESSAGE),
                        ))
                    })?
            }
        };
        let api = Url::parse(&format!(
            "http://cloud.tvigle.ru/api/play/video/{video_id}/"
        ))?;
        let message_id = display_id.clone().unwrap_or_else(|| "None".into());
        let video_data = download_json(client, &api, NAME, &message_id)?;
        let item = match video_data.pointer("/playlist/items") {
            // An uncaught `IndexError` is reported as its message.
            Some(Value::Array(items)) => items
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            Some(_) => return Err(unsupported("playlist items that are not lists")),
            None => {
                let key = if video_data.get("playlist").is_none() {
                    "playlist"
                } else {
                    "items"
                };
                return Err(key_error(temp.as_deref(), key));
            }
        };
        let videos = item.get("videos");
        let error_message = item
            .get("errorMessage")
            .filter(|value| super::nuxt::truthy(value));
        if !videos.is_some_and(super::nuxt::truthy)
            && let Some(error_message) = error_message
        {
            let error_message = error_message
                .as_str()
                .ok_or_else(|| unsupported("error messages that are not strings"))?;
            if item.get("isGeoBlocked") == Some(&Value::Bool(true)) {
                return Err(crate::geo::restricted(
                    &prefixed(temp.as_deref(), error_message),
                    &["RU"],
                ));
            }
            return Err(Error::Reported(prefixed(
                temp.as_deref(),
                &format!("{NAME} returned error: {error_message}"),
            )));
        }
        let title = item
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(temp.as_deref(), "title"))?;
        let duration = match item.get("durationMilliseconds") {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse().ok(),
            _ => None,
        }
        .map(|milliseconds| milliseconds / 1000.0);
        let videos = match item.get("videos") {
            Some(Value::Object(videos)) => videos.clone(),
            Some(_) => return Err(unsupported("video maps that are not objects")),
            None => return Err(key_error(temp.as_deref(), "videos")),
        };
        let mut formats = Vec::new();
        for (vcodec, url_or_fmts) in &videos {
            match vcodec.as_str() {
                "hls" => {
                    let Some(m3u8_url) = url_or_none(url_or_fmts) else {
                        continue;
                    };
                    if let Some(stream) = super::extract_m3u8_master_optional(
                        client,
                        &Url::parse(&m3u8_url)?,
                        NAME,
                        &video_id,
                        Some("hls"),
                        false,
                        false,
                    ) {
                        formats.extend(stream.formats);
                    }
                }
                "dash" => {
                    let Some(mpd_url) = url_or_none(url_or_fmts) else {
                        continue;
                    };
                    if let Some(mut stream) = super::extract_mpd_optional_note(
                        client,
                        &Url::parse(&mpd_url)?,
                        NAME,
                        &video_id,
                        Some("dash"),
                        "Downloading MPD manifest",
                    ) {
                        super::ignore_subtitles(&mut stream, NAME, "DASH");
                        formats.extend(stream.formats);
                    }
                }
                _ => {
                    let Some(files) = url_or_fmts.as_object() else {
                        continue;
                    };
                    for (format_id, video_url) in files {
                        if format_id == "m3u8" {
                            continue;
                        }
                        let Some(video_url) = url_or_none(video_url) else {
                            continue;
                        };
                        let height = HEIGHT
                            .captures(format_id)
                            .and_then(|found| found[1].parse::<i64>().ok());
                        let filesize = match item
                            .pointer(&format!("/video_files_size/{vcodec}/{format_id}"))
                        {
                            Some(Value::Number(number)) => number.as_i64(),
                            Some(Value::String(text)) => text.trim().parse().ok(),
                            _ => None,
                        };
                        formats.push(dict_format(
                            vec![
                                ("url", video_url.into()),
                                ("format_id", format!("{vcodec}-{format_id}").into()),
                                ("vcodec", vcodec.clone().into()),
                                ("height", height.map_or(Value::Null, Value::from)),
                                ("filesize", filesize.map_or(Value::Null, Value::from)),
                            ],
                            NAME,
                            &video_id,
                        )?);
                    }
                }
            }
        }
        let get = |key: &str| item.get(key).cloned().unwrap_or(Value::Null);
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("display_id", display_id.map_or(Value::Null, Value::from)),
                ("title", title),
                ("description", get("description")),
                ("thumbnail", get("thumbnail")),
                (
                    "duration",
                    duration.map_or(Value::Null, |duration| json!(duration)),
                ),
                (
                    "age_limit",
                    super::videopress::parse_age_limit(item.get("ageRestrictions")),
                ),
                ("formats", Value::Null),
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
        assert!(suitable("http://www.tvigle.ru/video/sokrat/"));
        assert!(suitable("https://cloud.tvigle.ru/video/5267604/"));
        assert!(!suitable("http://www.tvigle.ru/video/sokrat"));
        assert_eq!(
            temp_id("https://cloud.tvigle.ru/video/5267604/").as_deref(),
            Some("5267604")
        );
    }
}
