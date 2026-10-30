//! Upstream `UlizaPlayerIE` and `UlizaPortalIE` (`yt_dlp/extractor/uliza.py`):
//! the ULIZA player-api page's `prog-index.m3u8` master playlist (fatal,
//! master only, with a live status from the `/hls/(dvr|video)/` path) and
//! the ulizaportal.jp page that hands its embedded player URL over as a
//! transparent result (with the expiry check on its `expires` parameter).
use super::{Extractor, dict_info, download_webpage_note_headers, extract_m3u8_fatal_master};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct UlizaPlayer;
pub struct UlizaPortal;

const PLAYER_NAME: &str = "UlizaPlayer";
const PORTAL_NAME: &str = "UlizaPortal";

static PLAYER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https://player-api\.p\.uliza\.jp/v1/players/[^?#]+\?(?:[^#]*&)?name=(?P<id>[^#&]+)",
    )
    .unwrap()
});
static PORTAL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https?://(?:www\.)?ulizaportal\.jp/pages/(?P<id>[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12})",
    )
    .unwrap()
});
static M3U8_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"["'](https://vms-api\.p\.uliza\.jp/v1/prog-index\.m3u8[^"']+)"#).unwrap()
});
static PLAYER_SRC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<script [^>]*\bsrc="(https://player-api\.p\.uliza\.jp/v1/players/[^"]+)""#)
        .unwrap()
});
static M3U8_TYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/hls/(dvr|video)/").unwrap());

/// `_match_valid_url` of the player (its `_match_id`).
pub fn player_suitable(url: &str) -> bool {
    PLAYER_URL.is_match(url)
}

/// `_match_valid_url` of the portal.
pub fn portal_suitable(url: &str) -> bool {
    PORTAL_URL.is_match(url)
}

/// `_search_regex(..., fatal)`: the first group, else an extractor error.
fn search_fatal(regex: &Regex, text: &str, id: &str, what: &str) -> Result<String> {
    regex
        .captures(text)
        .and_then(|found| found.get(1))
        .map(|found| found.as_str().to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{PLAYER_NAME}] {id}: Unable to extract {what}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

/// `make_archive_id('PIAULIZAPortal', video_id)`.
fn archive_id(video_id: &str) -> String {
    format!("piaulizaportal {video_id}")
}

impl Extractor for UlizaPlayer {
    fn key(&self) -> &'static str {
        "UlizaPlayer"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = PLAYER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(PLAYER_NAME);
        crate::screen::screen(&format!(
            "[{PLAYER_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let player_data = download_webpage_note_headers(
            client,
            url,
            PLAYER_NAME,
            &display_id,
            "Fetching player data",
            &[("Referer", "https://player-api.p.uliza.jp/")],
        )?;
        let m3u8_url = search_fatal(&M3U8_URL, &player_data, &display_id, "m3u8 url")?;
        let manifest = Url::parse(&m3u8_url)?;
        let video_id = manifest
            .query_pairs()
            .find(|(key, _)| key == "ss")
            .map_or_else(|| display_id.clone(), |(_, value)| value.into_owned());

        let mut stream =
            extract_m3u8_fatal_master(client, &manifest, PLAYER_NAME, &video_id, None, false)?;
        // `_extract_m3u8_formats` without `ext`.
        super::hls::without_ext(&mut stream);
        // `m3u8_type` is taken from the first format's URL.
        let m3u8_type = stream
            .formats
            .first()
            .and_then(|format| M3U8_TYPE.captures(format.url.as_str()))
            .map(|found| found[1].to_owned());
        // `'dvr'` is short-term archives; VOD and long-term archives are `not_live`.
        let live_status = match m3u8_type.as_deref() {
            Some("video") => "is_live",
            Some("dvr") => "was_live",
            _ => "not_live",
        };

        let fields = vec![
            ("id", Value::from(video_id.as_str())),
            ("title", Value::from(video_id.as_str())),
            ("formats", Value::Null),
            ("live_status", Value::from(live_status)),
            ("_old_archive_ids", json!([archive_id(&video_id)])),
        ];
        Ok(Some(dict_info(
            url,
            (PLAYER_NAME, "UlizaPlayer"),
            fields,
            stream.formats,
        )))
    }
}

impl Extractor for UlizaPortal {
    fn key(&self) -> &'static str {
        "UlizaPortal"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PORTAL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(PORTAL_NAME);
        crate::screen::screen(&format!(
            "[{PORTAL_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        // `expires` is only checked when the link carries it (`parse_qs` then
        // `int_or_none`).
        let expires = url
            .query_pairs()
            .find(|(key, _)| key == "expires")
            .and_then(|(_, value)| value.parse::<i64>().ok());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
            });
        if expires.is_some_and(|expires| expires != 0 && expires <= now) {
            return Err(Error::Reported(format!(
                "[{PORTAL_NAME}] {video_id}: The link is expired"
            )));
        }

        let webpage = super::download_webpage(client, url, PORTAL_NAME, &video_id)?;
        let player_data_url = search_fatal_portal(&PLAYER_SRC, &webpage, &video_id)?;
        // `url_result(player_data_url, UlizaPlayerIE, url_transparent=True,
        // display_id=video_id, video_title=title)`: `video_title` is the
        // result's `title` when there is one.
        let target = Url::parse(&player_data_url)?;
        let mut overlay = json!({"display_id": video_id});
        if let Some(title) = crate::html::html_extract_title(&webpage) {
            overlay["title"] = Value::from(title);
        }
        let mut follow = super::direct::build_at(target.clone(), &target, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": player_data_url, "fields": overlay}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

/// `_search_regex(player data url)` on the portal page (fatal).
fn search_fatal_portal(regex: &Regex, webpage: &str, id: &str) -> Result<String> {
    regex
        .captures(webpage)
        .and_then(|found| found.get(1))
        .map(|found| found.as_str().to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{PORTAL_NAME}] {id}: Unable to extract player data url; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(player_suitable(
            "https://player-api.p.uliza.jp/v1/players/timeshift-disabled/pia/admin?type=normal&playerobjectname=ulizaPlayer&name=livestream01_dvr&repeatable=true"
        ));
        assert!(!player_suitable(
            "https://player-api.p.uliza.jp/v1/players/default-player/pia/admin?type=normal"
        ));
        assert!(portal_suitable(
            "https://ulizaportal.jp/pages/005f18b7-e810-5618-cb82-0987c5755d44"
        ));
        assert!(!portal_suitable("https://ulizaportal.jp/pages/not-an-id"));
    }
}
