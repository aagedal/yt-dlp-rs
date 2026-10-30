//! Upstream `GBNewsIE` (`yt_dlp/extractor/gbnews.py`): the page's last
//! non-sidebar `simplestream` player element, its Simplestream player API
//! host and stream JSON, and the stream's HLS formats.
use super::{Extractor, dict_info, download_json_note, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct GbNews;

const NAME: &str = "GBNews";
const PLATFORM: &str = "safari";
const SSMP_URL: &str = "https://mm-v2.simplestream.com/ssmp/api.php";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?gbnews\.(?:uk|com)/(?:\w+/)?(?P<id>[^#?]+)")
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("GB News {what} are not supported natively"))
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// An attribute's text (`None` for a missing or value-less attribute).
fn attribute<'a>(attributes: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    attributes.get(name).and_then(Value::as_str)
}

/// The last `simplestream` element whose class does not contain `sidebar`.
fn video_data(webpage: &str) -> Result<Option<Map<String, Value>>> {
    let elements = crate::html_elements::elements_text_and_html_by_class("simplestream", webpage)
        .map_err(|_| unsupported("player elements that do not parse"))?;
    let mut video_data = None;
    for (_, html) in elements {
        let attributes = crate::html_elements::extract_attributes(&html);
        if !attribute(&attributes, "class")
            .unwrap_or_default()
            .contains("sidebar")
        {
            video_data = Some(attributes);
        }
    }
    Ok(video_data.filter(|data| !data.is_empty()))
}

/// `_get_ss_endpoint(data_id, data_env)`: the player's API host.
fn ss_endpoint(client: &Client, data_id: &str, data_env: &str, temp_id: &str) -> Result<String> {
    let api = super::update_url_query(
        &Url::parse(SSMP_URL)?,
        &[("id", data_id), ("env", data_env)],
    );
    let json = download_json_note(
        client,
        &api,
        NAME,
        "",
        "Downloading Simplestream JSON metadata",
    )?;
    json.get("response")
        .and_then(|response| response.get("api_hostname"))
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {temp_id}: No API host found; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

impl Extractor for GbNews {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        let display_id = temp_id.rsplit('/').next().unwrap_or_default().to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;

        let Some(video_data) = video_data(&webpage)? else {
            return Err(Error::Reported(format!(
                "[{NAME}] {temp_id}: Could not find video element"
            )));
        };
        let data_id = attribute(&video_data, "data-id")
            .filter(|id| !id.is_empty())
            .unwrap_or("GB003");
        let data_env = attribute(&video_data, "data-env")
            .filter(|env| !env.is_empty())
            .unwrap_or("production");
        let endpoint_url = ss_endpoint(client, data_id, data_env, &temp_id)?;

        let uvid = match video_data.get("data-uvid") {
            Some(Value::String(uvid)) => uvid.clone(),
            Some(_) => return Err(unsupported("value-less data-uvid attributes")),
            None => {
                return Err(Error::Reported(format!(
                    "{temp_id}: An extractor error has occurred. (caused by KeyError('data-uvid')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let video_type = match attribute(&video_data, "data-type") {
            None | Some("" | "vod") => "show",
            Some(video_type) => video_type,
        };
        // `query={'key': None}` is urlencoded as `None`.
        let key = attribute(&video_data, "data-key").unwrap_or("None");
        let stream_api = super::update_url_query(
            &Url::parse(&format!("{endpoint_url}/api/{video_type}/stream/{uvid}"))
                .map_err(|_| unsupported("API hosts that do not parse"))?,
            &[("key", key), ("platform", PLATFORM)],
        );
        let stream_data =
            download_json_note(client, &stream_api, NAME, &uvid, "Downloading stream JSON")?;
        if stream_data.get("drm").is_some_and(truthy) {
            // `report_drm`: `raise_no_formats(..., expected=True)`.
            crate::no_formats::raise_expected(
                NAME,
                Some(&uvid),
                None,
                "This video is DRM protected",
            )?;
        }

        let stream = stream_data
            .get("response")
            .and_then(|response| response.get("stream"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .ok_or_else(|| unsupported("stream responses without a stream URL"))?;
        let manifest = Url::parse(&stream)?;
        let mut formats =
            super::extract_m3u8_fatal_master(client, &manifest, NAME, &uvid, None, false)?.formats;
        for format in &mut formats {
            if format.ext.is_empty() || format.ext == "unknown_video" {
                format.ext = "mp4".into();
            }
        }

        let og = |property: &str| {
            crate::html::og_search_property(&webpage, property).map_or(Value::Null, Value::from)
        };
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", uvid.into()),
                ("display_id", display_id.into()),
                ("title", og("title")),
                ("description", og("description")),
                ("formats", Value::Null),
                ("thumbnail", og("image")),
                ("is_live", (video_type == "live").into()),
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
            "https://www.gbnews.com/news/bbc-claudine-gay-harvard-university-antisemitism-row"
        ));
        assert!(suitable("https://www.gbnews.uk/watchlive"));
        assert!(!suitable("https://www.gbnews.com/"));
    }

    #[test]
    fn last_non_sidebar_player_wins() {
        let page = r#"<div class="simplestream vod" data-uvid="1"></div><div class="simplestream live_player" data-uvid="2"></div><div class="simplestream sidebar" data-uvid="3"></div>"#;
        let data = video_data(page).unwrap().unwrap();
        assert_eq!(attribute(&data, "data-uvid"), Some("2"));
        assert!(video_data("<div></div>").unwrap().is_none());
    }
}
