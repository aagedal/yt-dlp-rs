//! Upstream `ArcPublishingIE` (`yt_dlp/extractor/arcpublishing.py`): Arc XP
//! (`arcpublishing:ORG:UUID`) videos from the organization's
//! `ansvideos/findByUuid` API: SMIL, HLS and progressive streams, English
//! subtitles and the headline, promo image, dates and duration; `powa`
//! player divs on other pages are followed.
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, SubtitleFormat, Subtitles, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct ArcPublishing;

pub(crate) const NAME: &str = "ArcPublishing";
pub(crate) const KEY: &str = "ArcPublishing";

const UUID: &str = r"[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^arcpublishing:(?P<org>[a-z]+):(?P<id>{UUID})")).unwrap()
});
static POWA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(<div[^>]+class="[^"]*\bpowa\b[^"]*"[^>]+data-uuid="{UUID}"[^>]*>)"#
    ))
    .unwrap()
});

/// `_POWA_DEFAULTS`.
const POWA_DEFAULTS: [(&[&str], &str); 2] = [
    (
        &["cmg", "prisa"],
        "%s-config-prod.api.cdn.arcpublishing.com/video",
    ),
    (
        &[
            "adn",
            "advancelocal",
            "answers",
            "bonnier",
            "bostonglobe",
            "demo",
            "gmg",
            "gruponacion",
            "infobae",
            "mco",
            "nzme",
            "pmn",
            "raycom",
            "spectator",
            "tbt",
            "tgam",
            "tronc",
            "wapo",
            "wweek",
        ],
        "video-api-cdn.%s.arcpublishing.com/api",
    ),
];

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls`: `powa` divs with an organization and UUID.
pub(crate) fn embed_urls(_url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in POWA.captures_iter(webpage) {
        let attributes = crate::html_elements::extract_attributes(&captures[1]);
        let text = |key: &str| {
            attributes
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        };
        let (Some(org), Some(uuid)) = (text("data-org"), text("data-uuid")) else {
            continue;
        };
        let Ok(embed) = Url::parse(&format!("arcpublishing:{org}:{uuid}")) else {
            continue;
        };
        if !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn crash(id: &str, cause: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by {cause}); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Arc Publishing {what} are not supported natively"))
}

/// `int_or_none(value, scale)`: Python `int(value) // scale`.
fn int_or_none(value: Option<&Value>, scale: i64) -> Option<i64> {
    let value = match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))?,
        Value::String(text) => text.trim().parse().ok()?,
        Value::Bool(value) => i64::from(*value),
        _ => return None,
    };
    Some(value.div_euclid(scale))
}

/// `try_get(video, lambda x: x[a][b])`.
fn try_get(video: &Value, a: &str, b: &str) -> Value {
    video
        .get(a)
        .and_then(|inner| inner.get(b))
        .cloned()
        .unwrap_or(Value::Null)
}

/// `_extract_smil_formats(url, uuid, fatal=False)`.
fn smil_formats(client: &Client, url: &str, uuid: &str) -> Result<Vec<VideoFormat>> {
    use std::io::Read;
    let smil_url = Url::parse(url).map_err(|_| unsupported("relative SMIL URLs"))?;
    crate::screen::screen(&format!("[{NAME}] {uuid}: Downloading SMIL file"));
    let mut response = match super::direct::request(client, &smil_url, None) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {uuid}: Unable to download SMIL file: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
    };
    let final_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    let smil =
        super::smil::parse(&text).ok_or_else(|| unsupported("SMIL files that are not SMIL"))?;
    let (formats, subtitles) =
        super::html5::smil_formats_and_subtitles(client, &smil, &final_url, uuid, NAME)?;
    if !subtitles.is_empty() {
        crate::screen::warning_once(&format!(
            "[{NAME}] Ignoring subtitle tracks found in the SMIL manifest; if any subtitle tracks are missing, {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    Ok(formats)
}

/// `_extract_m3u8_formats(url, uuid, 'mp4', live=is_live, m3u8_id='hls',
/// fatal=False)` with the URL's `[_x]HEIGHT[_-]VBR` video bitrate.
fn m3u8_formats(client: &Client, url: &str, uuid: &str, is_live: bool) -> Result<Vec<VideoFormat>> {
    let manifest = Url::parse(url).map_err(|_| unsupported("relative HLS URLs"))?;
    let Some(stream) = super::extract_m3u8_master_optional(
        client,
        &manifest,
        NAME,
        uuid,
        Some("hls"),
        false,
        is_live,
    ) else {
        return Ok(Vec::new());
    };
    let mut formats = stream.formats;
    for format in &mut formats {
        format.ext = "mp4".into();
    }
    // `all(f.get('acodec') == 'none' for f in m3u8_formats)`.
    if formats
        .iter()
        .all(|format| format.acodec.as_deref() == Some("none"))
    {
        return Ok(Vec::new());
    }
    for format in &mut formats {
        let Some(height) = format.height.filter(|height| *height != 0) else {
            continue;
        };
        let pattern = Regex::new(&format!(r"[_x]{height}[_-](\d+)")).expect("static pattern");
        let vbr = pattern
            .captures(format.url.as_str())
            .and_then(|found| found[1].parse::<u64>().ok());
        if let Some(vbr) = vbr {
            format.vbr = Some(vbr as f64);
            crate::info_order::mark_format_update(format, "vbr");
            crate::info_order::mark_int_vbr(format);
        }
    }
    Ok(formats)
}

impl Extractor for ArcPublishing {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let mut org = found["org"].to_owned();
        let uuid = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let template = POWA_DEFAULTS
            .iter()
            .find(|(orgs, _)| orgs.contains(&org.as_str()))
            .map_or(
                "%s-prod-cdn.video-api.arcpublishing.com/api",
                |(_, tmpl)| tmpl,
            );
        if org == "wapo" {
            org = "washpost".into();
        }
        let mut api = Url::parse(&format!(
            "https://{}/v1/ansvideos/findByUuid",
            template.replace("%s", &org)
        ))?;
        api.query_pairs_mut().append_pair("uuid", &uuid);
        let response = download_json(client, &api, NAME, &uuid)?;
        let video = match &response {
            // An `IndexError` is not wrapped as an extractor error.
            Value::Array(items) => items
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            Value::Object(_) => return Err(crash(&uuid, "KeyError(0)")),
            _ => return Err(unsupported("API responses that are not lists")),
        };
        let title = match video.get("headlines") {
            None => return Err(crash(&uuid, "KeyError('headlines')")),
            Some(Value::Object(headlines)) => match headlines.get("basic") {
                None => return Err(crash(&uuid, "KeyError('basic')")),
                Some(title) => title.clone(),
            },
            Some(_) => return Err(unsupported("headlines that are not objects")),
        };
        if !title.is_string() {
            return Err(unsupported("titles that are not strings"));
        }
        let is_live = video.get("status").and_then(Value::as_str) == Some("live");

        let mut urls: Vec<String> = Vec::new();
        let mut formats = Vec::new();
        let streams = match video.get("streams") {
            None => &Vec::new(),
            Some(Value::Array(streams)) => streams,
            Some(_) => return Err(unsupported("streams that are not lists")),
        };
        for stream in streams {
            let Value::Object(stream) = stream else {
                return Err(unsupported("streams that are not objects"));
            };
            let s_url = match stream.get("url") {
                Some(value) if !super::nuxt::truthy(value) => continue,
                None => continue,
                Some(Value::String(s_url)) => s_url.clone(),
                Some(_) => return Err(unsupported("stream URLs that are not strings")),
            };
            if urls.contains(&s_url) {
                continue;
            }
            urls.push(s_url.clone());
            let stream_type = match stream.get("stream_type") {
                None | Some(Value::Null) => None,
                Some(Value::String(kind)) => Some(kind.as_str()),
                Some(_) => return Err(unsupported("stream types that are not strings")),
            };
            match stream_type {
                Some("smil") => formats.extend(smil_formats(client, &s_url, &uuid)?),
                Some("ts" | "hls") => formats.extend(m3u8_formats(client, &s_url, &uuid, is_live)?),
                _ => {
                    let vbr = int_or_none(stream.get("bitrate"), 1);
                    let format_id = [
                        stream_type.map(str::to_owned),
                        vbr.filter(|vbr| *vbr != 0).map(|vbr| vbr.to_string()),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("-");
                    let int = |key: &str| {
                        int_or_none(stream.get(key), 1).map_or(Value::Null, Value::from)
                    };
                    let mut format = dict_format(
                        vec![
                            ("format_id", format_id.into()),
                            ("vbr", vbr.map_or(Value::Null, Value::from)),
                            ("width", int("width")),
                            ("height", int("height")),
                            ("filesize", int("filesize")),
                            ("url", s_url.into()),
                            ("quality", (-10).into()),
                        ],
                        NAME,
                        &uuid,
                    )?;
                    crate::info_order::mark_int_vbr(&mut format);
                    formats.push(format);
                }
            }
        }

        let mut subtitles = Subtitles::default();
        if let Some(Value::Array(tracks)) = video.pointer("/subtitles/urls") {
            for track in tracks {
                let Value::Object(track) = track else {
                    return Err(unsupported("subtitles that are not objects"));
                };
                let Some(track_url) = track.get("url").filter(|value| super::nuxt::truthy(value))
                else {
                    continue;
                };
                let Some(track_url) = track_url.as_str() else {
                    return Err(unsupported("subtitle URLs that are not strings"));
                };
                let parsed = Url::parse(track_url)
                    .map_err(|_| unsupported("subtitle URLs that are not absolute"))?;
                let ext = super::direct::determine_ext(track_url)
                    .unwrap_or_else(|| "unknown_video".into())
                    .to_lowercase();
                let track = SubtitleFormat {
                    name: None,
                    url: parsed,
                    ext,
                    protocol: None,
                    fragments: Vec::new(),
                    extra: Map::new(),
                };
                match subtitles
                    .entries
                    .iter_mut()
                    .find(|(key, _)| key.as_deref() == Some("en"))
                {
                    Some((_, tracks)) => tracks.push(track),
                    None => subtitles.entries.push((Some("en".into()), vec![track])),
                }
            }
        }

        let timestamp = video
            .get("created_date")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601);
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", uuid.into()),
                ("title", title),
                ("thumbnail", try_get(video, "promo_image", "url")),
                ("description", try_get(video, "subheadlines", "basic")),
                ("formats", Value::Null),
                (
                    "duration",
                    int_or_none(video.get("duration"), 100).map_or(Value::Null, Value::from),
                ),
                ("timestamp", timestamp.map_or(Value::Null, Value::from)),
                ("subtitles", Value::Null),
                ("is_live", is_live.into()),
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
            "arcpublishing:adn:8c99cb6e-b29c-4bc9-9173-7bf9979225ab"
        ));
        assert!(suitable(
            "arcpublishing:tronc:460f2931-8130-4719-8ea1-ffcb2d7cb685"
        ));
        assert!(!suitable(
            "arcpublishing:ADN:8c99cb6e-b29c-4bc9-9173-7bf9979225ab"
        ));
        assert!(!suitable("https://www.adn.com/video/"));
    }

    #[test]
    fn powa_divs_are_embeds() {
        let page = Url::parse("https://example.com/").unwrap();
        let html = r#"<div class="x powa y" data-org="gray" data-uuid="508116f7-e999-48db-b7c2-60a04842679b"></div>
            <div class="powa" data-uuid="508116f7-e999-48db-b7c2-60a04842679b"></div>"#;
        assert_eq!(
            embed_urls(&page, html)
                .iter()
                .map(Url::as_str)
                .collect::<Vec<_>>(),
            ["arcpublishing:gray:508116f7-e999-48db-b7c2-60a04842679b"]
        );
    }
}
