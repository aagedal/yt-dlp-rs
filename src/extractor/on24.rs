//! Upstream `On24IE` (`yt_dlp/extractor/on24.py`): ON24 events from the
//! event console's cached JSON: its `fhvideo1` video and `audio` files with
//! the event's locale, description, start date and registrant count.
use super::{Extractor, dict_format, dict_info};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct On24;

const NAME: &str = "on24";
const KEY: &str = "On24";

static VALID_URLS: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    let query = r"(?:[^#]*&)?eventid=(?P<id>\d{7})&(?:[^#]+&)?key=(?P<key>[0-9A-F]{32})";
    [
        crate::extractor::url_regex(r"^https?://event\.on24\.com/wcc/r/(?P<id>\d{7})/(?P<key>[0-9A-F]{32})")
            .unwrap(),
        crate::extractor::url_regex(&format!(
            r"^https?://event\.on24\.com/eventRegistration/console/(?:EventConsoleApollo\.jsp|apollox/mainEvent/?)\?{query}"
        ))
        .unwrap(),
        crate::extractor::url_regex(&format!(
            r"^https?://event\.on24\.com/eventRegistration/EventLobbyServlet/?\?{query}"
        ))
        .unwrap(),
    ]
});

fn ids(url: &str) -> Option<(String, String)> {
    VALID_URLS
        .iter()
        .find_map(|regex| regex.captures(url))
        .map(|found| (found["id"].to_owned(), found["key"].to_owned()))
}

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    ids(url).is_some()
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("ON24 {what} are not supported natively"))
}

/// Python's `str(value)` of a JSON scalar.
fn python_str(value: Option<&Value>) -> Result<String> {
    match value {
        None | Some(Value::Null) => Ok("None".into()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Bool(true)) => Ok("True".into()),
        Some(Value::Bool(false)) => Ok("False".into()),
        Some(Value::Number(number)) if number.is_f64() => Ok(crate::output::python_json_dumps(
            &Value::Number(number.clone()),
        )),
        Some(Value::Number(number)) => Ok(number.to_string()),
        Some(_) => Err(unsupported("values that are not scalars")),
    }
}

impl Extractor for On24 {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some((event_id, event_key)) = ids(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = super::update_url_query(
            &Url::parse("https://event.on24.com/apic/utilApp/EventConsoleCachedServlet")?,
            &[
                ("eventId", event_id.as_str()),
                ("displayProfile", "player"),
                ("key", event_key.as_str()),
                ("contentType", "A"),
            ],
        );
        let event_data = super::download_json(client, &api, NAME, &event_id)?;
        if !event_data.is_object() {
            return Err(unsupported("event data that are not objects"));
        }
        // `str(try_get(...)) or event_id`: a missing ID is `'None'`.
        let event_id = python_str(event_data.pointer("/presentationLogInfo/eventid"))?;
        let language = event_data
            .get("localelanguagecode")
            .cloned()
            .unwrap_or(Value::Null);
        let media_list = match event_data.get("mediaUrlInfo") {
            None => Vec::new(),
            Some(Value::Array(media)) => media.clone(),
            Some(_) => return Err(unsupported("media lists that are not lists")),
        };
        let mut formats = Vec::new();
        for media in &media_list {
            if !media.is_object() {
                return Err(unsupported("media entries that are not objects"));
            }
            let path = python_str(media.get("url"))?;
            let Some(media_url) = super::videopress::urljoin(
                Some("https://event.on24.com/media/news/corporatevideo/events/"),
                &path,
            ) else {
                continue;
            };
            let (format_id, ext, vcodec, acodec) = match media.get("code").and_then(Value::as_str) {
                Some("fhvideo1") => ("video", "mp4", "avc1.640020", "mp4a.40.2"),
                Some("audio") => ("audio", "wav", "none", "wav"),
                _ => continue,
            };
            formats.push(dict_format(
                vec![
                    ("format_id", format_id.into()),
                    ("url", media_url.into()),
                    ("language", language.clone()),
                    ("ext", ext.into()),
                    ("vcodec", vcodec.into()),
                    ("acodec", acodec.into()),
                ],
                NAME,
                &event_id,
            )?);
        }
        let title = event_data
            .get("description")
            .and_then(Value::as_str)
            .map_or(Value::Null, |title| title.trim().into());
        // `int_or_none(startdate, 1000)`: `int(value) // 1000`.
        let timestamp = match event_data.pointer("/session/startdate") {
            Some(Value::Number(number)) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
            Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
            Some(Value::Bool(value)) => Some(i64::from(*value)),
            _ => None,
        }
        .map_or(Value::Null, |value| value.div_euclid(1000).into());
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", event_id.clone().into()),
                ("title", title),
                ("timestamp", timestamp),
                (
                    "webpage_url",
                    format!("https://event.on24.com/wcc/r/{event_id}/{event_key}").into(),
                ),
                (
                    "view_count",
                    event_data
                        .get("registrantcount")
                        .cloned()
                        .unwrap_or(Value::Null),
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
        assert!(suitable(
            "https://event.on24.com/wcc/r/2639291/82829018E813065A122363877975752E?mode=login"
        ));
        assert!(suitable(
            "https://event.on24.com/eventRegistration/EventLobbyServlet?target=reg20.jsp&eventid=3543176&key=BC0F6B968B67C34B50D461D40FDB3E18&groupId=3143628"
        ));
        assert!(!suitable(
            "https://event.on24.com/wcc/r/263929/82829018E813065A122363877975752E"
        ));
    }
}
