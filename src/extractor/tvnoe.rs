//! Upstream `TVNoeIE` (`yt_dlp/extractor/tvnoe.py`): a programme page's
//! `var INIT_PLAYER` tracks as non-fatal HLS/DASH formats with its headings,
//! premiere date and player poster.
use super::{
    Extractor, dict_info, download_webpage, extract_m3u8_master_optional, extract_mpd_optional_note,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct TvNoe;

const NAME: &str = "tvnoe";
const KEY: &str = "TVNoe";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?tvnoe\.cz/porad/(?P<id>[\w-]+)").unwrap());
static INIT_PLAYER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+INIT_PLAYER\s*=").unwrap());
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<p\s+class="">(.+?)</p>"#).unwrap());
static PREMIERE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Premiéra:\s*(\d{1,2}\.\d{1,2}\.\d{4})").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `({find_element(tag=TAG)}, {clean_html})` (parse errors give `None`).
fn heading(webpage: &str, tag: &str) -> Option<String> {
    crate::html_elements::element_text_and_html_by_tag(tag, webpage)
        .ok()
        .map(|(text, _)| crate::html::clean_html(&text))
}

impl Extractor for TvNoe {
    fn key(&self) -> &'static str {
        KEY
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
        let player = match super::search_json(&INIT_PLAYER, &webpage, true) {
            Some(Some(player)) => player,
            Some(None) => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {id}: Failed to parse JSON; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract init player; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };

        let mut formats = Vec::new();
        // `('tracks', ..., lambda _, v: url_or_none(v['src']))`: the filter
        // runs over each track group's children.
        let children = |value: &Value| match value {
            Value::Array(items) => items.clone(),
            Value::Object(map) => map.values().cloned().collect(),
            _ => Vec::new(),
        };
        let tracks: Vec<Value> = player
            .get("tracks")
            .map(children)
            .unwrap_or_default()
            .iter()
            .flat_map(children)
            .collect();
        for source in tracks {
            let Some(src) = source
                .get("src")
                .and_then(Value::as_str)
                .filter(|src| super::json_ld::url_or_none(src).is_some())
            else {
                continue;
            };
            let ext = source
                .get("type")
                .and_then(Value::as_str)
                .map(|mime| super::direct::mimetype2ext_or(mime, None))
                .filter(|ext| !ext.is_empty());
            match ext.as_deref() {
                Some("m3u8") => {
                    let manifest = Url::parse(src)?;
                    if let Some(stream) = extract_m3u8_master_optional(
                        client,
                        &manifest,
                        NAME,
                        &id,
                        Some("hls"),
                        false,
                        false,
                    ) {
                        formats.extend(stream.formats);
                    }
                }
                Some("mpd") => {
                    let manifest = Url::parse(src)?;
                    if let Some(mut stream) = extract_mpd_optional_note(
                        client,
                        &manifest,
                        NAME,
                        &id,
                        Some("dash"),
                        "Downloading MPD manifest",
                    ) {
                        super::ignore_subtitles(&mut stream, NAME, "DASH");
                        formats.extend(stream.formats);
                    }
                }
                ext => crate::screen::warning(&format!(
                    "[{NAME}] Unsupported stream type: {}",
                    ext.unwrap_or("None")
                )),
            }
        }

        let description = DESCRIPTION.captures(&webpage).map_or(Value::Null, |found| {
            crate::html::clean_html(&found[1]).into()
        });
        let mut fields = vec![
            ("id", Value::from(id.clone())),
            ("description", description),
            ("formats", Value::Null),
        ];
        if let Some(title) = heading(&webpage, "h2") {
            fields.push(("title", title.into()));
        }
        let cleaned = crate::html::clean_html(&webpage);
        if let Some(date) = PREMIERE
            .captures_iter(&cleaned)
            .find_map(|found| crate::dates::unified_strdate(&found[1], true))
        {
            fields.push(("release_date", date.into()));
        }
        if let Some(series) = heading(&webpage, "h1") {
            fields.push(("series", series.into()));
        }
        if let Some(thumbnail) = crate::html_elements::element_html_by_id("player-live", &webpage)
            .ok()
            .flatten()
            .and_then(|element| {
                crate::html_elements::extract_attributes(&element)
                    .get("poster")
                    .and_then(Value::as_str)
                    .filter(|poster| !poster.is_empty())
                    .map(str::to_owned)
            })
            .and_then(|poster| Url::parse("https://www.tvnoe.cz/").ok()?.join(&poster).ok())
        {
            fields.push(("thumbnail", thumbnail.as_str().into()));
        }
        Ok(Some(dict_info(url, (NAME, KEY), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.tvnoe.cz/porad/43205-zamysleni-tomase-halika-7-nedele-velikonocni"
        ));
        assert!(!suitable("https://www.tvnoe.cz/porady"));
    }
}
