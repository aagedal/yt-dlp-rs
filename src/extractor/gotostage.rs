//! Upstream `GoToStageIE` (`yt_dlp/extractor/gotostage.py`): GoToStage
//! recordings through the contents API, a placeholder registration (a 409
//! answer is accepted) and the registrant's asset download URL.
use super::{Extractor, dict_info};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct GoToStage;

const NAME: &str = "GoToStage";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?gotostage\.com/channel/[a-z0-9]+/recording/(?P<id>[a-z0-9]+)/watch",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError({key})); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("GoToStage {what} are not supported natively"))
}

/// `_download_json(url, video_id, note=..., errnote=..., headers=...)`.
fn get_json(
    client: &Client,
    url: &Url,
    video_id: &str,
    (note, errnote): (&str, &str),
    headers: &[(&str, &str)],
) -> Result<Value> {
    super::download_json_note_headers(client, url, NAME, video_id, note, headers).map_err(|error| {
        match error {
            Error::ExtractorRequest { source, .. } => Error::Reported(super::request_errnote_text(
                NAME, video_id, errnote, &source,
            )),
            error => error,
        }
    })
}

/// `value[key]` of a dict.
fn field<'a>(value: &'a Map<String, Value>, key: &str, video_id: &str) -> Result<&'a Value> {
    value
        .get(key)
        .ok_or_else(|| key_error(video_id, &format!("'{key}'")))
}

/// `try_get(metadata, lambda x: x[key], str)`.
fn string(value: Option<&Value>) -> Value {
    value
        .filter(|value| value.is_string())
        .cloned()
        .unwrap_or(Value::Null)
}

impl Extractor for GoToStage {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let contents = get_json(
            client,
            &Url::parse(&format!(
                "https://api.gotostage.com/contents?ids={video_id}"
            ))?,
            &video_id,
            (
                "Downloading video metadata",
                "Unable to download video metadata",
            ),
            &[],
        )?;
        // `[0]`.
        let metadata = match contents {
            Value::Array(items) => items
                .into_iter()
                .next()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            Value::Object(_) => return Err(key_error(&video_id, "0")),
            _ => return Err(unsupported("contents responses that are not lists")),
        };
        let Value::Object(metadata) = metadata else {
            return Err(unsupported("contents that are not objects"));
        };
        let mut registration = Map::new();
        for (key, source) in [
            ("product", "product"),
            ("resourceType", "contentType"),
            ("productReferenceKey", "productRefKey"),
        ] {
            registration.insert(key.into(), field(&metadata, source, &video_id)?.clone());
        }
        registration.insert("firstName".into(), "foo".into());
        registration.insert("lastName".into(), "bar".into());
        registration.insert("email".into(), "foobar@example.com".into());
        let registration_response = super::post_json_errnote_expecting(
            client,
            &Url::parse("https://api-registrations.logmeininc.com/registrations")?,
            (NAME, &video_id),
            ("Register user", "Unable to register user"),
            &[("Content-Type", "application/json")],
            crate::output::python_json_dumps(&Value::Object(registration)),
            &[409],
        )?;
        let Value::Object(registration_response) = registration_response else {
            return Err(unsupported("registration responses that are not objects"));
        };
        let registration_key = match field(&registration_response, "registrationKey", &video_id)? {
            Value::String(key) => key.clone(),
            _ => return Err(unsupported("registration keys that are not strings")),
        };
        let content = get_json(
            client,
            &Url::parse(&format!(
                "https://api.gotostage.com/contents/{video_id}/asset"
            ))?,
            &video_id,
            ("Get download url", "Unable to get download url"),
            &[("x-registrantkey", &registration_key)],
        )?;
        let media = string(content.get("cdnLocation"));
        let thumbnail = metadata
            .get("thumbnail")
            .and_then(|thumbnail| thumbnail.get("location"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map_or(Value::Null, Value::from);
        // `try_get(..., float)`: JSON integers are not floats.
        let duration = metadata
            .get("duration")
            .filter(|duration| duration.is_f64())
            .cloned()
            .unwrap_or(Value::Null);
        let fields = vec![
            ("id", Value::from(video_id.as_str())),
            ("title", string(metadata.get("title"))),
            ("url", media.clone()),
            ("ext", "mp4".into()),
            ("thumbnail", thumbnail),
            ("duration", duration),
            ("categories", json!([string(metadata.get("category"))])),
            ("is_live", false.into()),
        ];
        let formats = match media.as_str() {
            Some(media) => {
                let mut format: crate::VideoFormat =
                    serde_json::from_value(json!({"format_id": "", "url": media, "ext": "mp4"}))
                        .map_err(|error| Error::Reported(format!("[{NAME}] {error}")))?;
                crate::info_order::mark_format(&mut format, "url-only");
                vec![format]
            }
            None => Vec::new(),
        };
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("untyped-dict;{}", keys.join(","));
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            dict_info(url, (NAME, NAME), fields, formats),
            &shape,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.gotostage.com/channel/8901680603948959494/recording/60bb55548d434f21b9ce4f0e225c4895/watch"
        ));
        assert!(suitable(
            "https://www.gotostage.com/channel/bacc3d/recording/831e74/watch?source=HOMEPAGE"
        ));
        assert!(!suitable("https://www.gotostage.com/channel/bacc3d"));
    }
}
