//! Upstream `NovaPlayIE` (`yt_dlp/extractor/novaplay.py`): play.nova.bg
//! videos from the page's Next.js `video` props, the client API's access
//! token (cached for the run) and the fite.tv streams API's HLS playlist.
use super::{Extractor, dict_info, download_json_headers, download_json_note, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct NovaPlay;

const NAME: &str = "NovaPlay";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://play\.nova\.bg/video/[^?#]+/(?P<id>\d+)").unwrap()
});

/// The class attribute `_access_token`, kept for later URLs.
static ACCESS_TOKEN: Mutex<Option<String>> = Mutex::new(None);

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
    Error::UnsupportedHtml5(format!("NovaPlay {what} are not supported natively"))
}

fn get<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| key_error(id, key))
}

/// `_og_search_property(prop, webpage, name, fatal=False)`.
fn og_search(webpage: &str, prop: &str, name: &str) -> Value {
    match crate::html::og_search_property(webpage, prop) {
        Some(value) => value.into(),
        None => {
            super::search_warning(NAME, name);
            Value::Null
        }
    }
}

/// `int_or_none(value)`.
fn int_or_none(value: &Value) -> Result<Value> {
    Ok(match value {
        Value::Null => Value::Null,
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Value::String(text) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        Value::Bool(value) => i64::from(*value).into(),
        _ => return Err(unsupported("view counts that are not numbers")),
    })
}

fn access_token(client: &Client) -> Result<String> {
    if let Some(token) = ACCESS_TOKEN.lock().unwrap().clone() {
        return Ok(token);
    }
    let api = Url::parse("https://play.nova.bg/api/client")?;
    let response = download_json_note(client, &api, NAME, "", "Fetching access token")?;
    let token = match response.get("accessToken") {
        Some(Value::String(token)) => token.clone(),
        Some(_) => return Err(unsupported("access tokens that are not strings")),
        None => return Err(key_error("", "accessToken")),
    };
    *ACCESS_TOKEN.lock().unwrap() = Some(token.clone());
    Ok(token)
}

impl Extractor for NovaPlay {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let token = access_token(client).map_err(|error| match error {
            // The KeyError is reported with the URL's ID.
            Error::Reported(text) if text.starts_with(": ") => {
                Error::Reported(format!("{video_id}{text}"))
            }
            error => error,
        })?;
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        let mut video_props = &next_data;
        for key in ["props", "pageProps", "video"] {
            video_props = get(video_props, key, &video_id)?;
        }
        let streams = Url::parse(&format!(
            "https://nbg-api.fite.tv/api/v2/videos/{video_id}/streams"
        ))?;
        let authorization = format!("Bearer {token}");
        let streams = download_json_headers(
            client,
            &streams,
            NAME,
            &video_id,
            &[
                ("x-flipps-user-agent", "Flipps/75/9.7"),
                ("x-flipps-version", "2022-05-17"),
                ("Authorization", &authorization),
            ],
        )?;
        let first = match &streams {
            Value::Array(streams) => streams
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            _ => return Err(unsupported("stream lists that are not lists")),
        };
        let mut href = first;
        for key in ["links", "play", "href"] {
            href = get(href, key, &video_id)?;
        }
        let manifest = match href {
            Value::String(href) => {
                Url::parse(href).map_err(|_| unsupported("playlist URLs that are not URLs"))?
            }
            _ => return Err(unsupported("playlist URLs that are not strings")),
        };
        let stream = super::extract_m3u8_fatal_master(
            client,
            &manifest,
            NAME,
            &video_id,
            Some("hls"),
            false,
        )?;
        let title = get(video_props, "title", &video_id)?.clone();
        let alt_title = video_props.get("slug").cloned().unwrap_or(Value::Null);
        let thumbnail = og_search(&webpage, "image", "thumbnail URL");
        let description = og_search(&webpage, "description", "OpenGraph description");
        // `parse_duration(video_props['duration'])`: only strings parse.
        let duration = match get(video_props, "duration", &video_id)? {
            Value::String(text) => match crate::parse_duration(text) {
                Some(crate::info::PyNumber::Int(seconds)) => Value::from(seconds),
                Some(crate::info::PyNumber::Float(seconds)) => Value::from(seconds),
                None => Value::Null,
            },
            _ => Value::Null,
        };
        let timestamp = match get(video_props, "published_at", &video_id)? {
            Value::Null => Value::Null,
            Value::String(text) => {
                crate::dates::parse_iso8601(text).map_or(Value::Null, Value::from)
            }
            _ => return Err(unsupported("publication dates that are not strings")),
        };
        let view_count = int_or_none(get(video_props, "view_count", &video_id)?)?;
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("alt_title", alt_title),
                ("thumbnail", thumbnail),
                ("description", description),
                ("formats", Value::Null),
                ("duration", duration),
                ("timestamp", timestamp),
                ("view_count", view_count),
            ],
            stream.formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://play.nova.bg/video/ochakvaite/season-0/ochakvaite-2022-07-22-sybudi-se-sat/606627"
        ));
        assert!(!suitable("https://play.nova.bg/video/606627"));
    }
}
