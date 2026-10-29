//! Upstream `MojevideoIE` (`yt_dlp/extractor/mojevideo.py`): the page's
//! `vHash` list as signed `cache01.mojevideo.sk` MP4 formats (one per hash,
//! in upstream's quality order) with the OpenGraph title and description
//! and the JSON-LD metadata.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Mojevideo;

const NAME: &str = "Mojevideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?mojevideo\.sk/video/(?P<id>\w+)/(?P<display_id>[\w()]+?)\.html",
    )
    .unwrap()
});
static VIDEO_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bvId\s*=\s*(\d+)").unwrap());
static VIDEO_EXP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bvEx\s*=\s*["'](\d+)"#).unwrap());
static VIDEO_HASHES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bvHash\s*=").unwrap());

/// `(suffix, quality, format_note)` zipped with the hashes.
const QUALITIES: [(&str, i64, &str); 5] = [
    ("", 1, "normálna kvalita"),
    ("_lq", 0, "nízka kvalita"),
    ("_hd", 2, "HD-720p"),
    ("_fhd", 3, "FULL HD-1080p"),
    ("_2k", 4, "2K-1440p"),
];

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Mojevideo {what} are not supported natively"))
}

fn search_error(video_id: &str, name: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {video_id}: Unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_search_json(r'\bvHash\s*=', ..., contains_pattern=r'\[(?s:.+)\]',
/// transform_source=js_to_json)`: the greedy `[...]` after the start and its
/// first JSON value.
fn video_hashes(webpage: &str, video_id: &str) -> Result<Vec<Value>> {
    let found = VIDEO_HASHES
        .find(webpage)
        .ok_or_else(|| search_error(video_id, "video hashes"))?;
    let rest = webpage[found.end()..].trim_start();
    let end = rest
        .starts_with('[')
        .then(|| rest.rfind(']'))
        .flatten()
        .ok_or_else(|| search_error(video_id, "video hashes"))?;
    let text = crate::js_to_json::js_to_json(&rest[..=end]);
    match serde_json::Deserializer::from_str(&text)
        .into_iter::<Value>()
        .next()
    {
        Some(Ok(Value::Array(hashes))) => Ok(hashes),
        Some(Ok(_)) => Err(unsupported("video hashes that are not lists")),
        _ => Err(unsupported("video hashes that do not parse")),
    }
}

/// `str(int(video_id, 16))`.
fn hex_to_decimal(video_id: &str) -> Result<String> {
    u128::from_str_radix(video_id, 16)
        .map(|value| value.to_string())
        .map_err(|_| unsupported("video IDs that are not hexadecimal"))
}

impl Extractor for Mojevideo {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;

        let video_id_dec = match VIDEO_ID.captures(&webpage) {
            Some(found) => found[1].to_owned(),
            None => {
                super::search_warning(NAME, "video id");
                hex_to_decimal(&video_id)?
            }
        };
        let video_exp = VIDEO_EXP
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| search_error(&video_id, "video expiry"))?;
        let hashes = video_hashes(&webpage, &video_id)?;

        let mut formats = Vec::new();
        for (video_hash, (suffix, quality, format_note)) in hashes.iter().zip(QUALITIES) {
            // An f-string-free `update_url_query` value: `str()` of scalars.
            let video_hash = match video_hash {
                Value::String(hash) => hash.clone(),
                Value::Number(number) => number.to_string(),
                _ => return Err(unsupported("video hashes that are not strings")),
            };
            let base = Url::parse(&format!(
                "https://cache01.mojevideo.sk/securevideos69/{video_id_dec}{suffix}.mp4"
            ))?;
            let format_url =
                super::update_url_query(&base, &[("md5", &video_hash), ("expires", &video_exp)]);
            formats.push(dict_format(
                vec![
                    ("format_id", format!("mp4-{quality}").into()),
                    ("quality", quality.into()),
                    ("format_note", format_note.into()),
                    ("url", format_url.as_str().into()),
                ],
                NAME,
                &video_id,
            )?);
        }

        // `_og_search_title(webpage, default=None)` or the `<title>` without
        // ` - Mojevideo` (fatal).
        let title = match crate::html::og_search_property(&webpage, "title") {
            Some(title) if !title.is_empty() => title,
            _ => {
                let title = crate::html::html_extract_title(&webpage)
                    .ok_or_else(|| search_error(&video_id, "title"))?;
                title
                    .strip_suffix(" - Mojevideo")
                    .map_or(title.clone(), str::to_owned)
            }
        };
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            super::search_warning(NAME, "OpenGraph description");
        }

        let mut fields: Vec<(String, Value)> = vec![
            ("id".into(), video_id.clone().into()),
            ("display_id".into(), display_id.into()),
            ("formats".into(), Value::Null),
            ("title".into(), title.into()),
            (
                "description".into(),
                description.map_or(Value::Null, Value::from),
            ),
        ];
        // `**self._search_json_ld(webpage, video_id, default={})`.
        for (key, value) in super::json_ld::search(&webpage) {
            if key == "url" {
                return Err(unsupported("JSON-LD content URLs"));
            }
            match fields.iter_mut().find(|(name, _)| *name == key) {
                Some((_, old)) => *old = value,
                None => fields.push((key, value)),
            }
        }
        let fields = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.mojevideo.sk/video/3d17c/chlapci_dobetonovali_sme_mame_hotovo.html"
        ));
        assert!(suitable(
            "https://www.mojevideo.sk/video/2feb2/band_maid_onset_(instrumental)_live_zepp_tokyo_(full_hd).html"
        ));
        assert!(!suitable("https://www.mojevideo.sk/video/3d17c/"));
    }

    #[test]
    fn hashes_and_hex_ids() {
        let page = "var vHash=['a','b'],vEx='1';x=[1];";
        assert_eq!(
            video_hashes(page, "1").unwrap(),
            vec![Value::from("a"), Value::from("b")]
        );
        assert_eq!(hex_to_decimal("3d17c").unwrap(), "250236");
    }
}
