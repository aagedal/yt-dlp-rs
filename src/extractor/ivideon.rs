//! Upstream `IvideonIE` (`yt_dlp/extractor/ivideon.py`): Ivideon TV live
//! cameras as the demo-session FLV streams in three qualities, named from
//! the camera page's config (non-fatal) or its meta tag and heading.
use super::{Extractor, dict_format, dict_info};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Ivideon;

const NAME: &str = "ivideon";
const KEY: &str = "Ivideon";
const QUALITIES: [&str; 3] = ["low", "mid", "hi"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?ivideon\.com/tv/(?:[^/]+/)*camera/(?P<id>\d+-[\da-f]+)/(?P<camera_id>\d+)",
    )
    .unwrap()
});
static CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+config\s*=\s*(\{.+?\});").unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<h1[^>]+class="b-video-title"[^>]*>([^<]+)"#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_download_webpage(..., fatal=False)`: an HTTP error is a warning.
fn optional_webpage(client: &Client, url: &Url, id: &str) -> Result<Option<String>> {
    match super::download_webpage(client, url, NAME, id) {
        Ok(webpage) => Ok(Some(webpage)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download webpage: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

impl Extractor for Ivideon {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let server_id = found["id"].to_owned();
        let camera_id = found["camera_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let camera_url = url.join(&format!("/tv/camera/{server_id}/{camera_id}/"))?;
        let mut camera_name: Option<Value> = None;
        let mut description = Value::Null;
        if let Some(webpage) = optional_webpage(client, &camera_url, &server_id)?
            && !webpage.is_empty()
        {
            if let Some(config) = CONFIG.captures(&webpage).map(|found| found[1].to_owned()) {
                // `_parse_json(..., fatal=False)`.
                match crate::json::from_str::<Value>(&config) {
                    Ok(config) => {
                        let info = config
                            .get("ivTvAppOptions")
                            .and_then(|options| options.get("currentCameraInfo"))
                            .filter(|info| super::nuxt::truthy(info));
                        if let Some(info) = info {
                            camera_name = info.get("camera_name").cloned();
                            description = info
                                .get("misc")
                                .and_then(|misc| misc.get("description"))
                                .cloned()
                                .unwrap_or(Value::Null);
                        }
                    }
                    Err(_) => {
                        let detail = crate::json::python_decode_error_text(&config)
                            .unwrap_or_else(|| "Expecting value".into());
                        crate::screen::warning(&format!(
                            "[{NAME}] {server_id}: Failed to parse JSON: {detail}"
                        ));
                        // Upstream then calls `.get` on the `None` config.
                        return Err(Error::Reported(
                            "'NoneType' object has no attribute 'get'".into(),
                        ));
                    }
                }
            }
            if !camera_name.as_ref().is_some_and(super::nuxt::truthy) {
                camera_name = crate::html::html_search_meta(&webpage, "name")
                    .filter(|name| !name.is_empty())
                    .or_else(|| TITLE.captures(&webpage).map(|found| found[1].to_owned()))
                    .map(Value::from);
            }
        }
        let mut formats = Vec::new();
        for (quality, format_id) in QUALITIES.iter().enumerate() {
            let mut stream = Url::parse("https://streaming.ivideon.com/flv/live")?;
            stream
                .query_pairs_mut()
                .append_pair("server", &server_id)
                .append_pair("camera", &camera_id)
                .append_pair("sessionId", "demo")
                .append_pair("q", &quality.to_string());
            formats.push(dict_format(
                vec![
                    ("url", stream.to_string().into()),
                    ("format_id", (*format_id).into()),
                    ("ext", "flv".into()),
                    ("quality", (quality as i64).into()),
                ],
                NAME,
                &server_id,
            )?);
        }
        let title = camera_name
            .filter(super::nuxt::truthy)
            .unwrap_or_else(|| server_id.clone().into());
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", server_id.into()),
                ("title", title),
                ("description", description),
                ("is_live", true.into()),
                ("formats", Value::Null),
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
            "https://www.ivideon.com/tv/camera/100-916ca13b5c4ad9f564266424a026386d/0/"
        ));
        assert!(suitable(
            "https://www.ivideon.com/tv/map/22.917923/-31.816406/16/camera/100-e7bc16c7d4b5bbd633fd5350b66dfa9a/0"
        ));
        assert!(!suitable("https://www.ivideon.com/tv/camera/x/0"));
    }
}
