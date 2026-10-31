//! Upstream `ClypIE` (`yt_dlp/extractor/clyp.py`): a clip's `api.clyp.it`
//! JSON (with the URL's `token`), its Ogg/MP3 audio URLs and the page's
//! `wavStreamUrl`.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Clyp;

const NAME: &str = "Clyp";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?clyp\.it/(?P<id>[a-z0-9]+)").unwrap()
});
static WAV_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"var\s*wavStreamUrl\s*=\s*["'](?P<url>https?://[^'"]+)"#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `float_or_none`.
pub(crate) fn float_or_none(value: Option<&Value>) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    number
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

/// `value[key]`'s `KeyError`.
pub(crate) fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for Clyp {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `parse_qs(url).get('token', [None])[0]` (blank values dropped).
        let token = url
            .query_pairs()
            .find(|(key, value)| key == "token" && !value.is_empty())
            .map(|(_, value)| value.into_owned());
        let api = Url::parse(&format!("https://api.clyp.it/{id}"))?;
        let api = match &token {
            Some(token) => super::update_url_query(&api, &[("token", token)]),
            None => api,
        };
        let metadata = download_json(client, &api, NAME, &id)?;
        let mut formats = Vec::new();
        for secure in ["", "Secure"] {
            for ext in ["Ogg", "Mp3"] {
                let format_id = format!("{secure}{ext}");
                let Some(format_url) = metadata
                    .get(format!("{format_id}Url"))
                    .filter(|value| super::nuxt::truthy(value))
                else {
                    continue;
                };
                formats.push(dict_format(
                    vec![
                        ("url", format_url.clone()),
                        ("format_id", format_id.into()),
                        ("vcodec", "none".into()),
                        ("acodec", ext.to_lowercase().into()),
                    ],
                    NAME,
                    &id,
                )?);
            }
        }
        let page = download_webpage(client, url, NAME, &id)?;
        if let Some(wav_url) = WAV_URL
            .captures(&page)
            .map(|found| crate::html::clean_html(&found["url"]).trim().to_owned())
            .filter(|wav_url| !wav_url.is_empty())
        {
            formats.push(dict_format(
                vec![
                    ("url", wav_url.into()),
                    ("format_id", "wavStreamUrl".into()),
                    ("vcodec", "none".into()),
                    ("acodec", "wav".into()),
                ],
                NAME,
                &id,
            )?);
        }
        let title = metadata
            .get("Title")
            .cloned()
            .ok_or_else(|| key_error(&id, "Title"))?;
        if !title.is_string() {
            return Err(Error::UnsupportedHtml5(
                "Clyp titles that are not strings are not supported natively".into(),
            ));
        }
        let timestamp = metadata
            .get("DateCreated")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, false))
            .map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", id.clone().into()),
                ("title", title),
                (
                    "description",
                    metadata.get("Description").cloned().unwrap_or(Value::Null),
                ),
                ("duration", float_or_none(metadata.get("Duration"))),
                ("timestamp", timestamp),
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
        assert!(suitable("https://clyp.it/iynkjk4b"));
        assert!(suitable(
            "https://clyp.it/b04p1odi?token=b0078e077e15835845c528a44417719d"
        ));
        assert!(!suitable("https://clyp.it/"));
    }
}
