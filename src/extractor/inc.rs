//! Upstream `IncIE` (`yt_dlp/extractor/inc.py`): an inc.com article's
//! `kaltura_player_ID` element (else its `pageInfo.videos` JSON
//! `vid_kaltura_id`) and partner ID as a `url` result of
//! `kaltura:PARTNER:ENTRY`.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Inc;

const NAME: &str = "Inc";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?inc\.com/(?:[^/]+/)+(?P<id>[^.]+).html")
        .unwrap()
});
static PARTNER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+_?bizo_data_partner_id\s*=\s*["'](\d+)"#).unwrap());
/// `id=(["\'])kaltura_player_(?P<id>.+?)\1`, per quote.
static PLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"id=(?:"kaltura_player_(?P<a>.+?)"|'kaltura_player_(?P<b>.+?)')"#).unwrap()
});
static VIDEOS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"pageInfo\.videos\s*=\s*\[(.+)\];").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Inc {
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
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let partner_id = PARTNER
            .captures(&webpage)
            .map_or_else(|| "1034971".to_owned(), |found| found[1].to_owned());
        let player = PLAYER.captures(&webpage).map(|found| {
            found
                .name("a")
                .or_else(|| found.name("b"))
                .map(|id| id.as_str().to_owned())
                .unwrap_or_default()
        });
        let kaltura_id = match player {
            Some(id) => id,
            None => {
                let videos = VIDEOS.captures(&webpage).ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] {display_id}: Unable to extract kaltura id; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?;
                let video: Value = serde_json::from_str(&videos[1]).map_err(|_| {
                    Error::UnsupportedHtml5(
                        "Inc pageInfo.videos values that are not one JSON object are not supported natively".into(),
                    )
                })?;
                match video.get("vid_kaltura_id") {
                    Some(Value::String(id)) => id.clone(),
                    Some(_) => {
                        return Err(Error::UnsupportedHtml5(
                            "Inc non-string vid_kaltura_id values are not supported natively"
                                .into(),
                        ));
                    }
                    None => {
                        return Err(Error::Reported(format!(
                            "{display_id}: An extractor error has occurred. (caused by KeyError('vid_kaltura_id')); {}",
                            crate::BUG_REPORTS_MESSAGE
                        )));
                    }
                }
            }
        };
        Ok(Some(super::kaltura::follow(
            url,
            format!("kaltura:{partner_id}:{kaltura_id}"),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.inc.com/tip-sheet/bill-gates-says-these-5-books-will-make-you-smarter.html"
        ));
        assert!(!suitable("https://www.inc.com/"));
    }
}
