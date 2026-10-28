//! Upstream `RteIE` and `RteRadioIE` (`yt_dlp/extractor/rte.py`, through
//! `RteBaseIE`): the IPTV and web playlist feeds (both read; the first
//! show's metadata, every show's RTMP, HLS and HDS formats) and the web
//! feed's `IE_NAME said: MESSAGE` 404 error.
use super::clyp::{float_or_none, key_error};
use super::{Extractor, dict_format, dict_info};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Rte;
pub struct RteRadio;

static RTE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?rte\.ie/player/[^/]{2,3}/show/[^/]+/(?P<id>[0-9]+)").unwrap()
});
static RADIO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?rte\.ie/radio/utils/radioplayer/rteradioweb\.html#!rii=(?:b?[0-9]*)(?:%3A|:|%5F|_)(?P<id>[0-9]+)",
    )
    .unwrap()
});
static RTMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<url>rtmpe?://[^/]+)/(?P<app>.+)/(?P<playpath>mp4:.*)").unwrap()
});

const ENDPOINTS: [&str; 2] = [
    "https://feeds.rasset.ie/rteavgen/player/playlist?type=iptv&format=json&showId=",
    "http://www.rte.ie/rteavgen/getplaylist/?type=web&format=json&id=",
];

/// `RteIE._match_id`.
pub fn suitable(url: &str) -> bool {
    RTE_URL.is_match(url)
}

/// `RteRadioIE._match_id`.
pub fn radio_suitable(url: &str) -> bool {
    RADIO_URL.is_match(url)
}

impl Extractor for Rte {
    fn key(&self) -> &'static str {
        "Rte"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        extract(client, url, &RTE_URL, ("rte", "Rte"))
    }
}

impl Extractor for RteRadio {
    fn key(&self) -> &'static str {
        "RteRadio"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        extract(client, url, &RADIO_URL, ("rte:radio", "RteRadio"))
    }
}

/// A feed request's outcome: its JSON, or the `ExtractorError` upstream's
/// loop catches (with a 404's body for the `said:` message).
enum Feed {
    Json(Value),
    Failed(Error, Option<String>),
}

/// `_download_json(ep_url + item_id, item_id)`, keeping a 404's body.
fn feed(client: &Client, url: &Url, name: &'static str, id: &str) -> Result<Feed> {
    use std::io::Read;
    crate::screen::screen(&format!("[{name}] {id}: Downloading JSON metadata"));
    let mut response = match super::direct::request_expecting(client, url, &[], &[404]) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            return Ok(Feed::Failed(
                Error::ExtractorRequest {
                    extractor: name,
                    id: id.to_owned(),
                    what: "JSON metadata",
                    source: Box::new(error),
                },
                None,
            ));
        }
        Err(error) => return Err(error),
    };
    let status = response.status();
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    if status.as_u16() == 404 {
        let reason = status.canonical_reason().unwrap_or_default();
        return Ok(Feed::Failed(
            Error::Reported(format!(
                "[{name}] {id}: Unable to download JSON metadata: HTTP Error 404: {reason} (caused by <HTTPError 404: {reason}>)"
            )),
            Some(text),
        ));
    }
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    Ok(match crate::json::from_str(&text) {
        Ok(data) => Feed::Json(data),
        Err(error) => Feed::Failed(
            Error::Reported(format!(
                "[{name}] {id}: Failed to parse JSON  (caused by {error})"
            )),
            None,
        ),
    })
}

/// `unescapeHTML` of a string (`None` stays `None`).
fn unescape(value: Option<&Value>, what: &str) -> Result<Value> {
    match value {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::String(text)) => Ok(crate::html::unescape_html(text).into()),
        Some(_) => Err(Error::UnsupportedHtml5(format!(
            "RTÉ show {what}s that are not strings are not supported natively"
        ))),
    }
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.into()),
        _ => None,
    }
}

/// Python truthiness of a `mg.get(KEY)` string.
fn text<'a>(mg: &'a Value, key: &str) -> Option<&'a str> {
    match mg.get(key) {
        Some(value) if super::lnk::truthy(Some(value)) => match value {
            Value::String(text) => Some(text),
            _ => None,
        },
        _ => None,
    }
}

/// `RteBaseIE._real_extract`.
fn extract(
    client: &Client,
    url: &Url,
    pattern: &Regex,
    (name, key): (&'static str, &'static str),
) -> Result<Option<VideoInfo>> {
    let Some(found) = pattern.captures(url.as_str()) else {
        return Ok(None);
    };
    let item_id = found["id"].to_owned();
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));

    let mut info: Option<Vec<(&str, Value)>> = None;
    let mut formats = Vec::new();
    for (num, endpoint) in ENDPOINTS.iter().enumerate() {
        let api = Url::parse(&format!("{endpoint}{item_id}"))?;
        let data = match feed(client, &api, name, &item_id)? {
            Feed::Json(data) => data,
            Feed::Failed(error, body) => {
                if num + 1 < ENDPOINTS.len() || !formats.is_empty() {
                    continue;
                }
                if let Some(body) = body {
                    // `_parse_json(..., fatal=False)`.
                    match crate::json::from_str::<Value>(&body) {
                        Ok(error_info) => {
                            if super::lnk::truthy(Some(&error_info)) {
                                let message = match error_info.get("message") {
                                    Some(Value::String(message)) => message.clone(),
                                    Some(Value::Null) => "None".into(),
                                    Some(other) => other.to_string(),
                                    None => return Err(key_error(&item_id, "message")),
                                };
                                return Err(Error::Reported(format!(
                                    "[{name}] {item_id}: {name} said: {message}"
                                )));
                            }
                        }
                        Err(_) => {
                            let text =
                                crate::json::python_decode_error_text(&body).unwrap_or_default();
                            crate::screen::warning(&format!(
                                "[{name}] {item_id}: Failed to parse JSON: {text}"
                            ));
                        }
                    }
                }
                return Err(error);
            }
        };

        // `try_get(data, lambda x: x['shows'][0], dict)`.
        let Some(show) = data
            .get("shows")
            .and_then(Value::as_array)
            .and_then(|shows| shows.first())
            .filter(|show| show.is_object())
        else {
            continue;
        };

        if info.is_none() {
            let title = unescape(
                Some(
                    show.get("title")
                        .ok_or_else(|| key_error(&item_id, "title"))?,
                ),
                "title",
            )?;
            let description = unescape(show.get("description"), "description")?;
            let thumbnail = show.get("thumbnail").cloned().unwrap_or(Value::Null);
            // `float_or_none(show.get('duration'), 1000)`.
            let duration = float_or_none(show.get("duration"))
                .as_f64()
                .and_then(|seconds| serde_json::Number::from_f64(seconds / 1000.0))
                .map_or(Value::Null, Value::Number);
            let timestamp = show
                .get("published")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map_or(Value::Null, Value::from);
            info = Some(vec![
                ("id", Value::from(item_id.as_str())),
                ("title", title),
                ("description", description),
                ("thumbnail", thumbnail),
                ("timestamp", timestamp),
                ("duration", duration),
            ]);
        }

        // `try_get(show, lambda x: x['media:group'][0], dict)`.
        let Some(mg) = show
            .get("media:group")
            .and_then(Value::as_array)
            .and_then(|groups| groups.first())
            .filter(|mg| mg.is_object())
        else {
            continue;
        };

        if let Some(mg_url) = text(mg, "url")
            && let Some(rtmp) = RTMP.captures(mg_url)
        {
            let app = &rtmp["app"];
            formats.push(dict_format(
                vec![
                    ("url", format!("{}/{app}", &rtmp["url"]).into()),
                    ("app", app.into()),
                    ("play_path", rtmp["playpath"].into()),
                    ("player_url", url.as_str().into()),
                    ("ext", "flv".into()),
                    ("format_id", "rtmp".into()),
                ],
                name,
                &item_id,
            )?);
        }

        if let (Some(server), Some(path)) = (text(mg, "hls_server"), text(mg, "hls_url")) {
            let manifest = Url::parse(&format!("{server}{path}"))?;
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                name,
                &item_id,
                Some("hls"),
                false,
                false,
            ) {
                formats.extend(stream.formats);
            }
        }

        if let (Some(server), Some(path)) = (text(mg, "hds_server"), text(mg, "hds_url")) {
            let manifest = Url::parse(&format!("{server}{path}"))?;
            formats.extend(super::extract_f4m_optional(
                client,
                name,
                &manifest,
                &item_id,
                Some("hds"),
            )?);
        }

        let mg_rte_server = str_or_none(mg.get("rte:server")).filter(|text| !text.is_empty());
        let mg_url = str_or_none(mg.get("url")).filter(|text| !text.is_empty());
        if let (Some(server), Some(path)) = (mg_rte_server, mg_url)
            && let Some(hds_url) = super::json_ld::url_or_none(&format!("{server}{path}"))
        {
            formats.extend(super::extract_f4m_optional(
                client,
                name,
                &Url::parse(&hds_url)?,
                &item_id,
                Some("hds"),
            )?);
        }
    }

    let Some(mut fields) = info else {
        return Err(Error::UnsupportedHtml5(
            "RTÉ feeds without a show are not supported natively".into(),
        ));
    };
    fields.push(("formats", Value::Null));
    Ok(Some(dict_info(url, (name, key), fields, formats)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.rte.ie/player/ie/show/iwitness-862/10478715/"
        ));
        assert!(!suitable("http://www.rte.ie/player/ie/show/iwitness-862/"));
        assert!(radio_suitable(
            "http://www.rte.ie/radio/utils/radioplayer/rteradioweb.html#!rii=16:10507902:2414:27-12-2015:"
        ));
        assert!(radio_suitable(
            "http://rte.ie/radio/utils/radioplayer/rteradioweb.html#!rii=b16_3250678_8861_06-04-2012_"
        ));
        assert!(!radio_suitable(
            "http://rte.ie/radio/utils/radioplayer/rteradioweb.html"
        ));
    }
}
