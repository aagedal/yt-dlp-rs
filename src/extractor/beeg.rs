//! Upstream `BeegIE` (`yt_dlp/extractor/beeg.py`): Beeg videos from the
//! `store.externulls.com` facts JSON: each HLS resource's master playlist
//! with its `fl_cdn_N` height, the file's metadata and the first fact's ID
//! and date; the page gives the age limit.
use super::{Extractor, dict_info, download_json_note, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Beeg;

const NAME: &str = "Beeg";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?beeg\.(?:com(?:/video)?)/-?(?P<id>\d+)").unwrap()
});
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"fl_cdn_(\d+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Beeg {what} are not supported natively"))
}

/// `int_or_none`.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

impl Extractor for Beeg {
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
        let api = Url::parse(&format!(
            "https://store.externulls.com/facts/file/{video_id}"
        ))?;
        let video = download_json_note(
            client,
            &api,
            NAME,
            &video_id,
            &format!("Downloading JSON for {video_id}"),
        )?;
        // Upstream iterates `fc_facts` unchecked (a `TypeError` otherwise).
        let Some(Value::Array(fc_facts)) = video.get("fc_facts") else {
            return Err(unsupported("files without a facts list"));
        };
        // The fact with the lowest `id` (`try_get` skips incomparable ones).
        let mut first_fact: Option<&serde_json::Map<String, Value>> = None;
        for fact in fc_facts {
            let Value::Object(fact) = fact else {
                return Err(unsupported("facts that are not objects"));
            };
            let lower = match first_fact {
                None => true,
                Some(first) if first.is_empty() => true,
                Some(first) => match (
                    fact.get("id").and_then(Value::as_f64),
                    first.get("id").and_then(Value::as_f64),
                ) {
                    (Some(id), Some(first)) => id < first,
                    _ => false,
                },
            };
            if lower {
                first_fact = Some(fact);
            }
        }
        let empty = serde_json::Map::new();
        let first_fact = first_fact.unwrap_or(&empty);
        let file = video.get("file");
        let resources = file
            .and_then(|file| file.get("hls_resources"))
            .filter(|resources| super::nuxt::truthy(resources))
            .or_else(|| first_fact.get("hls_resources"));
        let Some(Value::Object(resources)) = resources else {
            return Err(unsupported("files without HLS resources"));
        };
        let mut formats = Vec::new();
        for (format_id, video_uri) in resources {
            if !super::nuxt::truthy(video_uri) {
                continue;
            }
            let Value::String(video_uri) = video_uri else {
                return Err(unsupported("HLS resources that are not strings"));
            };
            let height = HEIGHT
                .captures(format_id)
                .and_then(|found| found[1].parse::<u32>().ok());
            let m3u8_id = height.map_or_else(|| "None".to_owned(), |height| height.to_string());
            let manifest = Url::parse(&format!("https://video.beeg.com/{video_uri}"))?;
            let stream = super::extract_m3u8_fatal_master(
                client,
                &manifest,
                NAME,
                &video_id,
                Some(&m3u8_id),
                false,
            )?;
            for mut format in stream.formats {
                // `f['height'] = height`: in place when the variant had one.
                if format.height.is_some() {
                    crate::info_order::mark_format_update_in_place(&mut format, "height");
                } else {
                    crate::info_order::mark_format_update(&mut format, "height");
                }
                format.height = height;
                formats.push(format);
            }
        }
        let stuff = |key: &str| {
            file.and_then(|file| file.get("stuff"))
                .and_then(|stuff| stuff.get(key))
                .cloned()
                .unwrap_or(Value::Null)
        };
        let display_id = match first_fact.get("id") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(id)) => Value::from(id.clone()),
            Some(Value::Number(id)) => Value::from(id.to_string()),
            Some(_) => return Err(unsupported("fact IDs that are not numbers")),
        };
        let timestamp = match first_fact.get("fc_created") {
            Some(Value::String(date)) => crate::dates::unified_timestamp(date, true),
            None | Some(Value::Null) => None,
            Some(_) => return Err(unsupported("fact dates that are not strings")),
        };
        let tags: Vec<Value> = match video.get("tags") {
            Some(Value::Array(tags)) => tags
                .iter()
                .filter_map(|tag| tag.get("tg_name"))
                .filter(|name| !name.is_null())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let duration = int_or_none(file.and_then(|file| file.get("fl_duration")));
        let age_limit = crate::html::rta_search(&webpage);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("display_id", display_id),
                ("title", stuff("sf_name")),
                ("description", stuff("sf_story")),
                ("timestamp", timestamp.map_or(Value::Null, Value::from)),
                ("duration", duration.map_or(Value::Null, Value::from)),
                ("tags", Value::Array(tags)),
                ("formats", Value::Null),
                ("age_limit", age_limit.map_or(Value::Null, Value::from)),
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
        assert!(suitable("https://beeg.com/-0983946056129650"));
        assert!(suitable("https://beeg.com/-0599050563103750?t=4-861"));
        assert!(suitable("https://beeg.com/1941093077?t=911-1391"));
        assert!(suitable("https://beeg.com/video/1277207756"));
        assert!(!suitable("https://beeg.com/tag/x"));
    }
}
