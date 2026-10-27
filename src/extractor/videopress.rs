//! Upstream `VideoPressIE` (`yt_dlp/extractor/videopress.py`): an embed's
//! `public-api.wordpress.com` video JSON (requested with a random birthday)
//! as its MP4/Ogg files and original upload.
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct VideoPress;

const NAME: &str = "VideoPress";
const QUALITIES: [&str; 3] = ["std", "dvd", "hd"];
const FIELDS: &str = "description,duration,file_url_base,files,height,original,poster,rating,title,upload_date,width";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://video(?:\.word)?press\.com/embed/(?P<id>[\da-zA-Z]{8})").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `random_birthday(...)`: a date from 1950-01-01 to 1995-12-31.
fn random_birthday() -> (String, String, String) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let mut offset = (nanos.wrapping_mul(6_364_136_223_846_793_005) >> 17) % 16_802;
    let (mut year, mut month) = (1950_u32, 1_u32);
    loop {
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = match month {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        if offset < days {
            return (
                year.to_string(),
                month.to_string(),
                (offset + 1).to_string(),
            );
        }
        offset -= days;
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
}

/// `parse_age_limit(s)`.
pub(crate) fn parse_age_limit(value: Option<&Value>) -> Value {
    static AGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,2})\+?$").unwrap());
    static TV: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^TV[_-]?(Y|Y7|G|PG|14|MA)$").unwrap());
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .filter(|age| (0..=21).contains(age))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => {
            if let Some(found) = AGE.captures(text) {
                return found[1].parse::<i64>().map_or(Value::Null, Value::from);
            }
            let upper = text.to_uppercase();
            let us = match upper.as_str() {
                "G" => Some(0),
                "PG" => Some(10),
                "PG-13" => Some(13),
                "R" => Some(16),
                "NC" => Some(18),
                _ => None,
            };
            us.or_else(|| {
                TV.captures(&upper).map(|found| match &found[1] {
                    "Y7" => 7,
                    "14" => 14,
                    "MA" => 17,
                    _ => 0,
                })
            })
            .map_or(Value::Null, Value::from)
        }
        _ => Value::Null,
    }
}

/// `urljoin(base_url, path)`.
pub(crate) fn urljoin(base: Option<&str>, path: &str) -> Option<String> {
    static ABSOLUTE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:[a-zA-Z][a-zA-Z0-9+\-.]*:)?//").unwrap());
    static BASE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:https?:)?//").unwrap());
    if path.is_empty() {
        return None;
    }
    if ABSOLUTE.is_match(path) {
        return Some(path.to_owned());
    }
    let base = base.filter(|base| BASE.is_match(base))?;
    Url::parse(&super::proto_relative(base))
        .ok()?
        .join(path)
        .ok()
        .map(String::from)
}

impl Extractor for VideoPress {
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
        let (year, month, day) = random_birthday();
        let api = super::update_url_query(
            &Url::parse(&format!(
                "https://public-api.wordpress.com/rest/v1.1/videos/{id}"
            ))?,
            &[
                ("birth_year", &year),
                ("birth_month", &month),
                ("birth_day", &day),
                ("fields", FIELDS),
            ],
        );
        let video = download_json(client, &api, NAME, &id)?;
        let title = video
            .get("title")
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&id, "title"))?;
        if !title.is_string() {
            return Err(Error::UnsupportedHtml5(
                "VideoPress titles that are not strings are not supported natively".into(),
            ));
        }
        let file_url_base = video.get("file_url_base");
        let base_url = file_url_base
            .and_then(|base| base.get("https").filter(|value| super::nuxt::truthy(value)))
            .or_else(|| file_url_base.and_then(|base| base.get("http")))
            .and_then(Value::as_str);

        let mut formats = Vec::new();
        if let Some(files) = video.get("files").and_then(Value::as_object) {
            for (format_id, file) in files {
                let Some(file) = file.as_object() else {
                    continue;
                };
                for (ext, path) in file {
                    if ext != "mp4" && ext != "ogg" {
                        continue;
                    }
                    let Some(path) = path.as_str() else {
                        return Err(Error::UnsupportedHtml5(
                            "VideoPress file paths that are not strings are not supported natively"
                                .into(),
                        ));
                    };
                    let format_url = urljoin(base_url, path).ok_or_else(|| {
                        Error::UnsupportedHtml5(
                            "VideoPress files without a joinable URL are not supported natively"
                                .into(),
                        )
                    })?;
                    let quality = QUALITIES
                        .iter()
                        .position(|quality| quality == format_id)
                        .map_or(-1, |index| index as i64);
                    formats.push(dict_format(
                        vec![
                            ("url", format_url.into()),
                            ("format_id", format!("{format_id}-{ext}").into()),
                            (
                                "ext",
                                super::direct::determine_ext(path)
                                    .unwrap_or_else(|| ext.clone())
                                    .into(),
                            ),
                            ("quality", quality.into()),
                        ],
                        NAME,
                        &id,
                    )?);
                }
            }
        }
        if let Some(original) = video
            .get("original")
            .filter(|value| super::nuxt::truthy(value))
        {
            formats.push(dict_format(
                vec![
                    ("url", original.clone()),
                    ("format_id", "original".into()),
                    ("quality", (QUALITIES.len() as i64).into()),
                    ("width", int_or_none(video.get("width"))),
                    ("height", int_or_none(video.get("height"))),
                ],
                NAME,
                &id,
            )?);
        }

        let duration = match video.get("duration") {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
            _ => None,
        }
        .map_or(Value::Null, |duration| Value::from(duration / 1000.0));
        let timestamp = video
            .get("upload_date")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, true))
            .map_or(Value::Null, Value::from);
        let mut fields = vec![
            ("id", Value::from(id.clone())),
            ("title", title),
            (
                "description",
                video.get("description").cloned().unwrap_or(Value::Null),
            ),
            (
                "thumbnail",
                video.get("poster").cloned().unwrap_or(Value::Null),
            ),
            ("duration", duration),
            ("timestamp", timestamp),
            ("age_limit", parse_age_limit(video.get("rating"))),
            ("formats", Value::Null),
        ];
        // Processing drops a `None` duration.
        fields.retain(|(key, value)| *key != "duration" || !value.is_null());
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://videopress.com/embed/kUJmAcSf"));
        assert!(suitable("https://video.wordpress.com/embed/kUJmAcSf"));
        assert!(!suitable("https://videopress.com/embed/kUJm"));
    }

    #[test]
    fn age_limits_follow_parse_age_limit() {
        assert_eq!(parse_age_limit(Some(&json!("G"))), json!(0));
        assert_eq!(parse_age_limit(Some(&json!("pg-13"))), json!(13));
        assert_eq!(parse_age_limit(Some(&json!("17+"))), json!(17));
        assert_eq!(parse_age_limit(Some(&json!("TV_MA"))), json!(17));
        assert_eq!(parse_age_limit(Some(&json!(30))), Value::Null);
        assert_eq!(parse_age_limit(Some(&json!("X"))), Value::Null);
    }

    #[test]
    fn birthdays_stay_in_upstreams_range() {
        let (year, month, day) = random_birthday();
        let year: u32 = year.parse().unwrap();
        assert!((1950..=1995).contains(&year));
        assert!((1..=12).contains(&month.parse::<u32>().unwrap()));
        assert!((1..=31).contains(&day.parse::<u32>().unwrap()));
    }
}
