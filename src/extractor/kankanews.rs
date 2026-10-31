//! Upstream `KankaNewsIE` (`yt_dlp/extractor/kankanews.py`): an article's
//! `omsid` and the signed `getvideo` API's video URL as an untyped result.
use super::{Extractor, broken_warning, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct KankaNews;

const NAME: &str = "KankaNews";
const SECRET: &str = "&28c8edde3d61a0411511d3b1866f0636";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?kankanews\.com/a/\d+-\d+-\d+/(?P<id>\d+)\.shtml",
    )
    .unwrap()
});
static VIDEO_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"omsid\s*=\s*"(\d+)""#).unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"g\.title\s*=\s*"([^"]+)""#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// A clock-seeded xorshift64 for `random.choices`.
fn random_below(bound: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static STATE: Mutex<u64> = Mutex::new(0);
    let mut state = STATE.lock().unwrap_or_else(|poison| poison.into_inner());
    if *state == 0 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos()),
        );
        *state = hasher.finish() | 1;
    }
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state % bound.max(1)
}

/// The signed `getvideo` query (`urlencode(params)` in upstream's order).
fn signed_query(video_id: &str, nonce: &str, timestamp: u64) -> String {
    let params =
        format!("nonce={nonce}&omsid={video_id}&platform=pc&timestamp={timestamp}&version=1.0");
    let inner = crate::md5::hex(format!("{params}{SECRET}").as_bytes());
    let sign = crate::md5::hex(inner.as_bytes());
    format!("{params}&sign={sign}")
}

impl Extractor for KankaNews {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        broken_warning();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let video_id = VIDEO_ID
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract video id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let nonce: String = (0..8)
            .map(|_| ALPHABET[random_below(ALPHABET.len() as u64) as usize] as char)
            .collect();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_secs());
        let mut api = Url::parse("https://api-app.kankanews.com/kankan/pc/getvideo")?;
        api.set_query(Some(&signed_query(&video_id, &nonce, timestamp)));
        let response = download_json(client, &api, NAME, &video_id)?;
        let crash = |key: &str| {
            Error::Reported(format!(
                "{display_id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        // `['result']['video']`, then `meta['videourl']`.
        let mut meta = &response;
        for key in ["result", "video"] {
            meta = match meta {
                Value::Object(map) => map.get(key).ok_or_else(|| crash(key))?,
                _ => {
                    return Err(Error::UnsupportedHtml5(
                        "KankaNews API values that are not objects are not supported natively"
                            .into(),
                    ));
                }
            };
        }
        let Value::Object(meta) = meta else {
            return Err(Error::UnsupportedHtml5(
                "KankaNews API values that are not objects are not supported natively".into(),
            ));
        };
        let media = match meta.get("videourl") {
            None => return Err(crash("videourl")),
            Some(Value::String(media)) => Url::parse(media)?,
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "KankaNews video URLs that are not strings are not supported natively".into(),
                ));
            }
        };
        let title = TITLE
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract title; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.id = video_id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(
            &mut info,
            "thumbnail",
            meta.get("titlepic").cloned().unwrap_or(Value::Null),
        );
        Ok(Some(crate::info_order::mark_info(
            info,
            "untyped-dict;id,url,title,thumbnail",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.kankanews.com/a/2022-11-08/00310276054.shtml?appid=1088227"
        ));
        assert!(!suitable("https://www.kankanews.com/a/2022-11/1.shtml"));
    }

    #[test]
    fn queries_are_signed_like_upstream() {
        // `hashlib.md5(hashlib.md5((urlencode(params) + SECRET).encode())
        // .hexdigest().encode()).hexdigest()` from the pinned upstream.
        assert_eq!(
            signed_query("4485057", "abcd1234", 1_700_000_000),
            "nonce=abcd1234&omsid=4485057&platform=pc&timestamp=1700000000&version=1.0&sign=34c156784efb548b99689ccf6511e7c7"
        );
    }
}
