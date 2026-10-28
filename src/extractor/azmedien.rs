//! Upstream `AZMedienIE` (`yt_dlp/extractor/azmedien.py`): AZ Medien
//! (TeleZüri, TeleBärn, Tele M1, TVO) videos as `url` results of their
//! Kaltura entry: the URL's `#video=` ID, else the page's Apollo state.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct AzMedien;

const NAME: &str = "AZMedien";
const PARTNER_ID: &str = "1719221";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.|tv\.)?(?:telezueri\.ch|telebaern\.tv|telem1\.ch|tvo-online\.ch)/[^/?#]+/(?P<id>[^/?#]+-\d+)(?:\#video=(?P<kaltura_id>[_0-9a-z]+))?",
    )
    .unwrap()
});
static APOLLO_STATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__APOLLO_STATE__\s*=\s*").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for AzMedien {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        let mut entry_id = found.name("kaltura_id").map(|id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        if entry_id.is_none() {
            let webpage = download_webpage(client, url, NAME, &display_id)?;
            let data = super::search_json(&APOLLO_STATE, &webpage, false)
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] {display_id}: Unable to extract video data; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?
                .ok_or_else(|| {
                    Error::UnsupportedHtml5(
                        "AZ Medien video data that do not parse are not supported natively".into(),
                    )
                })?;
            // `(lambda _, v: v['__typename'] == 'KalturaData', 'kalturaId',
            // any, {require('kaltura id')})`.
            let values: Vec<&Value> = match &data {
                Value::Object(items) => items.values().collect(),
                Value::Array(items) => items.iter().collect(),
                _ => Vec::new(),
            };
            let kaltura_id = values
                .into_iter()
                .filter(|item| {
                    item.get("__typename").and_then(Value::as_str) == Some("KalturaData")
                })
                .filter_map(|item| item.get("kalturaId"))
                .find(|id| !id.is_null());
            entry_id = Some(match kaltura_id {
                Some(Value::String(id)) => id.clone(),
                Some(Value::Number(id)) => id.to_string(),
                Some(_) => {
                    return Err(Error::UnsupportedHtml5(
                        "AZ Medien Kaltura IDs that are not strings are not supported natively"
                            .into(),
                    ));
                }
                None => {
                    return Err(Error::Reported(format!(
                        "[{NAME}] {display_id}: Unable to extract kaltura id; {}",
                        crate::BUG_REPORTS_MESSAGE
                    )));
                }
            });
        }
        let entry_id = entry_id.unwrap_or_default();
        Ok(Some(super::follow_url(
            url,
            format!("kaltura:{PARTNER_ID}:{entry_id}"),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://tv.telezueri.ch/sonntalk/bundesrats-vakanzen-eu-rahmenabkommen-133214569"
        ));
        assert!(suitable(
            "https://www.telebaern.tv/telebaern-news/montag-1-oktober-2018-ganze-sendung-133531189#video=0_7xjo9lf1"
        ));
        assert!(!suitable("https://www.telebaern.tv/telebaern-news"));
    }
}
