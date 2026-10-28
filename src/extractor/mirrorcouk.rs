//! Upstream `MirrorCoUKIE` (`yt_dlp/extractor/mirrorcouk.py`): Mirror
//! articles as transparent JW Platform results of the page's
//! `json-placeholder` `videoData.videoId`, with the article ID as
//! `display_id`.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct MirrorCoUk;

const NAME: &str = "MirrorCoUK";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?mirror\.co\.uk/[/+\[\w-]+-(?P<id>\d+)").unwrap()
});
static DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"div\s+class="json-placeholder"\s+data-json=""#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Mirror {what} are not supported natively"))
}

impl Extractor for MirrorCoUk {
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
        // `_search_json(..., transform_source=unescapeHTML)` (fatal).
        let data = match super::search_json_detailed(&DATA, &webpage, &crate::html::unescape_html) {
            Some(Ok(data)) => data,
            Some(Err(error)) => {
                let cause = error.map_or_else(String::new, |error| {
                    format!(
                        " (caused by JSONDecodeError({}))",
                        crate::batch::py_repr(&error)
                    )
                });
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract data - Failed to parse JSON{cause}; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract data; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let Value::Object(data) = data else {
            return Err(unsupported("video data that are not objects"));
        };
        let video = match data.get("videoData") {
            Some(Value::Object(video)) => video,
            Some(_) => return Err(unsupported("video data that are not objects")),
            None => return Err(key_error(&display_id, "videoData")),
        };
        let video_id = match video.get("videoId") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            Some(_) => return Err(unsupported("video IDs that are not strings")),
            None => return Err(key_error(&display_id, "videoId")),
        };
        let target = format!("jwplatform:{video_id}");
        // `ie_key='JWPlatform'` only extracts URLs it is suitable for.
        if !super::jwplatform::suitable(&target) {
            return Err(unsupported("video IDs that are not JW Platform IDs"));
        }
        let mut fields = Map::new();
        fields.insert("display_id".into(), display_id.into());
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.mirror.co.uk/tv/tv-news/love-island-fans-baffled-after-27163139"
        ));
        assert!(suitable("https://mirror.co.uk/news/[x+y-12"));
        assert!(!suitable("https://www.mirror.co.uk/news/no-id"));
    }
}
