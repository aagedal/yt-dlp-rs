//! Upstream `CharlieRoseIE` (`yt_dlp/extractor/charlierose.py`): the
//! video player page's first HTML5 media entry (duplicate source URLs
//! removed), titled from its OpenGraph title without the ` - Charlie Rose`
//! suffix, with the OpenGraph thumbnail and description.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct CharlieRose;

const NAME: &str = "CharlieRose";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?charlierose\.com/(?:video|episode)(?:s|/player)/(?P<id>\d+)")
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_og_search_property(prop, webpage, name, fatal=False)`: a warning when
/// absent.
fn og_search(webpage: &str, prop: &str, name: &str) -> Option<String> {
    let value = crate::html::og_search_property(webpage, prop);
    if value.is_none() {
        super::search_warning(NAME, name);
    }
    value
}

impl Extractor for CharlieRose {
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
        let player = Url::parse(&format!("https://charlierose.com/video/player/{video_id}"))
            .expect("valid player URL");
        let webpage = download_webpage(client, &player, NAME, &video_id)?;
        // `remove_end(self._og_search_title(webpage), ' - Charlie Rose')`.
        let title = og_search(&webpage, "title", "OpenGraph title").map(|title| {
            title
                .strip_suffix(" - Charlie Rose")
                .map_or(title.clone(), str::to_owned)
        });
        let entry = super::html5::parse_html5_media_entries(
            client, &player, &webpage, &video_id, NAME, None, None,
        )?
        .into_iter()
        .next()
        // An uncaught `IndexError` is reported as its message.
        .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        // `_remove_duplicate_formats`: the first format for each URL.
        let mut formats: Vec<crate::VideoFormat> = Vec::new();
        for format in entry.formats {
            if !formats.iter().any(|seen| seen.url == format.url) {
                formats.push(format);
            }
        }
        let thumbnail = og_search(&webpage, "image", "thumbnail URL");
        let description = og_search(&webpage, "description", "OpenGraph description");
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        if formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        // The media entry's keys, then `info_dict.update({...})` (`None`
        // values kept).
        let mut keys = vec!["formats", "subtitles", "thumbnail", "id", "title"];
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra.insert("title".into(), Value::Null);
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        keys.push("description");
        if description.is_none() {
            info.extra.insert("description".into(), Value::Null);
        }
        keys.push("_format_sort_fields");
        if thumbnail.is_none() {
            info.extra.insert("thumbnail".into(), Value::Null);
        }
        info.formats = formats;
        info.subtitles = entry.subtitles;
        info.metadata.thumbnail = thumbnail;
        info.metadata.description = description;
        info.format_sort_fields = vec!["proto".into()];
        info.direct = None;
        info.id = video_id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let untitled = info.extra.contains_key(crate::info::TITLE_MISSING);
        let shape = format!(
            "dict{};{}",
            if untitled { "-untitled" } else { "" },
            keys.join(",")
        );
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://charlierose.com/videos/27996"));
        assert!(suitable(
            "https://charlierose.com/episodes/30887?autoplay=true"
        ));
        assert!(suitable("https://www.charlierose.com/video/player/27996"));
        assert!(!suitable("https://charlierose.com/videos/"));
    }
}
