//! Upstream `DumpertIE` (`yt_dlp/extractor/dumpert.py`): an item's
//! `mobile_api` info JSON, its video variants (HLS or progressive, ranked by
//! `qualities`), stills and stats.
use super::clyp::key_error;
use super::{Extractor, dict_format, dict_info, download_json, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Dumpert;

const NAME: &str = "Dumpert";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<protocol>https?)://(?:(?:www|legacy)\.)?dumpert\.nl/(?:(?:mediabase|embed|item)/|[^#]*[?&]selectedId=)(?P<id>[0-9]+[/_][0-9a-zA-Z]+)",
    )
    .unwrap()
});

const QUALITIES: [&str; 5] = ["flv", "mobile", "tablet", "720p", "1080p"];

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Dumpert {what} are not supported natively"))
}

impl Extractor for Dumpert {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let raw = found["id"].to_owned();
        let id = raw.replace('_', "/");
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "http://api-live.dumpert.nl/mobile_api/json/info/{}",
            id.replace('/', "_")
        ))?;
        let data = download_json(client, &api, NAME, &id)?;
        let item = data
            .get("items")
            .ok_or_else(|| key_error(&raw, "items"))?
            .as_array()
            .ok_or_else(|| unsupported("item lists that are not arrays"))?
            .first()
            // An `IndexError` is not wrapped in an `ExtractorError`.
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let title = item
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&raw, "title"))?;
        if !title.is_string() {
            return Err(unsupported("titles that are not strings"));
        }
        let media = item
            .get("media")
            .ok_or_else(|| key_error(&raw, "media"))?
            .as_array()
            .ok_or_else(|| unsupported("media lists that are not arrays"))?
            .iter()
            .find(|media| media.get("mediatype").and_then(Value::as_str) == Some("VIDEO"))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "{raw}: An extractor error has occurred. (caused by StopIteration()); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let mut formats = Vec::new();
        for variant in media
            .get("variants")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(uri) = variant.get("uri").filter(|uri| super::nuxt::truthy(uri)) else {
                continue;
            };
            let uri = uri
                .as_str()
                .ok_or_else(|| unsupported("variant URIs that are not strings"))?;
            let version = variant.get("version").and_then(Value::as_str);
            let preference = version
                .and_then(|version| QUALITIES.iter().position(|known| *known == version))
                .map_or(-1, |index| index as i64);
            if super::direct::determine_ext(uri).as_deref() == Some("m3u8") {
                let manifest = Url::parse(uri)?;
                let stream = extract_m3u8_fatal(client, &manifest, NAME, &id, version, false)?;
                for mut format in stream.formats {
                    format.quality = Some(preference as f64);
                    crate::info_order::mark_int_quality(&mut format);
                    formats.push(format);
                }
            } else {
                formats.push(dict_format(
                    vec![
                        ("url", uri.into()),
                        (
                            "format_id",
                            variant.get("version").cloned().unwrap_or(Value::Null),
                        ),
                        ("quality", preference.into()),
                    ],
                    NAME,
                    &id,
                )?);
            }
        }
        let mut thumbnails = Vec::new();
        let stills = item
            .get("stills")
            .filter(|stills| super::nuxt::truthy(stills));
        for kind in ["thumb", "still"] {
            for size in ["", "-medium", "-large"] {
                let still_id = format!("{kind}{size}");
                let Some(still_url) = stills
                    .and_then(|stills| stills.get(&still_id))
                    .filter(|still| super::nuxt::truthy(still))
                else {
                    continue;
                };
                thumbnails.push(json!({"id": still_id, "url": still_url}));
            }
        }
        let stats = item.get("stats").filter(|stats| super::nuxt::truthy(stats));
        let stat =
            |key: &str| super::streamable::int_or_none(stats.and_then(|stats| stats.get(key)));
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", id.clone().into()),
                ("title", title),
                (
                    "description",
                    item.get("description").cloned().unwrap_or(Value::Null),
                ),
                ("thumbnails", Value::Array(thumbnails)),
                ("formats", Value::Null),
                (
                    "duration",
                    super::streamable::int_or_none(media.get("duration")),
                ),
                ("like_count", stat("kudos_total")),
                ("view_count", stat("views_total")),
            ],
            formats,
        );
        // The thumbnails' `{'id', 'url'}` key order.
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=id,url");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.dumpert.nl/item/6646981_951bc60f"));
        assert!(suitable(
            "http://legacy.dumpert.nl/mediabase/6646981/951bc60f"
        ));
        assert!(suitable(
            "https://www.dumpert.nl/toppers/dag?selectedId=100086074_f5cef3ac"
        ));
        assert!(!suitable("https://www.dumpert.nl/toppers"));
    }
}
