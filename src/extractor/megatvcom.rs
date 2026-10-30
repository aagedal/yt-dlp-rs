//! Upstream `MegaTVComIE` (`yt_dlp/extractor/megatvcom.py`): a megatv.com
//! show or article page's `player_div_id` attributes (the source playlist,
//! poster, subtitle and title) with its description and published time. The
//! article id lookup of article pages is ported as upstream has it. The embed
//! extractor (`MegaTVComEmbedIE`) is not ported: upstream cannot resolve its
//! canonical URLs any more.
use super::{Extractor, dict_format, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, SubtitleFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct MegaTVCom;

const NAME: &str = "megatvcom";
const PLAYER_DIV_ID: &str = "player_div_id";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?megatv\.com/(?:\d{4}/\d{2}/\d{2}|[^/]+/(?P<id>\d+))/(?P<slug>[^/]+)",
    )
    .unwrap()
});
static ARTICLE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<article[^>]*\sid=["']Article_(\d+)["']"#).unwrap());
static SCRIPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<script[^>]*>[^<]+</script>").unwrap());
static ATTRIBUTE_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^data-(?:kwik_)?").unwrap());

/// `_match_valid_url` for the id and slug.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_extract_player_attrs`: the player div's `data-` attributes, the `data-kwik_`
/// prefix removed (its `id` left out). A page without the div is a Python
/// `TypeError` upstream.
fn player_attrs(webpage: &str) -> Result<Map<String, Value>> {
    let element = crate::html_elements::element_html_by_id(PLAYER_DIV_ID, webpage)
        .map_err(Error::Reported)?
        .ok_or_else(|| {
            Error::Reported("expected string or bytes-like object, got 'NoneType'".into())
        })?;
    Ok(crate::html_elements::extract_attributes(&element)
        .into_iter()
        .filter(|(key, _)| key != "id")
        .map(|(key, value)| (ATTRIBUTE_PREFIX.replace(&key, "").into_owned(), value))
        .collect())
}

/// A non-empty string attribute (a Python truthy value).
fn attr<'a>(attrs: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    attrs
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

impl Extractor for MegaTVCom {
    fn key(&self) -> &'static str {
        "MegaTVCom"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["slug"].to_owned();
        let article = found.name("id").is_none();
        let mut video_id = found.name("id").map(|id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(
            client,
            url,
            NAME,
            video_id.as_deref().unwrap_or(&display_id),
        )?;
        if article {
            let id = ARTICLE_ID
                .captures(&webpage)
                .map(|found| found[1].to_owned())
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] Unable to extract article id; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?;
            video_id = Some(id);
        }
        let video_id = video_id.unwrap_or_default();
        let attrs = player_attrs(&webpage)?;
        let title = attr(&attrs, "label")
            .map(str::to_owned)
            .or_else(|| crate::html::og_search_property(&webpage, "title"));
        let description_class = if article {
            "article-wrapper"
        } else {
            "story_content"
        };
        let description =
            crate::html_elements::elements_text_and_html_by_class(description_class, &webpage)
                .map_err(Error::Reported)?
                .into_iter()
                .next()
                .map(|(text, _)| text)
                .ok_or_else(|| {
                    Error::Reported("expected string or bytes-like object, got 'NoneType'".into())
                })?;
        let description = crate::html::clean_html(&SCRIPT.replace_all(&description, ""));
        let description = if description.is_empty() {
            crate::html::og_search_property(&webpage, "description")
        } else {
            Some(description)
        };
        let thumbnail = attr(&attrs, "image")
            .map(str::to_owned)
            .or_else(|| crate::html::og_search_property(&webpage, "image"));
        let timestamp = crate::html::html_search_meta(&webpage, "article:published_time")
            .and_then(|date| crate::dates::unified_timestamp(&date, false));
        let source = attr(&attrs, "source").ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: No source found; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let (formats, mut subtitles) = if super::direct::determine_ext(source).as_deref()
            == Some("m3u8")
        {
            let manifest = Url::parse(source)?;
            let stream = extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, true)?;
            (stream.formats, stream.subtitles)
        } else {
            let format = dict_format(vec![("url", source.into())], NAME, &video_id)?;
            (vec![format], crate::Subtitles::default())
        };
        if let Some(subs) = attr(&attrs, "subs") {
            let track = SubtitleFormat {
                name: None,
                url: Url::parse(subs)?,
                ext: "vtt".into(),
                protocol: None,
                fragments: Vec::new(),
                extra: Map::new(),
            };
            let lang = Some("und".to_owned());
            match subtitles.entries.iter_mut().find(|(key, _)| *key == lang) {
                Some((_, tracks)) => tracks.push(track),
                None => subtitles.entries.push((lang, vec![track])),
            }
        }

        let mut info = dict_info(
            url,
            (NAME, "MegaTVCom"),
            vec![
                ("id", video_id.clone().into()),
                ("display_id", display_id.into()),
                ("title", title.map_or(Value::Null, Value::from)),
                ("description", description.map_or(Value::Null, Value::from)),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
                ("timestamp", timestamp.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.megatv.com/tvshows/527800/epeisodio-65-12/"
        ));
        assert!(suitable(
            "https://www.megatv.com/2021/10/23/egkainia-gia-ti-nea-skini-omega-tou-dimotikou-theatrou-peiraia/"
        ));
        assert!(!suitable("https://www.megatv.com/embed/?p=2020520979"));
    }
}
