//! Upstream `TF1IE` (`yt_dlp/extractor/tf1.py`): a TF1 video page's GraphQL
//! `videoBySlug` as a `url_transparent` result of its `wat:STREAM_ID`
//! media, overlaid with the title, decoration thumbnails and description,
//! date, duration, tags, program label and season/episode numbers.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Tf1;

const NAME: &str = "TF1";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?tf1\.fr/[^/]+/(?P<program_slug>[^/]+)/videos/(?P<id>[^/?&#]+)\.html",
    )
    .unwrap()
});

/// `_match_valid_url`.
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
    Error::UnsupportedHtml5(format!("TF1 {what} are not supported natively"))
}

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

impl Extractor for Tf1 {
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
        let (program_slug, slug) = (found["program_slug"].to_owned(), found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let mut api = Url::parse("https://www.tf1.fr/graphql/web")?;
        // `json.dumps`' default separators.
        let variables =
            crate::output::python_json_dumps(&json!({"programSlug": program_slug, "slug": slug}));
        api.query_pairs_mut()
            .append_pair(
                "id",
                "9b80783950b85247541dd1d851f9cc7fa36574af015621f853ab111a679ce26f",
            )
            .append_pair("variables", &variables);
        let response = download_json(client, &api, NAME, &slug)?;
        let video = response
            .get("data")
            .ok_or_else(|| key_error(&slug, "data"))?
            .get("videoBySlug")
            .ok_or_else(|| key_error(&slug, "videoBySlug"))?;
        let wat_id = match video.get("streamId") {
            Some(Value::String(id)) => id.clone(),
            Some(_) => return Err(unsupported("stream IDs that are not strings")),
            None => return Err(key_error(&slug, "streamId")),
        };
        let mut tags = Vec::new();
        if let Some(Value::Array(items)) = video.get("tags") {
            for tag in items {
                if let Some(label) = tag.get("label").filter(|label| super::nuxt::truthy(label)) {
                    tags.push(label.clone());
                }
            }
        }
        let decoration = video
            .get("decoration")
            .filter(|decoration| super::nuxt::truthy(decoration))
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let mut thumbnails = Vec::new();
        if let Some(Value::Array(sources)) = decoration.pointer("/image/sources") {
            for source in sources {
                let Some(source_url) = source.get("url").filter(|url| super::nuxt::truthy(url))
                else {
                    continue;
                };
                thumbnails.push(json!({
                    "url": source_url,
                    "width": int_or_none(source.get("width")),
                }));
            }
        }
        let fields = [
            ("title", video.get("title").cloned().unwrap_or(Value::Null)),
            ("thumbnails", Value::Array(thumbnails)),
            (
                "description",
                decoration
                    .get("description")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "timestamp",
                video
                    .get("date")
                    .and_then(Value::as_str)
                    .and_then(crate::dates::parse_iso8601)
                    .map_or(Value::Null, Value::from),
            ),
            (
                "duration",
                int_or_none(video.pointer("/publicPlayingInfos/duration"))
                    .map_or(Value::Null, Value::from),
            ),
            ("tags", Value::Array(tags)),
            (
                "series",
                decoration
                    .get("programLabel")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "season_number",
                int_or_none(video.get("season")).map_or(Value::Null, Value::from),
            ),
            (
                "episode_number",
                int_or_none(video.get("episode")).map_or(Value::Null, Value::from),
            ),
        ];
        let target = Url::parse(&format!("wat:{wat_id}"))?;
        let Some(mut info) = super::wat::Wat.extract(client, &target)? else {
            return Ok(None);
        };
        // `url_transparent`: the outer non-`None` fields other than `_type`,
        // `url`, `id` and `ie_key` (the `wat.tv` extractor names stay).
        let mut keys = Vec::new();
        for (key, value) in fields {
            if value.is_null() {
                continue;
            }
            keys.push(key);
            match key {
                "title" => {
                    let Some(title) = value.as_str() else {
                        return Err(unsupported("titles that are not strings"));
                    };
                    info.title = title.to_owned();
                    info.extra.shift_remove(crate::info::TITLE_MISSING);
                }
                _ => crate::metadata_parser::set_field(&mut info, key, value),
            }
        }
        info.webpage_url = url.clone();
        let inner_shape = match info.extra.get(crate::info_order::INFO_SHAPE) {
            Some(Value::String(shape)) => shape.clone(),
            _ => String::new(),
        };
        let shape = format!(
            "{inner_shape};thumbnail=url,width|transparent:{}",
            keys.join(",")
        );
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info, &shape,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.tf1.fr/tmc/quotidien-avec-yann-barthes/videos/quotidien-premiere-partie-11-juin-2019.html"
        ));
        assert!(suitable(
            "http://www.tf1.fr/hd1/documentaire/videos/mylene-farmer-d-une-icone.html"
        ));
        assert!(!suitable(
            "https://www.tf1.fr/tmc/quotidien-avec-yann-barthes"
        ));
    }
}
