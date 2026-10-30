//! Upstream `NFHSNetworkIE` (`yt_dlp/extractor/nfhsnetwork.py`): NFHS
//! Network events from the `cfunity` game/event API (the first publisher's
//! broadcast or VOD) and the stream URL API's HLS playlist.
use super::{Extractor, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NfhsNetwork;

const NAME: &str = "NFHSNetwork";
const API: &str = "https://cfunity.nfhsnetwork.com/v2";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?nfhsnetwork\.com/events/[\w-]+/(?P<id>(?:gam|evt|dd|)?[\w\d]{0,10})",
    )
    .unwrap()
});
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<h1 class="sr-hidden">(.*?)</h1>"#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("NFHS Network {what} are not supported natively"))
}

/// Python truthiness.
fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(super::nuxt::truthy)
}

/// `str(value)` in a format string.
fn py_str(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".into(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(true)) => "True".into(),
        Some(Value::Bool(false)) => "False".into(),
        Some(Value::Number(number)) => match number.as_i64() {
            Some(number) => number.to_string(),
            None => crate::screen::py_float(number.as_f64().unwrap_or_default()),
        },
        Some(_) => "<object>".into(),
    }
}

/// `value[0]` of a `.get(...)` result: Python's errors for `None` and
/// empty lists.
fn first(value: Option<&Value>) -> Result<&Value> {
    match value {
        None | Some(Value::Null) => Err(Error::Reported(
            "'NoneType' object is not subscriptable".into(),
        )),
        Some(Value::Array(items)) => items
            .first()
            .ok_or_else(|| Error::Reported("list index out of range".into())),
        Some(_) => Err(unsupported("lists that are not lists")),
    }
}

impl Extractor for NfhsNetwork {
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
        let data = download_json(
            client,
            &Url::parse(&format!("{API}/game_or_event/{video_id}"))?,
            NAME,
            &video_id,
        )?;
        let publisher = first(data.get("publishers"))?;
        let Some(publisher) = publisher.as_object() else {
            return Err(unsupported("publishers that are not objects"));
        };
        // Some (older) videos don't have a broadcasts object.
        let broadcast = if truthy(publisher.get("broadcasts")) {
            first(publisher.get("broadcasts"))?
        } else {
            first(publisher.get("vods"))?
        };
        let Some(broadcast) = broadcast.as_object() else {
            return Err(unsupported("broadcasts that are not objects"));
        };
        let uploader = if truthy(publisher.get("formatted_name")) {
            publisher.get("formatted_name")
        } else {
            publisher.get("name")
        };
        let uploader_prefix = match publisher.get("type") {
            Some(Value::String(kind)) if kind == "school" => "schools",
            Some(Value::String(kind)) if kind.contains("association") => "associations",
            Some(Value::String(kind)) if kind == "publisher" || kind == "affiliate" => "affiliates",
            Some(Value::String(_)) => "schools",
            None | Some(Value::Null) => {
                return Err(Error::Reported(
                    "argument of type 'NoneType' is not iterable".into(),
                ));
            }
            Some(_) => return Err(unsupported("publisher types that are not strings")),
        };
        let uploader_page = format!(
            "https://www.nfhsnetwork.com/{uploader_prefix}/{}",
            py_str(publisher.get("slug"))
        );
        let location = format!(
            "{}, {}",
            py_str(data.get("city")),
            py_str(data.get("state_name"))
        );
        let is_live = truthy(broadcast.get("on_air"))
            || broadcast.get("status").and_then(Value::as_str) == Some("on_air");
        if broadcast
            .get("on_air")
            .is_some_and(|on_air| truthy(Some(on_air)) && !on_air.is_boolean())
        {
            return Err(unsupported("on-air flags that are not booleans"));
        }
        let start_time = match data.get("local_start_time") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.as_str()),
            Some(_) => return Err(unsupported("start times that are not strings")),
        };
        let timestamp = start_time.and_then(|text| crate::dates::unified_timestamp(text, true));
        let upload_date = start_time.and_then(|text| crate::dates::unified_strdate(text, true));

        // `_og_search_title(webpage)` (non-fatal), else the fatal heading.
        let og_title = crate::html::og_search_property(&webpage, "title");
        if og_title.is_none() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract OpenGraph title; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let title = match og_title.filter(|title| !title.is_empty()) {
            Some(title) => title,
            None => TITLE
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]))
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] {video_id}: Unable to extract title; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?,
        };
        let title = crate::batch::py_strip(title.split('|').next().unwrap_or_default()).to_owned();

        let (video_type, key) = if is_live {
            ("broadcasts", broadcast.get("key"))
        } else {
            (
                "vods",
                publisher
                    .get("vods")
                    .and_then(|vods| vods.get(0)?.get("key")),
            )
        };
        let key = py_str(key);
        let stream = download_json(
            client,
            &Url::parse(&format!("{API}/{video_type}/{key}/url"))?,
            NAME,
            &video_id,
        )?;
        let m3u8_url = match stream.get("video_url") {
            Some(Value::String(text)) => text.clone(),
            _ => return Err(unsupported("stream responses without a video URL")),
        };
        let m3u8_url =
            Url::parse(&m3u8_url).map_err(|_| unsupported("video URLs that are not URLs"))?;
        let formats = if is_live {
            super::extract_m3u8_fatal_live(client, &m3u8_url, NAME, &video_id, None, false)?
        } else {
            super::extract_m3u8_fatal_master(client, &m3u8_url, NAME, &video_id, None, false)?
        }
        .formats;
        let get = |value: Option<&Value>| value.cloned().unwrap_or(Value::Null);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("formats", Value::Null),
                ("description", get(broadcast.get("description"))),
                ("timestamp", timestamp.map_or(Value::Null, Value::from)),
                ("uploader", get(uploader)),
                ("uploader_id", get(publisher.get("publisher_key"))),
                ("uploader_url", uploader_page.into()),
                ("location", location.into()),
                ("upload_date", upload_date.map_or(Value::Null, Value::from)),
                ("is_live", is_live.into()),
                ("_format_sort_fields", json!(["res", "tbr"])),
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
            "https://www.nfhsnetwork.com/events/rockford-high-school-rockford-mi/gamcf7e54cfbc"
        ));
        assert!(suitable(
            "https://www.nfhsnetwork.com/events/wiaa-wi/9aa2f92f82"
        ));
        assert!(!suitable("https://www.nfhsnetwork.com/schools/x"));
    }
}
