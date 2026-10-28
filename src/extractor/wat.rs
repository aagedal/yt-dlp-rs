//! Upstream `WatIE` (`yt_dlp/extractor/wat.py`): TF1 group media (`wat:ID`,
//! `wat.tv/video/...-ID`) from the `mediainfocombo` API: its delivery's
//! DASH or HLS manifest (else, without formats, the `get/webhtml`
//! manifests), upstream's geo, DRM and delivery errors, and the title,
//! preview, date and duration.
use super::{
    Extractor, dict_info, download_json, extract_m3u8_master_optional, extract_mpd_optional_note,
};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Wat;

const NAME: &str = "wat.tv";
const KEY: &str = "Wat";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:wat:|https?://(?:www\.)?wat\.tv/video/.*-)(?P<id>[0-9a-z]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("WAT {what} are not supported natively"))
}

/// `video_id if video_id.isdigit() and len(video_id) > 6 else
/// str(int(video_id, 36))`.
fn media_id(id: &str) -> Option<String> {
    if id.len() > 6 && id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some(id.to_owned());
    }
    u128::from_str_radix(id, 36).ok().map(|id| id.to_string())
}

/// `extract_formats(manifest_urls)`: DASH (`dash`/`mpd`) and HLS entries.
fn extract_formats(
    client: &Client,
    manifests: &[(Option<String>, Value)],
    video_id: &str,
    formats: &mut Vec<crate::VideoFormat>,
    subtitles: &mut Subtitles,
) -> Result<()> {
    for (kind, url) in manifests {
        if !super::nuxt::truthy(url) {
            continue;
        }
        let Some(url) = url.as_str() else {
            return Err(unsupported("manifest URLs that are not strings"));
        };
        let stream = match kind.as_deref() {
            Some("dash" | "mpd") => {
                let manifest =
                    Url::parse(&url.replace("://das-q1.tf1.fr/", "://das-q1-ssl.tf1.fr/"))
                        .map_err(|_| unsupported("manifest URLs that are not URLs"))?;
                extract_mpd_optional_note(
                    client,
                    &manifest,
                    NAME,
                    video_id,
                    Some("dash"),
                    "Downloading MPD manifest",
                )
            }
            Some("hls") => {
                let manifest =
                    Url::parse(url).map_err(|_| unsupported("manifest URLs that are not URLs"))?;
                extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    video_id,
                    Some("hls"),
                    true,
                    false,
                )
            }
            _ => continue,
        };
        if let Some(stream) = stream {
            formats.extend(stream.formats);
            for (lang, tracks) in stream.subtitles.entries {
                match subtitles.entries.iter_mut().find(|(key, _)| *key == lang) {
                    Some((_, existing)) => existing.extend(tracks),
                    None => subtitles.entries.push((lang, tracks)),
                }
            }
        }
    }
    Ok(())
}

/// `report_drm(video_id)`.
fn report_drm(video_id: &str) -> Result<()> {
    crate::no_formats::raise_expected(NAME, Some(video_id), None, "This video is DRM protected")
}

impl Extractor for Wat {
    fn key(&self) -> &'static str {
        KEY
    }

    /// `_GEO_BYPASS = False`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&[])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = media_id(&found["id"])
            .ok_or_else(|| unsupported("media IDs that do not fit an integer"))?;
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let mut api = Url::parse(&format!(
            "https://mediainfo.tf1.fr/mediainfocombo/{video_id}"
        ))?;
        api.query_pairs_mut().append_pair("pver", "5010000");
        let video_data = download_json(client, &api, NAME, &video_id)?;
        let video_info = video_data
            .get("media")
            .ok_or_else(|| key_error(&video_id, "media"))?;
        if let Some(error_desc) = video_info
            .get("error_desc")
            .filter(|value| super::nuxt::truthy(value))
        {
            let Some(mut message) = error_desc.as_str().map(str::to_owned) else {
                return Err(unsupported("error descriptions that are not strings"));
            };
            match video_info.get("error_code").and_then(Value::as_str) {
                Some("GEOBLOCKED") => {
                    let countries: Vec<&str> = match video_info.get("geoList") {
                        Some(Value::Array(codes)) => {
                            codes.iter().filter_map(Value::as_str).collect()
                        }
                        _ => Vec::new(),
                    };
                    // No `video_id`: `get_temp_id`'s URL ID.
                    return Err(crate::geo::restricted(
                        &format!("[{NAME}] {}: {message}", &found["id"]),
                        &countries,
                    ));
                }
                Some("DELIVERY_ERROR") => {
                    let code = video_data.pointer("/delivery/code").and_then(Value::as_i64);
                    if matches!(code, Some(403 | 500)) {
                        report_drm(&video_id)?;
                    }
                    if let Some(detail) = video_data
                        .pointer("/delivery/error")
                        .and_then(Value::as_str)
                        .filter(|detail| !detail.is_empty())
                    {
                        message = if message.is_empty() {
                            detail.to_owned()
                        } else {
                            format!("{message}: {detail}")
                        };
                    }
                }
                _ => {}
            }
            return Err(Error::Reported(format!("[{NAME}] {video_id}: {message}")));
        }
        let title = match video_info.get("title") {
            Some(Value::String(title)) => title.clone(),
            Some(_) => return Err(unsupported("titles that are not strings")),
            None => return Err(key_error(&video_id, "title")),
        };
        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        let delivery = video_data
            .get("delivery")
            .filter(|delivery| super::nuxt::truthy(delivery))
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        let kind = match delivery.get("format") {
            None | Some(Value::Null) => None,
            Some(Value::String(kind)) => Some(kind.clone()),
            Some(_) => return Err(unsupported("delivery formats that are not strings")),
        };
        let delivery_url = delivery.get("url").cloned().unwrap_or(Value::Null);
        extract_formats(
            client,
            &[(kind, delivery_url)],
            &video_id,
            &mut formats,
            &mut subtitles,
        )?;
        if formats.is_empty() {
            if delivery.get("drm").is_some_and(super::nuxt::truthy) {
                report_drm(&video_id)?;
            }
            let webhtml = Url::parse(&format!("http://www.wat.tv/get/webhtml/{video_id}"))?;
            // `_download_json(..., fatal=False)`.
            let manifests = match download_json(client, &webhtml, NAME, &video_id) {
                Ok(manifests) => manifests,
                Err(Error::ExtractorRequest { source, .. }) => {
                    crate::screen::warning(&format!(
                        "[{NAME}] {video_id}: Unable to download JSON metadata: {}",
                        crate::upstream_error_text(&source)
                    ));
                    Value::Null
                }
                Err(Error::Reported(message)) if message.contains(": Failed to parse JSON ") => {
                    crate::screen::warning(&message);
                    Value::Null
                }
                Err(error) => return Err(error),
            };
            if let Value::Object(manifests) = manifests {
                let manifests: Vec<(Option<String>, Value)> = manifests
                    .into_iter()
                    .map(|(kind, url)| (Some(kind), url))
                    .collect();
                extract_formats(client, &manifests, &video_id, &mut formats, &mut subtitles)?;
            }
        }
        let upload_date = video_data
            .pointer("/mediametrie/chapters/0/estatS4")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_strdate(date, true));
        let duration = match video_info.get("duration") {
            Some(Value::Number(number)) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
            Some(Value::String(text)) => text.trim().parse().ok(),
            _ => None,
        };
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                (
                    "thumbnail",
                    video_info.get("preview").cloned().unwrap_or(Value::Null),
                ),
                ("upload_date", upload_date.map_or(Value::Null, Value::from)),
                ("duration", duration.map_or(Value::Null, Value::from)),
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
    fn ids_follow_upstream() {
        assert!(suitable("wat:13641379"));
        assert!(suitable(
            "http://www.wat.tv/video/soupe-figues-l-orange-aux-epices-6z1uz_2hvf7_.html"
        ));
        assert_eq!(media_id("13641379").as_deref(), Some("13641379"));
        assert_eq!(media_id("6z1uz").as_deref(), Some("11713067"));
    }
}
