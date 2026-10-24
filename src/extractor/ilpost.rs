//! Upstream `IlPostIE` (`yt_dlp/extractor/ilpost.py`): an episode page's
//! `ilpostpodcast` endpoint metadata and the podcast list POSTed back for
//! the episode's audio.
use super::clyp::{float_or_none, key_error};
use super::{Extractor, download_webpage, search_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct IlPost;

const NAME: &str = "IlPost";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?ilpost\.it/episodes/(?P<id>[^/?#]+)").unwrap()
});
static METADATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+ilpostpodcast\s*=").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python `str(value)` of a JSON scalar.
fn py_str(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(number.to_string()),
        _ => None,
    }
}

impl Extractor for IlPost {
    fn key(&self) -> &'static str {
        NAME
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
        let endpoint = match search_json(&METADATA, &webpage, false) {
            Some(Some(endpoint @ Value::Object(_))) => endpoint,
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "IlPost metadata that does not parse as an object is not supported natively"
                        .into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract metadata; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let field = |key: &str| -> Result<String> {
            let value = endpoint
                .get(key)
                .ok_or_else(|| key_error(&display_id, key))?;
            py_str(value).ok_or_else(|| {
                Error::UnsupportedHtml5(format!(
                    "IlPost {key} values that are not strings or integers are not supported natively"
                ))
            })
        };
        let episode_id = field("post_id")?;
        let podcast_id = field("podcast_id")?;
        let ajax_url = field("ajax_url")?;
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("action", "checkpodcast")
            .append_pair("cookie", &field("cookie")?)
            .append_pair("post_id", &episode_id)
            .append_pair("podcast_id", &podcast_id)
            .finish();
        let ajax_url = Url::parse(&ajax_url).map_err(|_| {
            Error::UnsupportedHtml5(format!(
                "IlPost endpoint URLs that are not absolute are not supported natively: {ajax_url}"
            ))
        })?;
        let podcasts = super::cloudycdn::post_json(client, &ajax_url, (NAME, &display_id), body)?;
        // `('data', 'postcastList', lambda _, v: str(v['id']) == episode_id,
        // {dict})`, first match.
        let episode = podcasts
            .get("data")
            .and_then(|data| data.get("postcastList"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|item| {
                item.is_object()
                    && item.get("id").and_then(py_str).as_deref() == Some(episode_id.as_str())
            })
            .filter(|episode| super::lnk::truthy(Some(episode)))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Episode could not be extracted; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let string = |key: &str| episode.get(key).filter(|value| value.is_string()).cloned();
        let url_value = |key: &str| {
            episode
                .get(key)
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
        };
        let media = url_value("podcast_raw_url");
        let timestamp = match super::streamable::int_or_none(episode.get("timestamp")) {
            Value::Null => None,
            value => Some(value),
        };
        // `float_or_none(scale=1000)`.
        let duration = match float_or_none(episode.get("milliseconds")) {
            Value::Number(number) => number
                .as_f64()
                .and_then(|value| serde_json::Number::from_f64(value / 1000.0))
                .map(Value::Number),
            _ => None,
        };
        let availability = episode.get("free").map(|free| {
            Value::from(if super::lnk::truthy(Some(free)) {
                "public"
            } else {
                "subscriber_only"
            })
        });
        let Some(media) = media else {
            return Err(Error::UnsupportedHtml5(
                "IlPost episodes without a podcast_raw_url are not supported natively".into(),
            ));
        };
        let media = Url::parse(&media)?;
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.vcodec = Some("none".into());
            format.format_id = String::new();
        }
        info.vcodec = Some("none".into());
        info.format_id = None;
        info.id = episode_id;
        let title = string("title");
        match &title {
            Some(Value::String(title)) => info.title = title.clone(),
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "IlPost episodes without a string title are not supported natively".into(),
                ));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let mut keys = vec!["id", "display_id", "series_id", "vcodec"];
        crate::metadata_parser::set_field(&mut info, "display_id", display_id.into());
        crate::metadata_parser::set_field(&mut info, "series_id", podcast_id.into());
        if title.is_some() {
            keys.push("title");
        }
        for (name, value) in [
            ("description", string("description")),
            ("url", Some(Value::Null)),
            ("thumbnail", url_value("image").map(Value::from)),
            ("timestamp", timestamp),
            ("duration", duration),
            ("availability", availability),
        ] {
            let Some(value) = value else { continue };
            keys.push(name);
            if !value.is_null() {
                crate::metadata_parser::set_field(&mut info, name, value);
            }
        }
        let shape = format!("untyped-dict;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.ilpost.it/episodes/1-avis-akvasas-ka/"
        ));
        assert!(suitable("https://ilpost.it/episodes/x"));
        assert!(!suitable("https://www.ilpost.it/podcasts/x"));
    }
}
