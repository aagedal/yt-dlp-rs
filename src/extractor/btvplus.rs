//! Upstream `BTVPlusIE` (`yt_dlp/extractor/btvplus.py`): bTV Plus shows,
//! series and news from the page's `videoUrl` player JSON: its `config`
//! player's `videojs(...)` sources as HLS formats and subtitles, with Open
//! Graph (else `product-title`) metadata.
use super::{Extractor, dict_info, download_json, download_webpage};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct BtvPlus;

const NAME: &str = "BTVPlus";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?btvplus\.bg/produkt/(?:predavaniya|seriali|novini)/(?P<id>\d+)",
    )
    .unwrap()
});
static PLAYER_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+videoUrl\s*=\s*['"]([^'"]+)['"]"#).unwrap());
static VIDEOJS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"videojs\(["'][^"']+["'],"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn reported(video_id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {video_id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("bTV Plus {what} are not supported natively"))
}

/// `get_element_by_class(name, webpage)`.
fn element_by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(text, _)| text)
}

impl Extractor for BtvPlus {
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
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let player_url = PLAYER_URL
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| reported(&video_id, "Unable to extract player URL"))?;
        // `urljoin('https://btvplus.bg', player_url)`.
        let player_url = Url::parse("https://btvplus.bg")?
            .join(&player_url)
            .map_err(|_| unsupported("player URLs that do not join"))?;
        let player = download_json(client, &player_url, NAME, &video_id)?;
        let player_config = match player.get("config") {
            Some(Value::String(config)) => config.clone(),
            Some(_) => return Err(unsupported("player configs that are not strings")),
            None => {
                return Err(Error::Reported(format!(
                    "{video_id}: An extractor error has occurred. (caused by KeyError('config')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let videojs_data = super::search_json(&VIDEOJS, &player_config, true)
            .ok_or_else(|| reported(&video_id, "Unable to extract videojs data"))?
            .ok_or_else(|| unsupported("videojs data that do not parse"))?;
        let sources: Vec<&Value> = match videojs_data.get("sources") {
            Some(Value::Array(items)) => items.iter().collect(),
            Some(Value::Object(items)) => items.values().collect(),
            _ => Vec::new(),
        };
        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        for source in sources {
            // `lambda _, v: url_or_none(v['src'])`.
            let Some(src) = source
                .get("src")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let ext = super::vidyard::mimetype2ext(source.get("type"));
            if ext.as_str() == Some("m3u8") {
                let Ok(manifest) = Url::parse(&src) else {
                    return Err(unsupported("source URLs that are not URLs"));
                };
                if let Some(stream) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("hls"),
                    true,
                    false,
                ) {
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            } else {
                crate::screen::warning(&format!(
                    "[{NAME}] Unknown format type {}; {}",
                    ext.as_str().unwrap_or("None"),
                    crate::BUG_REPORTS_MESSAGE
                ));
            }
        }
        let title = crate::html::og_search_property(&webpage, "title")
            .map(|title| crate::batch::py_strip(&title).to_owned())
            .filter(|title| !title.is_empty())
            .or_else(|| {
                element_by_class("product-title", &webpage)
                    .map(|title| crate::html::clean_html(&title))
            });
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("title", optional(title)),
                (
                    "thumbnail",
                    optional(crate::html::og_search_property(&webpage, "image")),
                ),
                (
                    "description",
                    optional(crate::html::og_search_property(&webpage, "description")),
                ),
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
            "https://btvplus.bg/produkt/predavaniya/67271/btv-reporterite/btv-reporterite-12-07-2025-g"
        ));
        assert!(suitable(
            "https://btvplus.bg/produkt/novini/67270/btv-novinite-centralna-emisija-12-07-2025"
        ));
        assert!(!suitable("https://btvplus.bg/produkt/filmi/1"));
    }
}
