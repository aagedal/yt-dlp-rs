//! Upstream `CaltransIE` (`yt_dlp/extractor/caltrans.py`): a camera page's
//! global script variables and its live HLS stream.
use super::{Extractor, download_webpage, extract_m3u8_fatal_live, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Caltrans;

const NAME: &str = "Caltrans";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:[^/]+\.)?ca\.gov/vm/loc/[^/]+/(?P<id>[a-z0-9_]+)\.htm").unwrap()
});
static GLOBAL_VARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<script[^<]+?([^<]+\.m3u8[^<]+)</script>").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_regex(r'NAME\s*=\s*"([^"]+)"', global_vars, LABEL, fatal=False)`.
fn variable(global_vars: &str, name: &str, label: &str) -> Option<String> {
    let pattern = Regex::new(&format!(r#"{name}\s*=\s*"([^"]+)""#)).expect("static pattern");
    let found = pattern
        .captures(global_vars)
        .map(|found| found[1].to_owned());
    if found.is_none() {
        search_warning(NAME, label);
    }
    found
}

impl Extractor for Caltrans {
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
        let webpage = download_webpage(client, url, NAME, &id)?;
        let global_vars = GLOBAL_VARS
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract Global Vars; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let route_place = variable(&global_vars, "routePlace", "Route Place");
        let location_name = variable(&global_vars, "locationName", "Location Name");
        let poster_url = variable(&global_vars, "posterURL", "Poster Url");
        let video_stream = variable(&global_vars, "videoStreamURL", "Video Stream URL");
        let manifest = video_stream
            .as_deref()
            .and_then(|text| Url::parse(text).ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(format!(
                    "Caltrans pages without an absolute videoStreamURL are not supported natively: {id}"
                ))
            })?;
        let mut info = extract_m3u8_fatal_live(client, &manifest, NAME, &id, None, false)?;
        // `ext='ts'`.
        for format in &mut info.formats {
            format.ext = "ts".into();
        }
        info.ext = "ts".into();
        info.id = id;
        info.title = format!(
            "{} : {}",
            route_place.as_deref().unwrap_or("None"),
            location_name.as_deref().unwrap_or("None")
        );
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.thumbnail = poster_url;
        crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(true));
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,is_live,formats,thumbnail",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://cwwp2.dot.ca.gov/vm/loc/d3/hwy50at24th.htm"
        ));
        assert!(!suitable("https://cwwp2.dot.ca.gov/vm/iframemap.htm"));
    }
}
