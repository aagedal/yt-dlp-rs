//! Upstream `CanalsurmasIE` (`yt_dlp/extractor/canalsurmas.py`): Canal Sur
//! Más videos from the RTVA API. A demo account's JWT (renewed within five
//! minutes of its expiry) authorizes the content metadata and its resource
//! files: HLS masters give their formats and subtitles, other files plain
//! formats.
use super::Extractor;
use super::{dict_format, dict_info, download_json_note_headers, post_json_errnote};
use super::{request_errnote_text, update_url_query};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

pub struct Canalsurmas;

const NAME: &str = "Canalsurmas";
const API_BASE: &str = "https://api-rtva.interactvty.com";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?canalsurmas\.es/videos/(?P<id>\d+)").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_is_jwt_expired(token)`: `exp` less than five minutes away.
fn is_expired(token: &str, id: &str) -> Result<bool> {
    let exp = super::podbayfm::jwt_payload(token)
        .as_ref()
        .and_then(|payload| payload.get("exp"))
        .and_then(Value::as_f64)
        .ok_or_else(|| super::clyp::key_error(id, "exp"))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    Ok(exp - now < 300.0)
}

/// The access token, requested again when missing or about to expire.
fn access_token(client: &Client, cached: &mut Option<String>, id: &str) -> Result<String> {
    if let Some(token) = cached.as_ref()
        && !is_expired(token, id)?
    {
        return Ok(token.clone());
    }
    let url = Url::parse(&format!("{API_BASE}/jwt/token/"))?;
    let body = json!({ "username": "canalsur_demo", "password": "dsUBXUcI" }).to_string();
    let response = post_json_errnote(
        client,
        &url,
        (NAME, ""),
        (
            "Downloading access token",
            "Failed to download access token",
        ),
        &[("Content-Type", "application/json")],
        body,
    )?;
    let token = response
        .get("access")
        .and_then(Value::as_str)
        .ok_or_else(|| super::clyp::key_error(id, "access"))?
        .to_owned();
    *cached = Some(token.clone());
    Ok(token)
}

/// `_call_api(endpoint, video_id, fields)`.
fn call_api(
    client: &Client,
    cached: &mut Option<String>,
    endpoint: &str,
    video_id: &str,
    fields: Option<&[&str]>,
) -> Result<Value> {
    let token = access_token(client, cached, video_id)?;
    let mut url = Url::parse(&format!(
        "{API_BASE}/api/2.0/contents/{endpoint}/{video_id}/"
    ))?;
    if let Some(fields) = fields {
        let optional = fields.join(",");
        url = update_url_query(&url, &[("optional_fields", optional.as_str())]);
    }
    let authorization = format!("jwtok {token}");
    let note = format!("Downloading {endpoint} API JSON");
    let errnote = format!("Failed to download {endpoint} API JSON");
    download_json_note_headers(
        client,
        &url,
        NAME,
        video_id,
        &note,
        &[("Authorization", authorization.as_str())],
    )
    .map_err(|error| match error {
        Error::ExtractorRequest { source, .. } => {
            Error::Reported(request_errnote_text(NAME, video_id, &errnote, &source))
        }
        error => error,
    })
}

impl Extractor for Canalsurmas {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = found["id"].to_owned();
        let mut token = None;
        let video_info = call_api(
            client,
            &mut token,
            "content",
            &video_id,
            Some(&["description", "image", "duration", "created_at", "tags"]),
        )?;
        let stream_info = call_api(
            client,
            &mut token,
            "content_resources",
            &video_id,
            Some(&["media_url"]),
        )?;

        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let resources = stream_info
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for stream_url in resources
            .filter_map(|resource| resource.get("media_url").and_then(Value::as_str))
            .filter_map(super::json_ld::url_or_none)
        {
            if super::direct::determine_ext(&stream_url).as_deref() == Some("m3u8") {
                let manifest = Url::parse(&stream_url)?;
                if let Some(stream) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("hls"),
                    true,
                    false,
                ) {
                    formats.extend(stream.formats);
                    super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            } else {
                formats.push(dict_format(
                    vec![("url", stream_url.into())],
                    NAME,
                    &video_id,
                )?);
            }
        }

        let get = |key: &str| video_info.get(key);
        let title = get("name")
            .and_then(Value::as_str)
            .map(|name| name.trim().to_owned());
        let description = get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let thumbnail = get("image")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let duration = get("duration").and_then(|value| match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.parse().ok(),
            _ => None,
        });
        let timestamp = get("created_at")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601);
        let tags: Vec<Value> = get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(Value::from)
            .collect();

        // `**traverse_obj(...)`: the keys whose values are missing (or an
        // empty tag list) are left out.
        let mut fields = vec![
            ("id", Value::from(video_id.as_str())),
            ("formats", Value::Null),
            // The subtitles are set on the info below (their key's place).
            ("subtitles", Value::Object(Default::default())),
        ];
        let optional = [
            ("title", title.map(Value::from)),
            ("description", description.map(Value::from)),
            ("thumbnail", thumbnail.map(Value::from)),
            ("duration", duration.map(Value::from)),
            ("timestamp", timestamp.map(Value::from)),
            ("tags", (!tags.is_empty()).then(|| Value::Array(tags))),
        ];
        fields.extend(
            optional
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        );
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_match_upstream_valid_url() {
        assert!(suitable(
            "https://www.canalsurmas.es/videos/44006-el-gran-queo-1-lora-del-rio-sevilla-20072014"
        ));
        assert!(suitable("https://canalsurmas.es/videos/44006"));
        assert!(!suitable("https://www.canalsurmas.es/programas/44006"));
    }
}
