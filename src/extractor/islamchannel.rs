//! Upstream `IslamChannelIE` and `IslamChannelSeriesIE`
//! (`yt_dlp/extractor/islamchannel.py`): Islam Channel videos from the
//! Simplestream show-stream API (the page's token, expiry and key) and its
//! tokenized HLS playlist, and series pages as playlists of their shows.
use super::{Extractor, dict_info, download_json_headers, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct IslamChannel;
pub struct IslamChannelSeries;

const NAME: &str = "IslamChannel";
const SERIES_NAME: &str = "IslamChannelSeries";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://watch\.islamchannel\.tv/watch/(?P<id>\d+)").unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://watch\.islamchannel\.tv/series/(?P<id>[a-f\d-]+)").unwrap()
});
static SHOW_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a\s+href="(/watch/\d+)"[^>]+?data-video-type="show">"#).unwrap()
});

/// `IslamChannelIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `IslamChannelSeriesIE._match_id`.
pub fn series_suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn attribute(webpage: &str, attribute: &str) -> Option<String> {
    Regex::new(&format!(r#"{attribute}="([^"]+)""#))
        .ok()?
        .captures(webpage)
        .map(|found| found[1].to_owned())
}

/// `_html_search_meta(names, webpage)`: the first name present, else a
/// warning naming the first.
fn search_meta(webpage: &str, names: &[&str]) -> Option<String> {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name));
    if found.is_none() {
        super::search_warning(NAME, names[0]);
    }
    found
}

impl Extractor for IslamChannel {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let required = |name: &str, label: &str| {
            attribute(&webpage, name).ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract {label}; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })
        };
        let thumbnail = attribute(&webpage, "data-poster")
            .or_else(|| {
                super::search_warning(NAME, "data poster");
                None
            })
            .or_else(|| search_meta(&webpage, &["og:image", "twitter:image"]));
        let token = required("data-token", "data token")?;
        let expiry = required("data-expiry", "data expiry")?;
        let key = required("data-key", "data key")?;
        let headers = [
            ("Token", token.as_str()),
            ("Token-Expiry", expiry.as_str()),
            ("Uvid", video_id.as_str()),
        ];
        let api = super::update_url_query(
            &Url::parse(&format!(
                "https://v2-streams-elb.simplestreamcdn.com/api/show/stream/{video_id}"
            ))?,
            &[("key", key.as_str()), ("platform", "chrome")],
        );
        let show_stream = download_json_headers(client, &api, NAME, &video_id, &headers)?;
        let streams_url = show_stream
            .pointer("/response/tokenization/url")
            .and_then(Value::as_str)
            .and_then(|url| Url::parse(url).ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Islam Channel streams without a tokenization URL are not supported natively"
                        .into(),
                )
            })?;
        let streams = download_json_headers(client, &streams_url, NAME, &video_id, &headers)?;
        let manifest = streams
            .pointer("/Streams/Adaptive")
            .and_then(Value::as_str)
            .and_then(|url| Url::parse(url).ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Islam Channel streams without an adaptive URL are not supported natively"
                        .into(),
                )
            })?;
        let stream =
            super::extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, true)?;
        let title = search_meta(&webpage, &["og:title", "twitter:title"]);
        let description = search_meta(
            &webpage,
            &["og:description", "twitter:description", "description"],
        );
        let thumbnails = match &thumbnail {
            Some(thumbnail) => json!([
                {
                    "id": "unscaled",
                    "url": thumbnail.split('?').next().unwrap_or_default(),
                    "ext": "jpg",
                    "preference": 2,
                },
                {"id": "orig", "url": thumbnail, "ext": "jpg", "preference": 1},
            ]),
            None => Value::Null,
        };
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", optional(title)),
                ("description", optional(description)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("thumbnails", thumbnails),
            ],
            stream.formats,
        );
        info.subtitles = stream.subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=id,url,ext,preference");
        }
        Ok(Some(info))
    }
}

impl Extractor for IslamChannelSeries {
    fn key(&self) -> &'static str {
        SERIES_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        start(SERIES_NAME, url);
        let webpage = download_webpage(client, url, SERIES_NAME, &playlist_id)?;
        // `playlist_from_matches(..., getter=urljoin(url, path), ie=...)`.
        let mut seen: Vec<String> = Vec::new();
        let mut entries = Vec::new();
        for found in SHOW_LINK.captures_iter(&webpage) {
            let target = url.join(&found[1])?.to_string();
            if seen.contains(&target) {
                continue;
            }
            seen.push(target.clone());
            entries.push(super::nrk::url_entry(
                &target,
                NAME,
                None,
                VALID_URL
                    .captures(&target)
                    .map(|found| found["id"].to_owned()),
                None,
            )?);
        }
        Ok(Some(super::nrk::playlist_result(
            url,
            SERIES_NAME,
            playlist_id,
            None,
            None,
            entries,
            true,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://watch.islamchannel.tv/watch/38604310"));
        assert!(series_suitable(
            "https://watch.islamchannel.tv/series/a6cccef3-3ef1-11eb-bc19-06b69c2357cd"
        ));
        assert!(!suitable("https://watch.islamchannel.tv/series/x"));
    }
}
