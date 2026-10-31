//! Upstream `UMGDeIE` (`yt_dlp/extractor/umg.py`): the page's JSON-LD, the
//! `umg-artist-screenname` meta artist, the `_3Y0Lj` description element and
//! the `hls.universal-music.de/get` HLS formats.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct UmgDe;

const NAME: &str = "umg:de";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?universal-music\.de/[^/?#]+/videos/(?P<slug>[^/?#]+-(?P<id>\d+))",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for UmgDe {
    fn key(&self) -> &'static str {
        "UMGDe"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["slug"].to_owned();
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        // `_search_json_ld(webpage, display_id)` (fatal).
        let json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract JSON-LD; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        // `traverse_obj(_html_search_meta(...), (filter, all))`: a missing
        // meta warns.
        let artist = crate::html::html_search_meta(&webpage, "umg-artist-screenname");
        if artist.is_none() {
            super::search_warning(NAME, "umg-artist-screenname");
        }
        let artists: Vec<Value> = artist
            .filter(|artist| !artist.is_empty())
            .map(Value::from)
            .into_iter()
            .collect();
        // `traverse_obj(webpage, ({find_element(cls='_3Y0Lj')}, {clean_html}))`.
        let description = crate::html_elements::elements_text_and_html_by_class("_3Y0Lj", &webpage)
            .ok()
            .and_then(|found| found.into_iter().next())
            .map(|(text, _)| crate::html::clean_html(&text));
        let manifest = super::update_url_query(
            &Url::parse("https://hls.universal-music.de/get")?,
            &[("id", &id)],
        );
        let mut info =
            super::extract_m3u8_fatal(client, &manifest, NAME, &display_id, None, false)?;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = "UMGDe".into();
        // `{**json_ld, 'id', 'artists', 'description', 'display_id',
        // 'formats'}`.
        let own = ["id", "artists", "description", "display_id", "formats"];
        let mut keys: Vec<String> = Vec::new();
        for (key, value) in json_ld {
            if !own.contains(&key.as_str()) {
                crate::metadata_parser::set_field(&mut info, &key, value);
            }
            keys.push(key);
        }
        for key in own {
            if !keys.iter().any(|existing| existing == key) {
                keys.push(key.to_owned());
            }
        }
        info.id = id;
        crate::metadata_parser::set_field(&mut info, "artists", Value::Array(artists));
        crate::metadata_parser::set_field(
            &mut info,
            "description",
            description.map_or(Value::Null, Value::from),
        );
        crate::metadata_parser::set_field(&mut info, "display_id", Value::from(display_id));
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("dict;{}", keys.join(",")),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.universal-music.de/sido/videos/jedes-wort-ist-gold-wert-457803"
        ));
        assert!(suitable(
            "https://www.universal-music.de/alexander-eder/videos/der-doktor-hat-gesagt-609533"
        ));
        assert!(!suitable("https://www.universal-music.de/sido/videos/"));
    }
}
