//! Upstream `HolodexIE` (`yt_dlp/extractor/holodex.py`): a Holodex watch page
//! as a YouTube `url` result (or its playlist when a `playlist` is given and
//! `--no-playlist` is not), and a Holodex playlist API's video IDs as YouTube
//! `url` entries.
use super::{Extractor, download_json, follow_url, nrk::url_entry};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Holodex;

const NAME: &str = "Holodex";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.|staging\.)?holodex\.net/(?:api/v2/playlist/(?P<playlist>\d+)|watch/(?P<id>[\w-]{11})(?:\?(?:[^#]+&)?playlist=(?P<playlist2>\d+))?)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Holodex {
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
        let video_id = found.name("id").map(|m| m.as_str().to_owned());
        let playlist_id = found
            .name("playlist")
            .or_else(|| found.name("playlist2"))
            .map(|m| m.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        // `_yes_playlist(playlist_id, video_id)`.
        let yes_playlist = match (&playlist_id, &video_id) {
            (Some(playlist_id), Some(video_id)) => {
                if super::no_playlist() {
                    crate::screen::screen(&format!(
                        "[{NAME}] Downloading just the video {video_id} because of --no-playlist"
                    ));
                    false
                } else {
                    crate::screen::screen(&format!(
                        "[{NAME}] Downloading playlist {playlist_id} - add --no-playlist to download just the video {video_id}"
                    ));
                    true
                }
            }
            (Some(_), None) => true,
            (None, _) => false,
        };
        let playlist_id = playlist_id.filter(|_| yes_playlist);
        let Some(playlist_id) = playlist_id else {
            let video_id = video_id.unwrap_or_default();
            return Ok(Some(follow_url(
                url,
                format!("https://www.youtube.com/watch?v={video_id}"),
            )));
        };

        let api = Url::parse(&format!(
            "https://holodex.net/api/v2/playlist/{playlist_id}"
        ))?;
        let data = download_json(client, &api, NAME, &playlist_id)?;
        // `traverse_obj(data, ('videos', ..., 'id'))`, then `orderedSet`.
        let mut ids: Vec<String> = Vec::new();
        if let Some(Value::Array(videos)) = data.get("videos") {
            for id in videos.iter().filter_map(|video| video.get("id")) {
                if let Value::String(id) = id
                    && !ids.contains(id)
                {
                    ids.push(id.clone());
                }
            }
        }
        let entries = ids
            .iter()
            .map(|id| url_entry(id, "Youtube", None, None, None))
            .collect::<Result<Vec<_>>>()?;
        // `playlist_result(entries, playlist_id, data.get('name'))`: a falsy
        // title is left out.
        let title = data
            .get("name")
            .filter(|name| super::nuxt::truthy(name))
            .and_then(|name| name.as_str().map(str::to_owned));
        let keys = if title.is_some() { "id,title" } else { "id" };
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://holodex.net/watch/9kQ2GtvDV3s"));
        assert!(suitable("https://holodex.net/api/v2/playlist/239"));
        assert!(suitable("https://staging.holodex.net/watch/s1ifBeukThg"));
        assert!(!suitable("https://holodex.net/"));
    }
}
