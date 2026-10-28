//! Upstream `TVIPlayerIE` (`yt_dlp/extractor/tviplayer.py`): TVI Player
//! videos and episodes from the page's `opts = {video: [...]}` player JSON,
//! its `videoUrl` HLS signed with the `wmsAuthSign` token fetched once per
//! run, with Open Graph fallbacks and the JSON-LD `TVEpisode` season.
use super::{Extractor, dict_info, download_webpage, download_webpage_note};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct TviPlayer;

const NAME: &str = "TVIPlayer";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://tviplayer\.iol\.pt(/programa/[\w-]+/[a-f0-9]+)?/\w+/(?P<id>\w+)")
        .unwrap()
});
static JSON_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\sopts\s*=\s*\{\s*video\s*:\s*\[").unwrap());

/// `_real_initialize`'s `wmsAuthSign` token.
static TOKEN: Mutex<Option<String>> = Mutex::new(None);

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none` of a JSON value.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

/// `json_data.get(KEY) or FALLBACK` for string values.
fn text_or(
    data: &Value,
    key: &str,
    fallback: impl FnOnce() -> Option<String>,
) -> Result<Option<String>> {
    match data.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
        Some(value) if super::nuxt::truthy(value) => Err(Error::UnsupportedHtml5(format!(
            "TVI Player {key} values that are not strings are not supported natively"
        ))),
        _ => Ok(fallback()),
    }
}

impl Extractor for TviPlayer {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_real_initialize`: the `wmsAuthSign` token (once per run).
    fn real_initialize(&self, client: &Client, _session: &super::login::Session) -> Result<()> {
        let url = Url::parse("https://services.iol.pt/matrix?userId=")?;
        let token = download_webpage_note(
            client,
            &url,
            NAME,
            "wmsAuthSign",
            "Trying to get wmsAuthSign token",
        )?;
        if let Ok(mut slot) = TOKEN.lock() {
            *slot = Some(token);
        }
        Ok(())
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        self.initialize(client, NAME, &video_id)?;
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let json_data = super::search_json(&JSON_DATA, &webpage, true)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract JSON data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "TVI Player JSON data that do not parse are not supported natively".into(),
                )
            })?;
        let video_url = match json_data.get("videoUrl") {
            Some(Value::String(video_url)) => video_url.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "TVI Player video URLs that are not strings are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "{video_id}: An extractor error has occurred. (caused by KeyError('videoUrl')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let token = TOKEN
            .lock()
            .ok()
            .and_then(|token| token.clone())
            .unwrap_or_default();
        let manifest = Url::parse(&format!("{video_url}?wmsAuthSign={token}")).map_err(|_| {
            Error::UnsupportedHtml5(
                "TVI Player video URLs that are not URLs are not supported natively".into(),
            )
        })?;
        let stream =
            super::extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, true)?;
        let search = |value: Option<String>, name: &str| {
            if value.is_none() {
                super::search_warning(NAME, name);
            }
            value
        };
        let title = text_or(&json_data, "title", || {
            search(
                crate::html::og_search_property(&webpage, "title"),
                "OpenGraph title",
            )
        })?;
        let thumbnail = text_or(&json_data, "cover", || {
            search(
                crate::html::og_search_property(&webpage, "image"),
                "thumbnail URL",
            )
        })?;
        // `(lambda _, v: v['@type'] == 'TVEpisode', 'seasonNumber', {int_or_none}, any)`.
        let season_number = super::json_ld::yield_json_ld(&webpage)
            .iter()
            .filter(|item| item.get("@type").and_then(Value::as_str) == Some("TVEpisode"))
            .find_map(|item| int_or_none(item.get("seasonNumber")));
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let number = |value: Option<i64>| value.map_or(Value::Null, Value::from);
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", optional(title)),
                ("thumbnail", optional(thumbnail)),
                ("duration", number(int_or_none(json_data.get("duration")))),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("season_number", number(season_number)),
                (
                    "episode_number",
                    number(int_or_none(json_data.get("episodeNum"))),
                ),
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
            "https://tviplayer.iol.pt/programa/jornal-das-8/53c6b3903004dc006243d0cf/video/61c8e8b90cf2c7ea0f0f71a9"
        ));
        assert!(suitable(
            "https://tviplayer.iol.pt/video/62c4131c0cf2f9a86eac06bb"
        ));
        assert!(suitable(
            "https://tviplayer.iol.pt/programa/para-sempre/61716c360cf2365a5ed894c4/episodio/t1e187"
        ));
        assert!(!suitable("https://tviplayer.iol.pt/video"));
        assert!(!suitable("https://iol.pt/video/abc"));
    }
}
