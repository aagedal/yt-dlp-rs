//! Upstream `FilmArchivIE` (`yt_dlp/extractor/filmarchiv.py`, FILMARCHIV
//! ON): the CDN playlist derived from the media ID as fatal HLS formats and
//! subtitles, with the page's `<title-div>` title and prose description.
use super::{Extractor, download_webpage, extract_m3u8_fatal};
use crate::{Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct FilmArchiv;

const NAME: &str = "FilmArchiv";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?filmarchiv\.at/de/filmarchiv-on/video/(?P<id>f_[0-9a-zA-Z]{5,})",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// The description traversal: each `div.border-base-content`'s content, its
/// `div.prose` contents, `clean_html`, `any` (the first, even when empty).
/// `get_elements_by_attribute` builds its list eagerly, so a parse error is
/// `try_call`'s `None` for that step (no matches).
fn description(webpage: &str) -> Option<String> {
    let find = |html: &str, class: &str| {
        crate::html_elements::elements_text_and_html_by_attribute_regex("div", "class", class, html)
            .unwrap_or_default()
    };
    find(webpage, r".*\bborder-base-content\b")
        .into_iter()
        .find_map(|(content, _)| {
            find(&content, r".*\bprose\b")
                .into_iter()
                .next()
                .map(|(prose, _)| crate::html::clean_html(&prose))
        })
}

impl Extractor for FilmArchiv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        let path = format!("{}/{}", &id[..6], &id[6..]);
        let mut info = extract_m3u8_fatal(
            client,
            &Url::parse(&format!(
                "https://cdn.filmarchiv.at/{path}_v1_sv1/playlist.m3u8"
            ))?,
            NAME,
            &id,
            None,
            true,
        )?;
        super::hls::without_ext(&mut info);
        // `{find_element(tag='title-div')}` (a parse error is `None`).
        let title = crate::html_elements::element_text_and_html_by_tag("title-div", &webpage)
            .ok()
            .map(|(content, _)| crate::html::clean_html(&content));
        let description = description(&webpage);
        info.id = id;
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
        info.metadata.description = description;
        info.metadata.thumbnail = Some(format!("https://cdn.filmarchiv.at/{path}_v1/poster.jpg"));
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,description,thumbnail,formats,subtitles",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.filmarchiv.at/de/filmarchiv-on/video/f_0305p7xKrXUPBwoNE9x6mh"
        ));
        assert!(!suitable(
            "https://www.filmarchiv.at/de/filmarchiv-on/video/f_03"
        ));
    }

    #[test]
    fn description_takes_the_first_prose_div() {
        let page = r#"<div class="x border-base-content"><div class="prose a">One <b>two</b></div></div><div class="border-base-content"><div class="prose">Three</div></div>"#;
        assert_eq!(description(page).as_deref(), Some("One two"));
        assert_eq!(description("<div class=prose>x</div>"), None);
    }
}
