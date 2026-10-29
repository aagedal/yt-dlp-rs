//! Upstream `TvoIE` (`yt_dlp/extractor/tvo.py`): TVO videos looked up through
//! the site's GraphQL API, a `url_transparent` result of the Brightcove
//! player (smuggled as CA-only) with the video's metadata.
use super::{Extractor, brightcove};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Tvo;

const NAME: &str = "TVO";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?tvo\.org/video(?:/documentaries)?/(?P<id>[\w-]+)").unwrap()
});

const GRAPHQL_URL: &str = "https://hmy0rc1bo2.execute-api.ca-central-1.amazonaws.com/graphql";
const GRAPHQL_QUERY: &str = "query getVideo($slug: String) {
                  getTVOOrgVideo(slug: $slug) {
                    contentCategory
                    description
                    length
                    program {
                      nodeUrl
                      title
                    }
                    programOrder
                    publishedAt
                    season
                    tags
                    thumbnail
                    title
                    videoSource {
                      brightcoveRefId
                    }
                  }
                }";

/// `BRIGHTCOVE_URL_TEMPLATE`.
const BRIGHTCOVE_URL_TEMPLATE: &str =
    "https://players.brightcove.net/18140038001/default_default/index.html?videoId=";

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none`: integers, and strings of digits.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `{clean_html}, filter` of a string value.
fn clean_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(crate::html::clean_html)
        .filter(|text| !text.is_empty())
}

impl Extractor for Tvo {
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
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `urllib.parse.urlparse(url).path.rstrip('/')`.
        let slug = url.path().trim_end_matches('/');
        let body = serde_json::to_string(&json!({
            "operationName": "getVideo",
            "variables": {"slug": slug},
            "query": GRAPHQL_QUERY,
        }))
        .map_err(|error| Error::UnsupportedHtml5(error.to_string()))?;
        let response = super::post_json_errnote(
            client,
            &Url::parse(GRAPHQL_URL)?,
            (NAME, &display_id),
            (
                "Downloading JSON metadata",
                "Unable to download JSON metadata",
            ),
            &[("Content-Type", "application/json")],
            body,
        )?;
        let video = response
            .pointer("/data/getTVOOrgVideo")
            .cloned()
            .unwrap_or(Value::Null);
        let brightcove_id = video
            .pointer("/videoSource/brightcoveRefId")
            .and_then(|id| match id {
                Value::String(text) => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract Brightcove ID; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;

        let mut fields = Map::new();
        fields.insert("display_id".into(), display_id.clone().into());
        fields.insert("episode_id".into(), display_id.clone().into());
        let title = clean_text(video.get("title"));
        if let Some(title) = &title {
            fields.insert("title".into(), title.clone().into());
        }
        if let Some(category) = clean_text(video.get("contentCategory")) {
            fields.insert("categories".into(), json!([category]));
        }
        if let Some(description) = clean_text(video.get("description")) {
            fields.insert("description".into(), description.into());
        }
        if let Some(duration) = video
            .get("length")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
        {
            fields.insert(
                "duration".into(),
                match duration {
                    crate::info::PyNumber::Int(seconds) => json!(seconds),
                    crate::info::PyNumber::Float(seconds) => json!(seconds),
                },
            );
        }
        if let Some(title) = title {
            fields.insert("episode".into(), title.into());
        }
        if let Some(number) = int_or_none(video.get("programOrder")) {
            fields.insert("episode_number".into(), number.into());
        }
        if let Some(season) = int_or_none(video.get("season")) {
            fields.insert("season_number".into(), season.into());
        }
        // `('tags', ..., {clean_html}, filter)`: no entries, no key.
        let tags: Vec<Value> = video
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| clean_text(Some(tag)))
            .map(Value::from)
            .collect();
        if !tags.is_empty() {
            fields.insert("tags".into(), Value::Array(tags));
        }
        if let Some(thumbnail) = video
            .get("thumbnail")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            fields.insert("thumbnail".into(), thumbnail.into());
        }
        if let Some(published) = video
            .get("publishedAt")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.insert("timestamp".into(), published.into());
        }
        // `program`: the series title, and its `nodeUrl` without `/programs/`.
        let program = video.get("program");
        if let Some(series) = clean_text(program.and_then(|program| program.get("title"))) {
            fields.insert("series".into(), series.into());
        }
        let series_id = clean_text(program.and_then(|program| program.get("nodeUrl")))
            .map(|node| node.strip_prefix("/programs/").unwrap_or(&node).to_owned())
            .filter(|id| !id.is_empty());
        if let Some(series_id) = series_id {
            fields.insert("series_id".into(), series_id.into());
        }

        // `smuggle_url(BRIGHTCOVE_URL, {'geo_countries': ['CA']})`.
        let target = brightcove::smuggle(
            &format!("{BRIGHTCOVE_URL_TEMPLATE}{brightcove_id}"),
            &json!({"geo_countries": ["CA"]}),
        );
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.tvo.org/video/how-can-ontario-survive-the-trade-war"
        ));
        assert!(suitable(
            "https://www.tvo.org/video/documentaries/the-pitch"
        ));
        assert!(!suitable("https://www.tvo.org/"));
    }
}
