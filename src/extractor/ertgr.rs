//! Upstream `ERTFlixCodenameIE` (`yt_dlp/extractor/ertgr.py`): an ERTFLIX title
//! by its codename (`ertflix:<codename>`) through the ERTFLIX player API: the
//! HLS and DASH manifests and direct files of its `main` media role. The
//! series and VOD pages (`ERTFlixIE`) and the ert.gr webtv embeds
//! (`ERTWebtvEmbedIE`) are not ported yet.
use super::{Extractor, dict_format, dict_info, download_json, update_url_query};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct ERTFlixCodename;

const NAME: &str = "ertflix:codename";
const KEY: &str = "ERTFlixCodename";
const API: &str = "https://api.app.ertflix.gr/v1/Player/AcquireContent";
/// `json.dumps` of the `$headers` query parameter (Python's separators).
const API_HEADERS: &str = r#"{"X-Api-Date-Format": "iso", "X-Api-Camel-Case": false}"#;

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ertflix:(?P<id>[\w-]+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `url_or_none` on a JSON value: the string itself when it is a URL.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    (Url::parse(text).is_ok() && !text.contains(char::is_whitespace)).then(|| text.to_owned())
}

/// `str_or_none` on a JSON value.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

impl Extractor for ERTFlixCodename {
    fn key(&self) -> &'static str {
        KEY
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
        let api = Url::parse(API)?;
        let api = update_url_query(
            &api,
            &[
                ("platformCodename", "www"),
                ("$headers", API_HEADERS),
                ("codename", video_id.as_str()),
            ],
        );
        // `_call_api` (non-fatal): an unsuccessful answer is no media info.
        let response = download_json(client, &api, NAME, &video_id)
            .ok()
            .filter(|response| {
                response
                    .get("Result")
                    .and_then(|result| result.get("Success"))
                    == Some(&Value::Bool(true))
            });
        let Some(response) = response else {
            return Err(Error::UnsupportedHtml5(format!(
                "ERTFLIX codename {video_id} without a successful player API answer returns nothing upstream and is not supported natively"
            )));
        };

        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let media_files = response
            .get("MediaFiles")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for media in media_files
            .iter()
            .filter(|media| media.get("RoleCodename").and_then(Value::as_str) == Some("main"))
        {
            let items = media
                .get("Formats")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in &items {
                let Some(format_url) = url_or_none(item.get("Url")) else {
                    continue;
                };
                let ext = super::direct::determine_ext(&format_url).unwrap_or_default();
                if ext == "m3u8" {
                    let manifest = Url::parse(&format_url)?;
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
                        for (lang, tracks) in stream.subtitles.entries {
                            match subtitles.entries.iter_mut().find(|(key, _)| *key == lang) {
                                Some((_, existing)) => existing.extend(tracks),
                                None => subtitles.entries.push((lang, tracks)),
                            }
                        }
                    }
                } else if ext == "mpd" {
                    let manifest = Url::parse(&format_url)?;
                    if let Some(stream) = super::extract_mpd_optional_note(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some("dash"),
                        "Downloading MPD manifest",
                    ) {
                        formats.extend(stream.formats);
                    }
                } else {
                    formats.push(dict_format(
                        vec![
                            ("url", format_url.clone().into()),
                            (
                                "format_id",
                                str_or_none(item.get("Id")).map_or(Value::Null, Value::from),
                            ),
                        ],
                        NAME,
                        &video_id,
                    )?);
                }
            }
        }

        if formats.is_empty() {
            return Err(Error::UnsupportedHtml5(format!(
                "ERTFLIX codename {video_id} without formats returns nothing upstream and is not supported natively"
            )));
        }
        // `'title': self._generic_title(url)`: the codename is the URL's basename.
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.clone().into()),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("title", video_id.clone().into()),
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
        assert!(suitable("ertflix:monogramma-praxitelis-tzanoylinos"));
        assert!(!suitable(
            "https://www.ertflix.gr/series/ser.3448-monogramma"
        ));
    }
}
