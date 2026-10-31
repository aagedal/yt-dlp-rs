//! Upstream `CellebriteIE` (`yt_dlp/extractor/cellebrite.py`): Cellebrite
//! pages' first Vidyard embed, processed like `VidyardIE`'s single chapter,
//! with the page's OpenGraph description (the video's when it has one), the
//! old `cellebrite` archive ID and the OpenGraph image as an extra thumbnail.
use super::{Extractor, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Cellebrite;

const NAME: &str = "Cellebrite";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://cellebrite\.com/(?:\w+)?/(?P<id>[\w-]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Errors carry the page's temporary ID.
fn key_error(slug: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{slug}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for Cellebrite {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let slug = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &slug)?;
        let Some(vidyard_url) = super::vidyard::embed_urls(url, &webpage).into_iter().next() else {
            return Err(Error::Reported(format!(
                "[{NAME}] {slug}: No Vidyard video embeds found on page; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        // `url_basename(vidyard_url)`.
        let video_id = vidyard_url
            .path()
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let api = Url::parse(&format!("https://play.vidyard.com/player/{video_id}.json"))?;
        let data = download_json(client, &api, NAME, &video_id)?;
        let payload = data
            .get("payload")
            .ok_or_else(|| key_error(&slug, "payload"))?;
        let chapter = match payload.get("chapters") {
            Some(Value::Array(chapters)) => chapters
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "Vidyard chapter lists that are not lists are not supported natively".into(),
                ));
            }
            None => return Err(key_error(&slug, "chapters")),
        };
        let (mut fields, formats, subtitles) = super::vidyard::video_json_fields(
            client, chapter, &video_id, NAME,
        )
        .map_err(|error| match error {
            Error::Reported(message) => Error::Reported(message.replacen(
                &format!("{video_id}: An extractor error"),
                &format!("{slug}: An extractor error"),
                1,
            )),
            error => error,
        })?;
        let display_id = fields
            .iter()
            .find(|(key, _)| *key == "display_id")
            .and_then(|(_, value)| value.as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        if let Some(display_id) = display_id {
            fields.push((
                "_old_archive_ids",
                json!([format!("{} {display_id}", NAME.to_lowercase())]),
            ));
        }
        if let Some(thumbnail) = crate::html::og_search_property(&webpage, "image") {
            match fields.iter_mut().find(|(key, _)| *key == "thumbnails") {
                Some((_, Value::Array(thumbnails))) => thumbnails.push(json!({"url": thumbnail})),
                _ => fields.push(("thumbnails", json!([{"url": thumbnail}]))),
            }
        }
        // `{'description': og_search_description(default=None), **info}`.
        let description = crate::html::og_search_property(&webpage, "description");
        // The key comes first; the video's own description keeps its value.
        let description = match fields.iter().position(|(key, _)| *key == "description") {
            Some(index) => fields.remove(index).1,
            None => description.map_or(Value::Null, Value::from),
        };
        fields.insert(0, ("description", description));
        Ok(Some(super::vidyard::video_json_info(
            url,
            (NAME, NAME),
            fields,
            formats,
            subtitles,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://cellebrite.com/en/how-to-lawfully-collect-the-maximum-amount-of-data-from-android-devices/"
        ));
        assert!(!suitable("https://cellebrite.com/"));
    }
}
