//! Upstream `NowCanalIE` (`yt_dlp/extractor/nowcanal.py`): NOW Canal pages
//! as `url` results of their Brightcove video.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct NowCanal;

const NAME: &str = "NowCanal";
const KEY: &str = "NowCanal";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?nowcanal\.pt(?:/[\w-]+)+/detalhe/(?P<id>[\w-]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for NowCanal {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = super::download_webpage(client, url, NAME, &id)?;
        static VIDEO_DATA: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"videoHandler\.addBrightcoveVideoWithJson\(\[\s*").unwrap()
        });
        let data = super::search_json(&VIDEO_DATA, &webpage, false)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract video data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "NOW Canal video data that do not parse are not supported natively".into(),
                )
            })?;
        let video_id = match data.get("brightcoveVideoId") {
            Some(Value::String(video_id)) => video_id.clone(),
            Some(Value::Number(video_id)) => video_id.to_string(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "NOW Canal video IDs that are not strings are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "{id}: An extractor error has occurred. (caused by KeyError('brightcoveVideoId')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        Ok(Some(super::follow_url(
            url,
            format!(
                "https://players.brightcove.net/6108484330001/chhIqzukMq_default/index.html?videoId={video_id}"
            ),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.nowcanal.pt/ultimas/detalhe/pedro-sousa-hjulmand-pode-ter-uma-saida-limpa-do-sporting-daqui-a-um-ano"
        ));
        assert!(!suitable("https://www.nowcanal.pt/ultimas"));
    }
}
