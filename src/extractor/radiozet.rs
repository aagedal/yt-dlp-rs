//! Upstream `RadioZetPodcastIE` (`yt_dlp/extractor/radiozet.py`): a podcast
//! page's player `data-id` and its `getPodcast` API entry as an untyped
//! result.
use super::{Extractor, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct RadioZetPodcast;

const NAME: &str = "RadioZetPodcast";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://player\.radiozet\.pl/Podcasty/.*?/(?P<id>.+)").unwrap()
});
static PODCAST_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div.*?\sid="player".*?\sdata-id=['"]([^'"]+)['"]"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `traverse_obj(value, path)` over dict keys and list indices.
fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |value, key| match value {
            Value::Object(map) => map.get(*key),
            Value::Array(items) => items.get(key.parse::<usize>().ok()?),
            _ => None,
        })
        .filter(|value| !value.is_null())
}

/// `strip_or_none`.
fn strip(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(|text| text.trim().to_owned())
}

impl Extractor for RadioZetPodcast {
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
        // `_html_search_regex(..., 'podcast id')`.
        let podcast_id = PODCAST_ID
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract podcast id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let api = Url::parse(&format!(
            "https://player.radiozet.pl/api/podcasts/getPodcast/(node)/{podcast_id}/(station)/radiozet"
        ))?;
        let response = download_json(client, &api, NAME, &display_id)?;
        let crash = |cause: &str| {
            Error::Reported(format!(
                "{display_id}: An extractor error has occurred. (caused by {cause}); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        // `['data'][0]`.
        let data = match response.get("data") {
            Some(Value::Array(items)) => items
                .first()
                // A bare `IndexError` is reported with its message only.
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "RadioZet podcast data that is not a list is not supported natively".into(),
                ));
            }
            None => return Err(crash("KeyError('data')")),
        };

        let media = match at(data, &["player", "stream"]) {
            None => None,
            Some(Value::String(media)) => Some(media.clone()),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "RadioZet stream URLs that are not strings are not supported natively".into(),
                ));
            }
        };
        let mut info = match media {
            Some(media) => {
                let media = Url::parse(&media)?;
                let ext = super::direct::determine_ext(media.as_str())
                    .unwrap_or_else(|| "unknown_video".into());
                let mut info = super::direct::with_format(
                    super::direct::build_at(media.clone(), &media, ext.clone()),
                    "",
                );
                for format in &mut info.formats {
                    format.ext = ext.clone();
                    format.format_id = String::new();
                }
                info.format_id = None;
                info
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {podcast_id}: No video formats found!; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        info.id = podcast_id;
        let title = strip(data.get("title"));
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let mut keys = vec!["id", "display_id", "title"];
        crate::metadata_parser::set_field(&mut info, "display_id", display_id.into());
        for (key, value) in [
            (
                "description",
                strip(at(data, &["program", "desc"])).map(Value::from),
            ),
            ("release_timestamp", at(data, &["published_date"]).cloned()),
            ("url", Some(Value::Null)),
            (
                "thumbnail",
                at(data, &["program", "image", "original"]).cloned(),
            ),
            ("duration", at(data, &["player", "duration"]).cloned()),
            (
                "series",
                strip(at(data, &["program", "title"])).map(Value::from),
            ),
            (
                "creator",
                strip(at(data, &["presenter", "0", "title"])).map(Value::from),
            ),
        ] {
            // Every key is set, `None` included (processing drops a `None`
            // `duration`).
            if key == "duration" && value.is_none() {
                continue;
            }
            keys.push(key);
            if key != "url" {
                crate::metadata_parser::set_field(&mut info, key, value.unwrap_or(Value::Null));
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("untyped-dict;{}", keys.join(",")),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://player.radiozet.pl/Podcasty/Nie-Ma-Za-Co/O-przedmiotach-szkolnych-ktore-przydaja-sie-w-zyciu"
        ));
        assert!(!suitable("https://player.radiozet.pl/Podcasty/x"));
        assert!(!suitable("https://radiozet.pl/Podcasty/a/b"));
    }
}
