//! Upstream `CloudyCDNIE` (`yt_dlp/extractor/cloudycdn.py`): an embed's
//! player JSON (a form POST), its non-fatal HLS sources (video-only chunk
//! lists without audio) and metadata.
use super::{Extractor, dict_info, extract_m3u8_optional};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct CloudyCdn;

const NAME: &str = "CloudyCDN";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?:https?:)?//embed\.(?P<domain>cloudycdn\.services|backscreen\.com)/(?P<site_id>[^/?#]+)/media/(?P<id>[\w-]+)",
    )
    .unwrap()
});
static VIDEO_ONLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"chunklist_b\d+_vo_").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_download_json(url, id, data=urlencode_postdata(...))`.
pub(crate) fn post_json(
    client: &Client,
    url: &Url,
    (extractor, id): (&'static str, &str),
    body: String,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: Downloading JSON metadata"));
    let mut response = super::direct::post(
        client,
        url,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body,
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor,
            id: id.to_owned(),
            what: "JSON metadata",
            source: Box::new(error),
        },
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
            "[{extractor}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

impl Extractor for CloudyCdn {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (domain, site_id, id) = (&found["domain"], &found["site_id"], found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://player.{domain}/player/{site_id}/media/{id}/"
        ))?;
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("version", "6.4.0")
            .append_pair("referer", url.as_str())
            .finish();
        let data = post_json(client, &api, (NAME, &id), body)?;
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        // `('source', 'sources', ..., 'src', {url_or_none})`.
        let sources: Vec<String> = match data.pointer("/source/sources") {
            Some(Value::Array(items)) => items.iter().collect::<Vec<_>>(),
            Some(Value::Object(items)) => items.values().collect(),
            _ => Vec::new(),
        }
        .into_iter()
        .filter_map(|source| source.get("src")?.as_str())
        .filter_map(super::json_ld::url_or_none)
        .collect();
        for source in sources {
            let manifest = Url::parse(&source)?;
            let Some(stream) = extract_m3u8_optional(client, &manifest, NAME, &id, None, true)
            else {
                continue;
            };
            for mut format in stream.formats {
                if VIDEO_ONLY.is_match(format.url.as_str()) {
                    format.acodec = Some("none".into());
                    crate::info_order::mark_format_update(&mut format, "acodec");
                }
                formats.push(format);
            }
            super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
        }
        let mut fields = vec![
            ("id", Value::from(id.clone())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        if let Some(title) = data.get("name").filter(|name| name.is_string()) {
            fields.push(("title", title.clone()));
        }
        let duration = super::streamable::int_or_none(data.get("duration"));
        if !duration.is_null() {
            fields.push(("duration", duration));
        }
        if let Some(timestamp) = data
            .get("upload_date")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.push(("timestamp", timestamp.into()));
        }
        if let Some(thumbnail) = data
            .pointer("/source/poster")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            fields.push(("thumbnail", thumbnail.into()));
        }
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://embed.cloudycdn.services/ltv/media/46k_d23-6000-105?"
        ));
        assert!(suitable(
            "https://embed.backscreen.com/ltv/media/32j_z25-0600-127?"
        ));
        assert!(!suitable("https://embed.cloudycdn.services/ltv/"));
    }
}
