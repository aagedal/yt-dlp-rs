//! Upstream `TeamTreeHouseIE` (`yt_dlp/extractor/teamtreehouse.py`): a library
//! video page's HTML5 media entry (sources, captions and poster), the preview
//! warning and duration, and the page's title and description. Course and
//! workshop pages (their `url_transparent` playlists) and the sign-in are not
//! ported; those pages fail with an explicit error.
use super::html5::parse_html5_media_entries;
use super::{Extractor, dict_info, direct, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct TeamTreeHouse;

const NAME: &str = "TeamTreeHouse";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?teamtreehouse\.com/library/(?P<id>[^/]+)")
        .unwrap()
});
const PREVIEW: &str = r#"data-preview="true""#;
static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-duration="(\d+)""#).unwrap());
static DURATION_TEXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"id="video-duration"[^>]*>([^<]*)<"#).unwrap());
static WORKSHOP_VIDEOS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<ul[^>]+id="workshop-videos"[^>]*>(.+?)</ul>"#).unwrap());
static STAGES_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div[^>]+id="syllabus-stages"[^>]+data-url="([^"]+)""#).unwrap()
});

/// `_VALID_URL`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// The first `<meta>` of `names` that has content, as `_html_search_meta`.
fn html_search_meta(webpage: &str, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name))
}

impl Extractor for TeamTreeHouse {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
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
        let fail = |name: &str| {
            Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let title = html_search_meta(&webpage, &["og:title", "twitter:title"]);
        let description = html_search_meta(
            &webpage,
            &["description", "og:description", "twitter:description"],
        );
        let entries =
            parse_html5_media_entries(client, url, &webpage, &display_id, NAME, None, None)?;
        let Some(mut entry) = entries.into_iter().next() else {
            // Course and workshop pages list `url_transparent` entries that
            // are not ported.
            let is_playlist = WORKSHOP_VIDEOS.is_match(&webpage);
            if !is_playlist && !STAGES_PATH.is_match(&webpage) {
                return Err(fail("stages path"));
            }
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: course and workshop pages are not supported yet"
            )));
        };
        // `determine_ext(subtitle['url'], 'srt')`.
        for (_, tracks) in &mut entry.subtitles.entries {
            for track in tracks {
                track.ext =
                    direct::determine_ext(track.url.as_str()).unwrap_or_else(|| "srt".into());
            }
        }

        let is_preview = webpage.contains(PREVIEW);
        let duration = if is_preview {
            crate::screen::warning(&format!(
                "[{NAME}] {display_id}: This is just a preview. You need to be signed in with a Basic account to download the entire video."
            ));
            Value::from(30)
        } else {
            let milliseconds = DURATION
                .captures(&webpage)
                .map(|found| found[1].to_owned())
                .ok_or_else(|| fail("duration"))?;
            let seconds = milliseconds
                .parse::<f64>()
                .ok()
                .map(|value| value / 1000.0)
                .filter(|value| *value != 0.0);
            match seconds {
                Some(seconds) => serde_json::json!(seconds),
                // `if not duration: duration = parse_duration(video-duration)`.
                None => DURATION_TEXT
                    .captures(&webpage)
                    .and_then(|found| crate::parse_duration(&found[1]))
                    .and_then(|duration| serde_json::to_value(duration).ok())
                    .unwrap_or(Value::Null),
            }
        };

        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let fields = vec![
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("thumbnail", optional(entry.thumbnail.clone())),
            ("id", display_id.clone().into()),
            ("title", optional(title)),
            ("description", optional(description)),
            ("duration", duration),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, entry.formats);
        info.subtitles = entry.subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://teamtreehouse.com/library/application-overview-2"
        ));
        assert!(suitable(
            "https://teamtreehouse.com/library/introduction-to-user-authentication-in-php"
        ));
        assert!(!suitable("https://teamtreehouse.com/"));
    }
}
