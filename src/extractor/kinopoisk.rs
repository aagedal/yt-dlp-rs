//! Upstream `KinoPoiskIE` (`yt_dlp/extractor/kinopoisk.py`): KinoPoisk films
//! from the OTT widget page's `application/json` models: the film status's
//! title, description, cover, duration and age restriction with the playlist
//! entity's HLS formats.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct KinoPoisk;

const NAME: &str = "KinoPoisk";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?kinopoisk\.ru/film/(?P<id>\d+)").unwrap()
});
static DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<script[^>]+\btype=["']application/json[^>]+>(.+?)<"#).unwrap()
});

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
    Error::UnsupportedHtml5(format!("KinoPoisk {what} are not supported natively"))
}

fn get<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| key_error(id, key))
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        Some(Value::Bool(value)) => Some(i64::from(*value)),
        _ => None,
    };
    number.map_or(Value::Null, Value::from)
}

/// The first truthy value of `keys` (`dict_get`/`a or b`), which must be a
/// string.
fn first_truthy(film: &Value, keys: &[&str], what: &str) -> Result<Value> {
    match keys
        .iter()
        .filter_map(|key| film.get(*key))
        .find(|value| super::nuxt::truthy(value))
    {
        None => Ok(Value::Null),
        Some(Value::String(text)) => Ok(text.clone().into()),
        Some(_) => Err(unsupported(what)),
    }
}

impl Extractor for KinoPoisk {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_GEO_COUNTRIES = ['RU']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["RU"])
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
        let mut widget = Url::parse("https://ott-widget.kinopoisk.ru/v1/kp/")?;
        widget.query_pairs_mut().append_pair("kpId", &video_id);
        let webpage = download_webpage(client, &widget, NAME, &video_id)?;
        let raw = DATA
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let data: Value = crate::json::from_str(&raw).map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
            ))
        })?;
        let data = get(&data, "models", &video_id)?;
        let film = get(data, "filmStatus", &video_id)?;
        // `film.get('title') or film['originalTitle']`.
        let title = match film.get("title").filter(|title| super::nuxt::truthy(title)) {
            Some(title) => title,
            None => get(film, "originalTitle", &video_id)?,
        };
        if !matches!(title, Value::String(_) | Value::Null) {
            return Err(unsupported("titles that are not strings"));
        }
        let title = title.clone();
        let playlist = match get(get(data, "playlistEntity", &video_id)?, "uri", &video_id)? {
            Value::String(playlist) => playlist,
            _ => return Err(unsupported("playlist URIs that are not strings")),
        };
        let manifest =
            Url::parse(playlist).map_err(|_| unsupported("playlist URIs that are not URLs"))?;
        let stream =
            extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, Some("hls"), false)?;
        let description = first_truthy(
            film,
            &[
                "descriptscription",
                "description",
                "shortDescriptscription",
                "shortDescription",
            ],
            "descriptions that are not strings",
        )?;
        let thumbnail = first_truthy(
            film,
            &["coverUrl", "posterUrl"],
            "thumbnails that are not strings",
        )?;
        let fields: Vec<(&str, Value)> = vec![
            ("id", video_id.into()),
            ("title", title),
            ("description", description),
            ("thumbnail", thumbnail),
            ("duration", int_or_none(film.get("duration"))),
            ("age_limit", int_or_none(film.get("restrictionAge"))),
            ("formats", Value::Null),
        ];
        Ok(Some(dict_info(url, (NAME, NAME), fields, stream.formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.kinopoisk.ru/film/81041/watch/"));
        assert!(suitable("https://kinopoisk.ru/film/81041"));
        assert!(!suitable("https://www.kinopoisk.ru/series/81041"));
    }
}
