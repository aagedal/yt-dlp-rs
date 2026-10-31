//! Upstream `ClipchampIE` (`yt_dlp/extractor/clipchamp.py`): a watch page's
//! next.js `video` data, the Cloudflare Stream player iframe's customer
//! subdomain and its DASH/HLS manifests.
use super::clyp::key_error;
use super::{
    Extractor, dict_info, download_webpage, download_webpage_note, extract_m3u8_master_optional,
    extract_mpd_optional, search_nextjs_data,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Clipchamp;

const NAME: &str = "Clipchamp";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?clipchamp\.com/watch/(?P<id>[\w-]+)").unwrap()
});
static SUBDOMAIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\bcustomer-domain-prefix=(?:"(?P<a>[\w-]+)"|'(?P<b>[\w-]+)')"#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Clipchamp {what} are not supported natively"))
}

/// Python's `repr`-free `str()` of a JSON value in an f-string.
fn py_str(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".into(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(true)) => "True".into(),
        Some(Value::Bool(false)) => "False".into(),
        Some(other) => other.to_string(),
    }
}

impl Extractor for Clipchamp {
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
        let next = search_nextjs_data(&webpage, NAME, &video_id)?;
        let mut data = &next;
        for key in ["props", "pageProps", "video"] {
            data = data.get(key).ok_or_else(|| key_error(&video_id, key))?;
        }
        if !data.is_object() {
            return Err(unsupported("video data that are not objects"));
        }
        let storage_location = data.get("storage_location");
        if storage_location.and_then(Value::as_str) != Some("cf_stream") {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unsupported clip storage location \"{}\"; {}",
                py_str(storage_location),
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let path = match data.get("download_url") {
            None => return Err(key_error(&video_id, "download_url")),
            Some(Value::String(path)) => path.clone(),
            Some(_) => return Err(unsupported("download URLs that are not strings")),
        };
        let iframe_url = Url::parse(&format!("https://iframe.cloudflarestream.com/{path}"))?;
        let iframe = download_webpage_note(
            client,
            &iframe_url,
            NAME,
            &video_id,
            "Downloading player iframe",
        )?;
        let subdomain = SUBDOMAIN
            .captures(&iframe)
            .and_then(|found| found.name("a").or_else(|| found.name("b")))
            .map(|found| found.as_str().to_owned());
        if subdomain.is_none() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract subdomain; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let subdomain = subdomain.unwrap_or_else(|| "customer-2ut9yn3y6fta1yxe".into());
        let manifest = |ext: &str| {
            Url::parse(&format!(
                "https://{subdomain}.cloudflarestream.com/{path}/manifest/video.{ext}"
            ))
            .map(|url| super::update_url_query(&url, &[("parentOrigin", "https://clipchamp.com")]))
        };
        let mut formats = Vec::new();
        if let Some(stream) =
            extract_mpd_optional(client, &manifest("mpd")?, NAME, &video_id, false)
        {
            for mut format in stream.formats {
                format.format_id = format!("dash-{}", format.format_id);
                formats.push(format);
            }
        }
        if let Some(stream) = extract_m3u8_master_optional(
            client,
            &manifest("m3u8")?,
            NAME,
            &video_id,
            Some("hls"),
            false,
            false,
        ) {
            formats.extend(stream.formats);
        }
        let names: Vec<&str> = ["first_name", "last_name"]
            .iter()
            .filter_map(|key| data.get("creator")?.get(key)?.as_str())
            .collect();
        let uploader = names.join(" ");
        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("formats", Value::Null),
            (
                "uploader",
                if uploader.is_empty() {
                    Value::Null
                } else {
                    uploader.into()
                },
            ),
        ];
        if let Some(title) = data
            .get("project")
            .and_then(|project| project.get("project_name"))
            .and_then(Value::as_str)
        {
            fields.push(("title", title.into()));
        }
        if let Some(timestamp) = data
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, true))
        {
            fields.push(("timestamp", timestamp.into()));
        }
        if let Some(thumbnail) = data
            .get("thumbnail_url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            fields.push(("thumbnail", thumbnail.into()));
        }
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://clipchamp.com/watch/gRXZ4ZhdDaU"));
        assert!(!suitable("https://clipchamp.com/en/"));
    }
}
