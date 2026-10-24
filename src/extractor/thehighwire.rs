//! Upstream `TheHighWireIE` (`yt_dlp/extractor/thehighwire.py`): the
//! `ark-video-embed` iframe page's first HTML5 media entry, with the
//! page's section header and episode description.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct TheHighWire;

const NAME: &str = "TheHighWire";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?thehighwire\.com/ark-videos/(?P<id>[^/?#]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `{find_element(cls=CLASS)}` (`html=False`: the content; `True`: the
/// element), `None` when absent or unparsable.
fn find_element(class: &str, html: &str, whole: bool) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(text, element)| if whole { element } else { text })
}

impl Extractor for TheHighWire {
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
        // `({find_element(cls='ark-video-embed', html=True)},
        // {extract_attributes}, 'src', {url_or_none}, {require('embed URL')})`.
        let embed = find_element("ark-video-embed", &webpage, true)
            .map(|element| crate::html_elements::extract_attributes(&element))
            .and_then(|attributes| match attributes.get("src") {
                Some(Value::String(src)) => super::json_ld::url_or_none(src),
                _ => None,
            })
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract embed URL; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let embed = Url::parse(&embed).map_err(|_| {
            Error::UnsupportedHtml5("TheHighWire embed URLs that do not parse".into())
        })?;
        let embed_page = download_webpage(client, &embed, NAME, &display_id)?;
        let title = find_element("section-header", &webpage, false)
            .map(|text| crate::html::clean_html(&text));
        let description = find_element("episode-description__copy", &webpage, false)
            .map(|text| crate::html::clean_html(&text));
        let entry = super::html5::parse_html5_media_entries(
            client,
            &embed,
            &embed_page,
            &display_id,
            NAME,
            Some("hls"),
            None,
        )?
        .into_iter()
        .next()
        .ok_or_else(|| {
            Error::Reported(format!(
                "{display_id}: An extractor error has occurred. (caused by IndexError('list index out of range')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let first = entry
            .formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        if entry.formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        // `{'id', **page_fields, **media_entry}`.
        let mut keys = vec!["id"];
        match title {
            Some(title) => {
                info.title = title;
                keys.push("title");
            }
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        if description.is_some() {
            keys.push("description");
        }
        // The entry's `thumbnail` is set even without a poster.
        keys.extend(["formats", "subtitles", "thumbnail"]);
        info.formats = entry.formats;
        info.subtitles = entry.subtitles;
        info.metadata.thumbnail = entry.thumbnail;
        info.metadata.description = description;
        info.direct = None;
        info.id = display_id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let untitled = !keys.contains(&"title");
        let shape = format!(
            "dict;{}{}",
            keys.join(","),
            if untitled { "-untitled" } else { "" }
        );
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://thehighwire.com/ark-videos/the-deposition-of-stanley-plotkin/"
        ));
        assert!(!suitable("https://thehighwire.com/ark-videos/"));
    }
}
