//! Upstream `DiscogsReleasePlaylistIE` (`yt_dlp/extractor/discogs.py`):
//! Discogs releases and masters as playlists of their API videos' YouTube
//! `url` results, with the release title.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct DiscogsReleasePlaylist;

const NAME: &str = "DiscogsReleasePlaylist";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?discogs\.com/(?P<type>release|master)/(?P<id>\d+)").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Discogs {what} are not supported natively"))
}

impl Extractor for DiscogsReleasePlaylist {
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
        let playlist_id = found["id"].to_owned();
        let playlist_type = found["type"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let display_id = format!("{playlist_type}{playlist_id}");
        let api = Url::parse(&format!(
            "https://api.discogs.com/{playlist_type}s/{playlist_id}"
        ))?;
        let response = download_json(client, &api, NAME, &display_id)?;
        // `('videos', lambda _, v: YoutubeIE.suitable(v['uri']))`: failing
        // items are skipped.
        let videos: Vec<&Value> = match response.get("videos") {
            Some(Value::Array(videos)) => videos.iter().collect(),
            Some(Value::Object(videos)) => videos.values().collect(),
            _ => Vec::new(),
        };
        let mut entries = Vec::new();
        for video in videos {
            let Some(uri) = video
                .get("uri")
                .and_then(Value::as_str)
                .filter(|uri| super::youtube::suitable(uri))
            else {
                continue;
            };
            let title = match video.get("title") {
                None | Some(Value::Null) => None,
                Some(Value::String(title)) => Some(title.clone()),
                Some(_) => return Err(unsupported("video titles that are not strings")),
            };
            entries.push(super::nrk::url_entry(
                uri,
                "Youtube",
                None,
                super::youtube::temp_id(uri),
                title,
            )?);
        }
        let title = match response.get("title") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => Some(title.clone()).filter(|title| !title.is_empty()),
            Some(_) => return Err(unsupported("release titles that are not strings")),
        };
        Ok(Some(super::nrk::playlist_result(
            url, NAME, display_id, title, None, entries, false,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.discogs.com/release/1-The-Persuader-Stockholm"
        ));
        assert!(suitable(
            "https://www.discogs.com/master/113-Vince-Watson-Moments-In-Time"
        ));
        assert!(!suitable("https://www.discogs.com/artist/1"));
    }
}
