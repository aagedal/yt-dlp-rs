//! Upstream `MedalTVIE` (`yt_dlp/extractor/medaltv.py`): a clip's
//! `medal.tv/api/content/ID` JSON, its fatal HLS and source HTTP formats
//! (privacy-protected guest URLs dropped), the `socialVideoUrl` redirect
//! fallback and the clip metadata.
use super::{Extractor, dict_format, dict_info, download_json_headers, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MedalTv;

const NAME: &str = "MedalTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?medal\.tv/games/[^/?#&]+/clips/(?P<id>[^/?#&]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `_request_webpage(url, video_id, note=...)`: the final URL.
fn final_url(client: &Client, url: &Url, id: &str, note: &str) -> Result<Url> {
    crate::screen::screen(&format!("[{NAME}] {id}: {note}"));
    super::direct::request_with_headers(client, url, None, &[])
        .map(|response| response.url().clone())
        .map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: NAME,
                id: id.to_owned(),
                what: "webpage",
                source: Box::new(error),
            },
            error => error,
        })
}

impl Extractor for MedalTv {
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
        let api = Url::parse(&format!("https://medal.tv/api/content/{id}"))?;
        let data =
            download_json_headers(client, &api, NAME, &id, &[("Accept", "application/json")])?;

        let mut formats = Vec::new();
        if let Some(m3u8_url) = url_or_none(data.get("contentUrlHls")) {
            let manifest = Url::parse(&m3u8_url)?;
            formats.extend(
                extract_m3u8_fatal_master(client, &manifest, NAME, &id, Some("hls"), false)?
                    .formats,
            );
        }
        if let Some(http_url) = url_or_none(data.get("contentUrl")) {
            formats.push(dict_format(
                vec![
                    ("url", http_url.into()),
                    ("format_id", "http-source".into()),
                    ("ext", "mp4".into()),
                    ("quality", 1.into()),
                ],
                NAME,
                &id,
            )?);
        }
        formats.retain(|format| {
            !format
                .url
                .as_str()
                .contains("video/privacy-protected-guest")
        });
        if formats.is_empty() {
            crate::screen::warning(&format!(
                "[{NAME}] Video formats are not available through API, falling back to social video URL"
            ));
            let social = Url::parse(&format!("https://medal.tv/api/content/{id}/socialVideoUrl"))?;
            let social = final_url(client, &social, &id, "Checking social video URL")?;
            formats.push(dict_format(
                vec![
                    ("url", social.as_str().into()),
                    ("format_id", "social-video".into()),
                    ("ext", "mp4".into()),
                    ("quality", (-1).into()),
                ],
                NAME,
                &id,
            )?);
        }

        let text = |key: &str| data.get(key).filter(|value| value.is_string()).cloned();
        let int = |value: Option<&Value>| {
            Some(super::streamable::int_or_none(value)).filter(|value| !value.is_null())
        };
        // `int_or_none(scale=1000)`: floor division.
        let timestamp = int(data.get("created"))
            .and_then(|value| value.as_i64())
            .map(|created| Value::from(created.div_euclid(1000)));
        let poster = data.get("poster");
        let uploader_id = poster
            .and_then(|poster| poster.get("userId"))
            .filter(|id| id.is_string())
            .cloned();
        let uploader_url = uploader_id
            .as_ref()
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(|id| Value::from(format!("https://medal.tv/users/{id}")));
        // `('tags', ..., {str})`: dict templates drop an empty result.
        let tags = match data.get("tags") {
            Some(Value::Array(tags)) => Some(tags.iter().collect::<Vec<_>>()),
            Some(Value::Object(tags)) => Some(tags.values().collect()),
            _ => None,
        }
        .map(|tags| {
            Value::Array(
                tags.into_iter()
                    .filter(|tag| tag.is_string())
                    .cloned()
                    .collect(),
            )
        })
        .filter(|tags| tags.as_array().is_some_and(|tags| !tags.is_empty()));
        let mut fields = vec![("id", Value::from(id.as_str())), ("formats", Value::Null)];
        // `traverse_obj(content_data, {...})`: the present fields.
        for (key, value) in [
            ("title", text("contentTitle")),
            ("description", text("contentDescription")),
            ("timestamp", timestamp),
            ("duration", int(data.get("videoLengthSeconds"))),
            ("view_count", int(data.get("views"))),
            ("like_count", int(data.get("likes"))),
            ("comment_count", int(data.get("comments"))),
            (
                "uploader",
                poster
                    .and_then(|poster| poster.get("displayName"))
                    .filter(|name| name.is_string())
                    .cloned(),
            ),
            ("uploader_id", uploader_id),
            ("uploader_url", uploader_url),
            ("tags", tags),
            (
                "thumbnail",
                url_or_none(data.get("thumbnailUrl")).map(Value::from),
            ),
        ] {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://medal.tv/games/valorant/clips/jTBFnLKdLy15K"
        ));
        assert!(suitable(
            "https://www.medal.tv/games/cod-cold-war/clips/2mA60jWAGQCBH"
        ));
        assert!(!suitable("https://medal.tv/games/valorant"));
    }
}
