//! Upstream `Funker530IE` (`yt_dlp/extractor/funker530.py`): Funker530 video
//! pages as `url_transparent` results of their first Rumble (else YouTube)
//! embed, with the page's description before `About the Author`.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Funker530;

const NAME: &str = "Funker530";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?funker530\.com/video/(?P<id>[^/?#]+)").unwrap()
});
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)(.+)About the Author").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_element_by_class(class_name, html)`: the first element's content.
fn element_by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(text, _)| text)
}

impl Extractor for Funker530 {
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
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = super::download_webpage(client, url, NAME, &display_id)?;
        let target = super::rumble::first_embed_url(&webpage)
            .and_then(|embed| url.join(&embed).ok())
            .or_else(|| super::youtube::embed_urls(url, &webpage).into_iter().next());
        let Some(target) = target else {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: No videos found on webpage"
            )));
        };
        // `_search_regex(..., clean_html(get_element_by_class(...)),
        // default=None)`: no element, no description.
        let description =
            element_by_class("video-desc-paragraph", &webpage).and_then(|paragraph| {
                DESCRIPTION
                    .captures(&crate::html::clean_html(&paragraph))
                    .map(|found| found[1].trim_matches(crate::batch::is_py_space).to_owned())
            });
        let mut fields = Map::new();
        fields.insert("description".into(), description.into());
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target.as_str(), "fields": Value::Object(fields)}),
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
            "https://funker530.com/video/azov-patrol-caught-in-open-under-automatic-grenade-launcher-fire/"
        ));
        assert!(!suitable("https://funker530.com/videos/"));
    }
}
