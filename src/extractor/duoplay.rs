//! Upstream `DuoplayIE` (`yt_dlp/extractor/duoplay.py`): Duoplay telecasts
//! (and `?ep=N` episodes) from the page's `<video-player>` manifest, played
//! with a registered session token, and its `:episode` metadata (movies
//! without the episode fields).
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Duoplay;

const NAME: &str = "Duoplay";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://duoplay\.ee/(?P<id>\d+)(?:[/?#]|$)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
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

/// `str_or_none(value)`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// The session registration (`errnote='Unable to register session'`).
fn register_session(client: &Client, manifest_url: &str, video_id: &str) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {video_id}: Registering session"));
    let url = Url::parse("https://sts.postimees.ee/session/register")?;
    let mut response = super::direct::request_expecting(
        client,
        &url,
        &[
            ("Accept", "application/json"),
            ("X-Original-URI", manifest_url),
        ],
        &[],
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::Reported(
            super::request_errnote_text(NAME, video_id, "Unable to register session", &error),
        ),
        error => error,
    })?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

impl Extractor for Duoplay {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let telecast_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `traverse_obj(parse_qs(url), ('ep', 0, {int_or_none}, {str_or_none}))`.
        let episode = url
            .query_pairs()
            .find(|(key, value)| key == "ep" && !value.is_empty())
            .and_then(|(_, value)| value.trim().parse::<i64>().ok())
            .map(|episode| episode.to_string());
        let video_id = match &episode {
            Some(episode) if !episode.is_empty() => {
                format!("{telecast_id}_{episode}")
            }
            _ => telecast_id.clone(),
        };
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let player = crate::html_elements::element_text_and_html_by_tag("video-player", &webpage)
            .ok()
            .map(|(_, whole)| crate::html_elements::extract_attributes(&whole));
        let manifest_url = player
            .as_ref()
            .and_then(|player| player.get("manifest-url"))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_owned);
        let Some(manifest_url) = manifest_url else {
            return Err(Error::Reported(format!(
                "[{NAME}] {telecast_id}: No video found"
            )));
        };
        let session = register_session(client, &manifest_url, &video_id)?;
        let token = match session.get("session") {
            Some(Value::String(token)) => token.clone(),
            Some(other) => other.to_string(),
            None => {
                return Err(Error::Reported(format!(
                    "{telecast_id}: An extractor error has occurred. (caused by KeyError('session')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        // `_parse_json(video_player.get(':episode') or '', ..., fatal=False) or {}`.
        let raw_episode = player
            .as_ref()
            .and_then(|player| player.get(":episode"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let episode_attr = match crate::json::from_str::<Value>(&raw_episode) {
            Ok(Value::Object(map)) if !map.is_empty() => Value::Object(map),
            Ok(_) => Value::Object(Default::default()),
            Err(_) => {
                let detail = crate::json::python_decode_error_text(&raw_episode)
                    .unwrap_or_else(|| "Expecting value".into());
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Failed to parse JSON: {detail}"
                ));
                Value::Object(Default::default())
            }
        };
        let manifest = super::update_url_query(
            &Url::parse(&manifest_url).map_err(|_| {
                Error::UnsupportedHtml5(
                    "Duoplay manifest URLs that are not URLs are not supported natively".into(),
                )
            })?,
            &[("s", token.as_str())],
        );
        let stream = extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, false)?;
        let attr = |key: &str| episode_attr.get(key);
        let text = |key: &str| attr(key).filter(|value| value.is_string()).cloned();
        let mut fields: Vec<(&str, Value)> =
            vec![("id", video_id.clone().into()), ("formats", Value::Null)];
        let mut set = |key: &'static str, value: Option<Value>| {
            let Some(value) = value else {
                return;
            };
            match fields.iter_mut().find(|(name, _)| *name == key) {
                Some((_, old)) => *old = value,
                None => fields.push((key, value)),
            }
        };
        set("title", text("title"));
        set("description", text("synopsis"));
        set(
            "thumbnail",
            episode_attr
                .pointer("/images/original")
                .filter(|value| !value.is_null())
                .cloned(),
        );
        set(
            "timestamp",
            attr("airtime")
                .and_then(Value::as_str)
                .and_then(|airtime| {
                    crate::dates::unified_timestamp(&format!("{airtime} +0200"), false)
                })
                .map(Value::from),
        );
        set(
            "cast",
            attr("cast")
                .filter(|value| super::nuxt::truthy(value))
                .and_then(Value::as_str)
                .map(|cast| Value::Array(cast.split(", ").map(Value::from).collect())),
        );
        set("release_year", int_or_none(attr("year")).map(Value::from));
        if attr("category").and_then(Value::as_str) != Some("movies") {
            let subtitle = text("subtitle").filter(super::nuxt::truthy);
            let episode_title = subtitle.clone().or_else(|| {
                episode
                    .as_ref()
                    .filter(|episode| !episode.is_empty())
                    .map(|episode| Value::from(format!("Episode {episode}")))
            });
            set("title", episode_title);
            set("series", text("title"));
            set(
                "series_id",
                str_or_none(attr("telecast_id")).map(Value::from),
            );
            set(
                "season_number",
                int_or_none(attr("season_id")).map(Value::from),
            );
            set("episode", subtitle);
            set(
                "episode_number",
                int_or_none(attr("episode_nr")).map(Value::from),
            );
            set(
                "episode_id",
                str_or_none(attr("episode_id")).map(Value::from),
            );
        }
        Ok(Some(dict_info(url, (NAME, NAME), fields, stream.formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://duoplay.ee/4312/storm-sorm?ep=2"));
        assert!(suitable("https://duoplay.ee/17/"));
        assert!(!suitable("https://duoplay.ee/sarjad"));
    }
}
