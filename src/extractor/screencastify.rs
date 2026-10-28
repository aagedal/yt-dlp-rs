//! Upstream `ScreencastifyIE` (`yt_dlp/extractor/screencastify.py`):
//! Screencastify watch pages from the umbrella service's watch JSON: its
//! DASH and HLS manifests requested with the manifest's auth query, which
//! is added to every format URL and kept as `extra_param_to_segment_url`.
use super::{Extractor, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Screencastify;

const NAME: &str = "Screencastify";

static VALID_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"^https?://watch\.screencastify\.com/v/(?P<id>[^/?#]+)").unwrap(),
        Regex::new(r"^https?://app\.screencastify\.com/v[23]/watch/(?P<id>[^/?#]+)").unwrap(),
    ]
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URLS.iter().any(|regex| regex.is_match(url))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Screencastify {what} are not supported natively"))
}

/// `urllib.parse.parse_qs(query)`: blank values dropped, repeated keys
/// grouped in first-seen order.
fn parse_qs(query: &str) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if value.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, values)) => values.push(value.into_owned()),
            None => groups.push((key.into_owned(), vec![value.into_owned()])),
        }
    }
    groups
}

/// A truthy `traverse_obj(info, ('manifest', KEY))` manifest URL.
fn manifest_url(info: &Value, key: &str) -> Result<Option<String>> {
    match info.get("manifest").and_then(|manifest| manifest.get(key)) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(url)) if url.is_empty() => Ok(None),
        Some(Value::String(url)) => Ok(Some(url.clone())),
        Some(_) => Err(unsupported("manifest URLs that are not strings")),
    }
}

impl Extractor for Screencastify {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URLS
            .iter()
            .find_map(|regex| regex.captures(url.as_str()))
        else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://umbrella.svc.screencastify.com/api/umbrellaService/watch/{video_id}"
        ))?;
        let info = download_json(client, &api, NAME, &video_id)?;
        if !info.is_object() {
            return Err(unsupported("watch responses that are not objects"));
        }
        let query_string = info
            .pointer("/manifest/auth/query")
            .cloned()
            .unwrap_or(Value::Null);
        let query = match &query_string {
            Value::Null => Vec::new(),
            Value::String(query) => parse_qs(query),
            _ => return Err(unsupported("auth queries that are not strings")),
        };
        let query: Vec<(&str, Vec<&str>)> = query
            .iter()
            .map(|(key, values)| (key.as_str(), values.iter().map(String::as_str).collect()))
            .collect();
        let with_query = |url: &Url| super::update_url_query_lists(url, &query);
        let parse = |manifest: &str| {
            Url::parse(manifest)
                .map(|manifest| with_query(&manifest))
                .map_err(|_| unsupported("manifest URLs that are not URLs"))
        };
        let mut formats = Vec::new();
        if let Some(manifest) = manifest_url(&info, "url")? {
            let manifest = parse(&manifest)?;
            if let Some(mut stream) = super::extract_mpd_optional_note(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("dash"),
                "Downloading MPD manifest",
            ) {
                // `_extract_mpd_formats` drops the manifest's subtitles.
                super::ignore_subtitles(&mut stream, NAME, "DASH");
                formats.extend(stream.formats);
            }
        }
        if let Some(manifest) = manifest_url(&info, "hlsUrl")? {
            let manifest = parse(&manifest)?;
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("hls"),
                false,
                false,
            ) {
                formats.extend(stream.formats);
            }
        }
        for format in &mut formats {
            format.url = with_query(&format.url);
        }
        let get = |key: &str| info.get(key).cloned().unwrap_or(Value::Null);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", get("title")),
                ("description", get("description")),
                ("uploader", get("userName")),
                ("formats", Value::Null),
                ("extra_param_to_segment_url", query_string),
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
        assert!(suitable(
            "https://watch.screencastify.com/v/sYVkZip3quLKhHw4Ybk8"
        ));
        assert!(suitable(
            "https://app.screencastify.com/v2/watch/BQ26VbUdfbQLhKzkktOk"
        ));
        assert!(!suitable("https://app.screencastify.com/v4/watch/x"));
    }

    #[test]
    fn parse_qs_drops_blank_values_and_groups_keys() {
        assert_eq!(
            parse_qs("a=1&b=&a=2&c=x+y%20"),
            vec![
                ("a".to_owned(), vec!["1".to_owned(), "2".to_owned()]),
                ("c".to_owned(), vec!["x y ".to_owned()]),
            ]
        );
    }
}
