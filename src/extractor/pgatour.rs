//! Upstream `PGATourIE` (`yt_dlp/extractor/pgatour.py`): PGA Tour videos
//! (and TOURCast `T` videos) as `url` results of their Brightcove player.
use super::Extractor;
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

pub struct PgaTour;

const NAME: &str = "PGATour";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?pgatour\.com/video/[\w-]+/(?P<tc>T)?(?P<id>\d+)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for PgaTour {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let tourcast = found.name("tc").is_some();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let (account_id, player_id) = if tourcast {
            ("6116716431001", "Vsd5Umu8r")
        } else {
            ("6082840763001", "FWIBYMBPj")
        };
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_URL.into(),
            format!(
                "https://players.brightcove.net/{account_id}/{player_id}_default/index.html?videoId={video_id}"
            )
            .into(),
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
            "https://www.pgatour.com/video/competition/T6322447785112/adam-hadwin-2023-the-players"
        ));
        assert!(suitable(
            "https://www.pgatour.com/video/features/6322506425112/follow-the-players"
        ));
        assert!(!suitable("https://www.pgatour.com/news"));
    }
}
