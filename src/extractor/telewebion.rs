//! Upstream `TelewebionIE` (`yt_dlp/extractor/telewebion.py`): Telewebion
//! episodes (decimal IDs as hexadecimal) from the GraphQL `queryEpisode`
//! (a JSON POST): its metadata and the channel's episode HLS formats.
use super::{Extractor, dict_info};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Telewebion;

const NAME: &str = "Telewebion";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?telewebion\.ir/episode/(?P<id>(?:0x[a-fA-F\d]+|\d+))").unwrap()
});

/// `textwrap.dedent` of the episode query.
const EPISODE_QUERY: &str = "
queryEpisode(filter: {EpisodeID: $EpisodeId}, first: 1) {
  title
  program {
    ProgramID
    title
  }
  image
  view_count
  duration
  started_at
  created_at
  channel {
    ChannelID
    name
    descriptor
  }
  tags {
    name
  }
}
";

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Telewebion {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_call_graphql_api(operation, video_id, query, variables)`.
fn call_graphql_api(
    client: &Client,
    operation: &str,
    video_id: &str,
    temp_id: &str,
    query: &str,
    variables: &[(&str, &str, &str)],
) -> Result<Value> {
    let parameters = if variables.is_empty() {
        String::new()
    } else {
        let parameters: Vec<String> = variables
            .iter()
            .map(|(name, type_, _)| format!("${name}: {type_}"))
            .collect();
        format!("({})", parameters.join(", "))
    };
    let query = format!("query {operation}{parameters} @cacheControl(maxAge: 60) {{{query}\n}}\n");
    let values: Vec<String> = variables
        .iter()
        .map(|(name, _, value)| format!("{}:{}", Value::from(*name), Value::from(*value)))
        .collect();
    // `json.dumps(..., separators=(',', ':'))`.
    let body = format!(
        "{{\"operationName\":{},\"query\":{},\"variables\":{{{}}}}}",
        Value::from(operation),
        Value::from(query),
        values.join(",")
    );
    let result = super::post_json_note_headers(
        client,
        &Url::parse("https://graph.telewebion.ir/graphql")?,
        NAME,
        video_id,
        "Downloading GraphQL JSON metadata",
        &[
            ("Content-Type", "application/json"),
            ("Accept", "application/json"),
        ],
        body,
    )?;
    let errors = result
        .get("errors")
        .filter(|errors| super::nuxt::truthy(errors));
    if !super::nuxt::truthy(&result) || errors.is_some() {
        // `traverse_obj(result, ('errors', ..., 'message', {str}))`.
        let messages: Vec<&str> = match errors {
            Some(Value::Array(errors)) => errors
                .iter()
                .filter_map(|error| error.get("message").and_then(Value::as_str))
                .collect(),
            Some(Value::Object(errors)) => errors
                .values()
                .filter_map(|error| error.get("message").and_then(Value::as_str))
                .collect(),
            _ => Vec::new(),
        };
        let message = if messages.is_empty() {
            "Unknown GraphQL API error".to_owned()
        } else {
            messages.join(", ")
        };
        return Err(Error::Reported(format!(
            "[{NAME}] {temp_id}: {message}; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    match result {
        Value::Object(mut result) => result
            .shift_remove("data")
            .ok_or_else(|| key_error(temp_id, "data")),
        _ => Err(unsupported("GraphQL results that are not objects")),
    }
}

/// `format_field(value, template=TEMPLATE, default=None)`.
fn fmt_url(template: &str, value: Option<&Value>) -> Result<Option<String>> {
    let text = match value {
        None => return Ok(None),
        Some(value) if !super::nuxt::truthy(value) => return Ok(None),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) if number.is_i64() || number.is_u64() => number.to_string(),
        Some(_) => return Err(unsupported("URL fields that are not strings")),
    };
    Ok(Some(template.replacen("%s", &text, 1)))
}

/// `int_or_none` (non-integers, including booleans, are not supported).
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

impl Extractor for Telewebion {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = if temp_id.starts_with("0x") {
            temp_id.clone()
        } else {
            // `hex(int(video_id))`.
            let number: u128 = temp_id
                .parse()
                .map_err(|_| unsupported("episode IDs beyond 128 bits"))?;
            format!("{number:#x}")
        };
        let episode_data = call_graphql_api(
            client,
            "getEpisodeDetail",
            &video_id,
            &temp_id,
            EPISODE_QUERY,
            &[("EpisodeId", "[ID!]", &video_id)],
        )?;
        let episode = episode_data
            .pointer("/queryEpisode/0")
            .cloned()
            .unwrap_or(Value::Null);
        let get = |path: &str| episode.pointer(path);
        let string = |path: &str| get(path).filter(|value| value.is_string()).cloned();
        let timestamp = |path: &str| {
            get(path)
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map(Value::from)
        };
        let tags: Vec<Value> = match get("/tags") {
            Some(Value::Array(tags)) => tags
                .iter()
                .filter_map(|tag| tag.get("name").filter(|name| name.is_string()).cloned())
                .collect(),
            Some(Value::Object(tags)) => tags
                .values()
                .filter_map(|tag| tag.get("name").filter(|name| name.is_string()).cloned())
                .collect(),
            _ => Vec::new(),
        };
        let descriptor = get("/channel/descriptor");
        let channel_url = fmt_url("https://telewebion.ir/live/%s", descriptor)?;
        let thumbnail = fmt_url(
            "https://static.telewebion.ir/episodeImages/%s/default",
            get("/image"),
        )?;
        // The traversal calls `_extract_m3u8_formats` even without a URL.
        let m3u8_url = fmt_url(
            &format!("https://cdna.telewebion.ir/%s/episode/{video_id}/playlist.m3u8"),
            descriptor.filter(|descriptor| descriptor.is_string()),
        )?
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Failed to obtain m3u8 URL; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let formats = super::extract_m3u8_fatal_master(
            client,
            &Url::parse(&m3u8_url)?,
            NAME,
            &video_id,
            Some("hls"),
            false,
        )?
        .formats;
        let fields: Vec<(&str, Option<Value>)> = vec![
            ("title", string("/title")),
            (
                "view_count",
                int_or_none(get("/view_count")).map(Value::from),
            ),
            ("duration", int_or_none(get("/duration")).map(Value::from)),
            ("tags", (!tags.is_empty()).then(|| Value::Array(tags))),
            ("release_timestamp", timestamp("/started_at")),
            ("timestamp", timestamp("/created_at")),
            ("series", string("/program/title")),
            ("series_id", string("/program/ProgramID")),
            ("channel", string("/channel/name")),
            ("channel_id", string("/channel/ChannelID")),
            ("channel_url", channel_url.map(Value::from)),
            ("thumbnail", thumbnail.map(Value::from)),
            ("formats", Some(Value::Null)),
            ("id", Some(Value::from(video_id.clone()))),
        ];
        // `traverse_obj`'s dict template leaves out `None` values.
        let fields = fields
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect();
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.telewebion.ir/episode/0x1b3139c/"));
        assert!(suitable("https://telewebion.ir/episode/162175536"));
        assert!(!suitable("https://telewebion.ir/live/tv3"));
    }
}
