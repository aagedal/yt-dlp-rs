//! Upstream `InfoQIE` (`yt_dlp/extractor/infoq.py`): a presentation page's
//! RTMP video (the base64 `jsclassref` path), its CloudFront-signed HTTP
//! video (`P.s`) and the MP3 download form's probed audio. Pages without a
//! `jsclassref` are not supported natively.
use super::{Extractor, dict_format, dict_info, download_webpage, is_valid_url, update_url_query};
use crate::{Error, Result, VideoInfo};
use base64::Engine as _;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct InfoQ;

const NAME: &str = "InfoQ";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?infoq\.com/(?:[^/]+/)+(?P<id>[^/]+)").unwrap()
});
static JSCLASSREF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"jsclassref\s*=\s*'([^']*)'").unwrap());
static PLAYER_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"P\.s\s*=\s*'([^']+)'").unwrap());
static CF_POLICY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"InfoQConstants\.scp\s*=\s*'([^']+)'").unwrap());
static CF_SIGNATURE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"InfoQConstants\.scs\s*=\s*'([^']+)'").unwrap());
static CF_KEY_PAIR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"InfoQConstants\.sck\s*=\s*'([^']+)'").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_regex(..., fatal)` without a default.
fn search(id: &str, webpage: &str, pattern: &Regex, what: &str) -> Result<String> {
    pattern
        .captures(webpage)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract {what}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

/// `_extract_cf_auth(webpage)`: the CloudFront policy, signature and key pair.
fn cf_auth(id: &str, webpage: &str) -> Result<[(&'static str, String); 3]> {
    Ok([
        ("Policy", search(id, webpage, &CF_POLICY, "policy")?),
        (
            "Signature",
            search(id, webpage, &CF_SIGNATURE, "signature")?,
        ),
        (
            "Key-Pair-Id",
            search(id, webpage, &CF_KEY_PAIR, "key-pair-id")?,
        ),
    ])
}

fn with_cf_auth(url: &str, auth: &[(&'static str, String); 3]) -> Result<Url> {
    let pairs: Vec<(&str, &str)> = auth
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    Ok(update_url_query(&Url::parse(url)?, &pairs))
}

/// `_extract_rtmp_video(webpage)`: `rtmpe://videof.infoq.com/cfx/st/` with
/// `mp4:` plus the base64 `jsclassref` path.
fn rtmp_formats(id: &str, webpage: &str) -> Result<Vec<crate::VideoFormat>> {
    let encoded = JSCLASSREF
        .captures(webpage)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| Error::UnsupportedHtml5("InfoQ pages without a jsclassref".into()))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .map_err(|_| {
            Error::UnsupportedHtml5("InfoQ jsclassref values that are not base64".into())
        })?;
    let real_id =
        percent_encoding::percent_decode_str(std::str::from_utf8(&decoded).map_err(|_| {
            Error::UnsupportedHtml5("InfoQ jsclassref values that are not UTF-8".into())
        })?)
        .decode_utf8()
        .map_err(|_| Error::UnsupportedHtml5("InfoQ jsclassref values that are not UTF-8".into()))?
        .into_owned();
    let playpath = format!("mp4:{real_id}");
    let ext = super::direct::determine_ext(&playpath).unwrap_or_else(|| "unknown_video".into());
    Ok(vec![dict_format(
        vec![
            ("format_id", "rtmp_video".into()),
            ("url", "rtmpe://videof.infoq.com/cfx/st/".into()),
            ("ext", ext.into()),
            ("play_path", playpath.into()),
        ],
        NAME,
        id,
    )?])
}

/// `_extract_http_video(webpage)`: the `P.s` URL with the CloudFront query.
fn http_video_formats(id: &str, webpage: &str) -> Result<Vec<crate::VideoFormat>> {
    let video_url = search(id, webpage, &PLAYER_URL, "video URL")?;
    let url = with_cf_auth(&video_url, &cf_auth(id, webpage)?)?;
    Ok(vec![dict_format(
        vec![
            ("format_id", "http_video".into()),
            ("url", url.as_str().into()),
            (
                "http_headers",
                serde_json::json!({"Referer": "https://www.infoq.com/"}),
            ),
        ],
        NAME,
        id,
    )?])
}

/// `_extract_http_audio(webpage, video_id)`: the MP3 form's `filename`
/// (relative to the mp3downloads directory), probed before it is listed.
fn http_audio_formats(id: &str, webpage: &str) -> Result<Vec<crate::VideoFormat>> {
    let form = crate::html_elements::elements_text_and_html_by_attribute("id", "mp3Form", webpage)
        .ok()
        .and_then(|forms| forms.into_iter().next().map(|(_, html)| html));
    let Some(form) = form else {
        return Ok(Vec::new());
    };
    let Some(filename) = super::login::hidden_inputs(&form)
        .into_iter()
        .find(|(name, _)| name == "filename")
        .map(|(_, value)| value)
    else {
        return Ok(Vec::new());
    };
    // `urljoin('http://ress.infoq.com/downloads/mp3downloads/', filename)`.
    let base = Url::parse("http://ress.infoq.com/downloads/mp3downloads/")?;
    let audio = base.join(&filename)?;
    let url = with_cf_auth(audio.as_str(), &cf_auth(id, webpage)?)?;
    Ok(vec![dict_format(
        vec![
            ("format_id", "http_audio".into()),
            ("url", url.as_str().into()),
            ("vcodec", "none".into()),
        ],
        NAME,
        id,
    )?])
}

impl Extractor for InfoQ {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = found["id"].to_owned();
        let webpage = download_webpage(client, url, NAME, &video_id)?;

        let title = crate::html::html_extract_title(&webpage);
        let description = crate::html::html_search_meta(&webpage, "description");
        if description.is_none() {
            super::search_warning(NAME, "description");
        }

        let mut formats = rtmp_formats(&video_id, &webpage)?;
        formats.extend(http_video_formats(&video_id, &webpage)?);
        let audio = http_audio_formats(&video_id, &webpage)?;
        let mut kept = Vec::new();
        for format in audio {
            // The MP3 is probed: a missing file is skipped.
            if is_valid_url(client, format.url.as_ref(), NAME, &video_id, "audio")? {
                kept.push(format);
            }
        }
        formats.extend(kept);

        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.map_or(Value::Null, Value::from)),
                ("description", description.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.infoq.com/presentations/A-Few-of-My-Favorite-Python-Things"
        ));
        assert!(suitable(
            "https://www.infoq.com/fr/presentations/changez-avis-sur-javascript"
        ));
        assert!(!suitable("https://www.infoq.com/"));
    }
}
