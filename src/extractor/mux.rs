//! Upstream `MuxIE` (`yt_dlp/extractor/mux.py`): a playback ID's fatal
//! `stream.mux.com/ID.m3u8` formats and subtitles (with the URL's
//! `playback-token` as `token`), titled by the ID. Generic-page iframe and
//! `<mux-player>` embeds (and their smuggled titles) are not detected.
use super::{Extractor, dict_info, extract_m3u8_fatal, update_url_query};
use crate::{Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Mux;

const NAME: &str = "Mux";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:stream\.new/v|player\.mux\.com)/(?P<id>[A-Za-z0-9-]+)",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Mux {
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
        // `traverse_obj(parse_qs(url), ('playback-token', -1))`.
        let token = url
            .query_pairs()
            .filter(|(key, _)| key == "playback-token")
            .map(|(_, value)| value.into_owned())
            .last();
        let mut manifest = Url::parse(&format!("https://stream.mux.com/{video_id}.m3u8"))?;
        // `filter_dict({'token': token})` drops only `None`.
        if let Some(token) = &token {
            manifest = update_url_query(&manifest, &[("token", token)]);
        }
        let stream = extract_m3u8_fatal(client, &manifest, NAME, &video_id, None, true)?;
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", video_id.into()),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            stream.formats,
        );
        info.subtitles = stream.subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://stream.new/v/OCtRWZiZqKvLbnZ32WSEYiGNvHdAmB01j/embed"
        ));
        assert!(suitable(
            "https://player.mux.com/OCtRWZiZqKvLbnZ32WSEYiGNvHdAmB01j"
        ));
        assert!(!suitable(
            "https://mux.com/OCtRWZiZqKvLbnZ32WSEYiGNvHdAmB01j"
        ));
    }
}
