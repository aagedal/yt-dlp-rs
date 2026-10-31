//! Upstream `HuffPostIE` (`yt_dlp/extractor/huffpost.py`): HuffPost Live
//! segments from the segments API: its live and live-again sources (HLS,
//! HDS or direct files) with the title, description, running time, start
//! date and sized images; HuffPost Live embed iframes are embeds.
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct HuffPost;

pub(crate) const NAME: &str = "HuffPost";
pub(crate) const KEY: &str = "HuffPost";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(embed\.)?live\.huffingtonpost\.com/(?:r/segment/[^/]+/|HPLEmbedPlayer/\?segmentId=)(?P<id>[0-9a-f]+)",
    )
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+?src=(?:"(?P<dq>https?://embed\.live\.huffingtonpost\.com/.+?)"|'(?P<sq>https?://embed\.live\.huffingtonpost\.com/.+?)')"#,
    )
    .unwrap()
});
static RESOLUTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^.*-([0-9]+x[0-9]+)\.").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Some(raw) = captures.name("dq").or_else(|| captures.name("sq")) else {
            continue;
        };
        let Ok(embed) = url.join(&crate::html::unescape_html(raw.as_str())) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("HuffPost {what} are not supported natively"))
}

impl Extractor for HuffPost {
    fn key(&self) -> &'static str {
        KEY
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
        let api = Url::parse(&format!(
            "http://embed.live.huffingtonpost.com/api/segments/{video_id}.json"
        ))?;
        let response = download_json(client, &api, NAME, &video_id)?;
        let data = response
            .get("data")
            .ok_or_else(|| key_error(&video_id, "data"))?;
        let title = data
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "title"))?;
        let duration = data
            .get("running_time")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
            .map_or(Value::Null, |duration| json!(duration));
        let start = data
            .pointer("/schedule/starts_at")
            .filter(|value| super::nuxt::truthy(value))
            .or_else(|| data.get("segment_start_date_time"));
        let upload_date = start
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_strdate(date, false))
            .map_or(Value::Null, Value::from);
        let description = data.get("description").cloned().unwrap_or(Value::Null);
        let images = match data.get("images") {
            Some(Value::Object(images)) => images,
            Some(_) => return Err(unsupported("image maps that are not objects")),
            None => return Err(key_error(&video_id, "images")),
        };
        let mut thumbnails = Vec::new();
        for image in images.values().filter(|image| super::nuxt::truthy(image)) {
            let Value::String(image) = image else {
                return Err(unsupported("image URLs that are not strings"));
            };
            if let Some(found) = RESOLUTION.captures(image) {
                thumbnails.push(json!({"url": image, "resolution": &found[1]}));
            }
        }
        let sources = data.get("sources");
        let mut live: Vec<(String, Value)> = Vec::new();
        for kind in ["live", "live_again"] {
            match sources.and_then(|sources| sources.get(kind)) {
                None => {}
                Some(Value::Object(items)) => {
                    live.extend(
                        items
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone())),
                    );
                }
                Some(_) => return Err(unsupported("source maps that are not objects")),
            }
        }
        let mut formats = Vec::new();
        for (key, source) in live {
            let Value::String(source) = source else {
                return Err(unsupported("source URLs that are not strings"));
            };
            match super::direct::determine_ext(&source).as_deref() {
                Some("m3u8") => {
                    if let Some(stream) = Url::parse(&source).ok().and_then(|manifest| {
                        super::extract_m3u8_master_optional(
                            client,
                            &manifest,
                            NAME,
                            &video_id,
                            Some("hls"),
                            false,
                            false,
                        )
                    }) {
                        formats.extend(stream.formats);
                    }
                }
                Some("f4m") => {
                    if let Ok(manifest) = Url::parse(&format!("{source}?hdcore=2.9.5")) {
                        formats.extend(super::extract_f4m_optional(
                            client,
                            NAME,
                            &manifest,
                            &video_id,
                            Some("hds"),
                        )?);
                    }
                }
                _ => {
                    formats.push(dict_format(
                        vec![
                            ("format", key.clone().into()),
                            ("format_id", key.replace('/', ".").into()),
                            ("ext", "mp4".into()),
                            ("url", source.into()),
                            (
                                "vcodec",
                                if key.starts_with("audio/") {
                                    "none".into()
                                } else {
                                    Value::Null
                                },
                            ),
                        ],
                        NAME,
                        &video_id,
                    )?);
                }
            }
        }
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("description", description),
                ("formats", Value::Null),
                ("duration", duration),
                ("upload_date", upload_date),
                ("thumbnails", Value::Array(thumbnails)),
            ],
            formats,
        );
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=url,resolution");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://live.huffingtonpost.com/r/segment/legalese-it/52dd3e4b02a7602131000677"
        ));
        assert!(suitable(
            "https://embed.live.huffingtonpost.com/HPLEmbedPlayer/?segmentId=52dd3e4b02a7602131000677"
        ));
        assert!(!suitable("http://www.huffingtonpost.com/entry/x"));
    }
}
