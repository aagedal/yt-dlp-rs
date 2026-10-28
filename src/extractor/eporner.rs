//! Upstream `EpornerIE` (`yt_dlp/extractor/eporner.py`): Eporner videos from
//! the `xhr/video` JSON (signed with the page's hash): HLS playlists and MP4
//! sources (with AV1 variants when the page offers them), merged with the
//! page's JSON-LD, duration and view count.
use super::{Extractor, dict_format, dict_info, download_json_note, download_webpage_handle};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Eporner;

const NAME: &str = "Eporner";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?eporner\.com/(?:(?:hd-porn|embed)/|video-)(?P<id>\w+)(?:/(?P<display_id>[\w-]+))?",
    )
    .unwrap()
});
static HASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"hash\s*[:=]\s*["']([\da-f]{32})"#).unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<title>(.+?) - EPORNER").unwrap());
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)[pP]").unwrap());
static FPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)fps").unwrap());
static VIEWS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"id=["']cinemaviews1["'][^>]*>\s*([0-9,]+)"#).unwrap());
static CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<[a-z][^>]*?\sclass\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// A fatal `_search_regex` error: `get_temp_id`'s URL ID.
fn regex_error(temp_id: &str, name: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {temp_id}: Unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `encode_base_n(int(s[lb:lb + 8], 16), 36)` for each 8-digit block.
fn calc_hash(hash: &str) -> String {
    const TABLE: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = String::new();
    for block in hash.as_bytes().chunks(8) {
        let mut num =
            u64::from_str_radix(std::str::from_utf8(block).unwrap_or("0"), 16).unwrap_or_default();
        if num == 0 {
            out.push('0');
            continue;
        }
        let mut digits = Vec::new();
        while num > 0 {
            digits.push(TABLE[(num % 36) as usize]);
            num /= 36;
        }
        digits.reverse();
        out.push_str(&String::from_utf8(digits).unwrap_or_default());
    }
    out
}

/// `bool(get_elements_by_class('download-av1', webpage))`.
fn has_av1(webpage: &str) -> bool {
    CLASS.captures_iter(webpage).any(|found| {
        found
            .get(1)
            .or_else(|| found.get(2))
            .is_some_and(|classes| {
                classes
                    .as_str()
                    .split_whitespace()
                    .any(|c| c == "download-av1")
            })
    })
}

fn int_match(regex: &Regex, text: &str) -> Option<i64> {
    regex
        .captures(text)
        .and_then(|found| found[1].parse::<i64>().ok())
}

impl Extractor for Eporner {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        let display_id = found
            .name("display_id")
            .map_or_else(|| temp_id.clone(), |id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let (webpage, final_url) = download_webpage_handle(client, url, NAME, &display_id)?;
        // `self._match_id(urlh.url)`.
        let video_id = VALID_URL
            .captures(final_url.as_str())
            .map(|found| found["id"].to_owned())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Eporner redirects to unsupported URLs are not supported natively".into(),
                )
            })?;
        let vid_hash = HASH
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| regex_error(&temp_id, "hash"))?;
        let title = match crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
        {
            Some(title) => title,
            None => TITLE
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]))
                .ok_or_else(|| regex_error(&temp_id, "title"))?,
        };
        let api = super::update_url_query(
            &Url::parse(&format!("http://www.eporner.com/xhr/video/{video_id}"))?,
            &[
                ("hash", calc_hash(&vid_hash).as_str()),
                ("device", "generic"),
                ("domain", "www.eporner.com"),
                ("fallback", "false"),
            ],
        );
        let video = download_json_note(client, &api, NAME, &display_id, "Downloading video JSON")?;
        if video.get("available") == Some(&Value::Bool(false)) {
            let message = match video.get("message") {
                Some(Value::String(message)) => message.clone(),
                Some(Value::Null) => "None".to_owned(),
                Some(other) => other.to_string(),
                None => {
                    return Err(Error::Reported(format!(
                        "{temp_id}: An extractor error has occurred. (caused by KeyError('message')); {}",
                        crate::BUG_REPORTS_MESSAGE
                    )));
                }
            };
            return Err(Error::Reported(format!(
                "[{NAME}] {temp_id}: {NAME} said: {message}"
            )));
        }
        let sources = video.get("sources").ok_or_else(|| {
            Error::Reported(format!(
                "{temp_id}: An extractor error has occurred. (caused by KeyError('sources')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let Value::Object(sources) = sources else {
            return Err(Error::UnsupportedHtml5(
                "Eporner sources that are not objects are not supported natively".into(),
            ));
        };
        let av1 = has_av1(&webpage);
        let mut formats = Vec::new();
        for (kind, formats_dict) in sources {
            let Value::Object(formats_dict) = formats_dict else {
                continue;
            };
            for (format_id, format_dict) in formats_dict {
                let Some(src) = format_dict
                    .get("src")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .filter(|src| src.starts_with("http"))
                else {
                    continue;
                };
                if kind == "hls" {
                    if let Ok(manifest) = Url::parse(&src)
                        && let Some(stream) = super::extract_m3u8_master_optional(
                            client,
                            &manifest,
                            NAME,
                            &display_id,
                            Some(kind),
                            false,
                            false,
                        )
                    {
                        formats.extend(stream.formats);
                    }
                    continue;
                }
                let height = int_match(&HEIGHT, format_id).map_or(Value::Null, Value::from);
                let fps = int_match(&FPS, format_id).map_or(Value::Null, Value::from);
                formats.push(dict_format(
                    vec![
                        ("url", src.as_str().into()),
                        ("format_id", format_id.as_str().into()),
                        ("height", height.clone()),
                        ("fps", fps.clone()),
                    ],
                    NAME,
                    &display_id,
                )?);
                if av1 {
                    formats.push(dict_format(
                        vec![
                            ("url", src.replace(".mp4", "-av1.mp4").into()),
                            ("format_id", format!("av1-{format_id}").into()),
                            ("height", height),
                            ("fps", fps),
                            ("vcodec", "av1".into()),
                        ],
                        NAME,
                        &display_id,
                    )?);
                }
            }
        }
        let json_ld = super::json_ld::search(&webpage);
        let duration = crate::html::html_search_meta(&webpage, "duration")
            .and_then(|duration| crate::parse_duration(&duration))
            .map(|duration| match duration {
                crate::info::PyNumber::Int(value) => Value::from(value),
                crate::info::PyNumber::Float(value) => Value::from(value),
            });
        let view_count = VIEWS
            .captures(&webpage)
            .and_then(|found| found[1].replace(',', "").parse::<i64>().ok());
        // `merge_dicts(json_ld, {...})`: the first non-`None` value of each
        // key wins, except that strings replace empty strings.
        let mut fields: Vec<(String, Value)> = json_ld.into_iter().collect();
        for (key, value) in [
            ("id", Some(Value::from(video_id))),
            ("display_id", Some(Value::from(display_id))),
            ("title", Some(Value::from(title))),
            ("duration", duration),
            ("view_count", view_count.map(Value::from)),
            ("formats", Some(Value::Null)),
            ("age_limit", Some(Value::from(18))),
        ] {
            let Some(value) = value else {
                continue;
            };
            match fields.iter_mut().find(|(name, _)| name == key) {
                Some((_, old)) => {
                    if value.is_string() && old.as_str() == Some("") {
                        *old = value;
                    }
                }
                None => fields.push((key.to_owned(), value)),
            }
        }
        let fields: Vec<(&str, Value)> = fields
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
            "http://www.eporner.com/hd-porn/95008/Infamous-Tiffany-Teen-Strip-Tease-Video/"
        ));
        assert!(suitable("http://www.eporner.com/hd-porn/3YRUtzMcWn0"));
        assert!(suitable("http://www.eporner.com/embed/3YRUtzMcWn0"));
        assert!(suitable(
            "https://www.eporner.com/video-FJsA19J3Y3H/one-of-the-greats/"
        ));
        assert!(!suitable("https://www.eporner.com/cat/hd-1080p/"));
    }

    #[test]
    fn hashes_follow_upstreams_vjs() {
        // Pinned upstream `calc_hash`.
        assert_eq!(
            calc_hash("0123456789abcdef00000000ffffffff"),
            "bd4yv1275p7z01z141z3"
        );
    }
}
