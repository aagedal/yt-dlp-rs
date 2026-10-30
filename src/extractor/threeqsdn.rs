//! Upstream `ThreeQSDNIE` (`yt_dlp/extractor/threeqsdn.py`): 3Q SDN playout
//! pages through their `config` JSON, with DASH, HLS and progressive sources
//! and the subtitle tracks. The embed path (`_extract_from_webpage`, which
//! yields `url_transparent` results with the uploader) is not ported.
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct ThreeQsdn;

const NAME: &str = "3qsdn";
const KEY: &str = "ThreeQSDN";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://playout\.3qsdn\.com/(?P<id>[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12})",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// The HTTP status of a request error (through the JSON download's wrapper).
fn http_status(error: &Error) -> Option<u16> {
    match error {
        Error::ExtractorRequest { source, .. } => http_status(source),
        Error::Http(source) | Error::HttpRetryAfter { source, .. } => {
            source.status().map(|status| status.as_u16())
        }
        _ => None,
    }
}

/// A source's URL string (upstream passes it straight to the manifest reader).
fn source_url(id: &str, source_type: &str, source: &Value) -> Result<Url> {
    source
        .as_str()
        .and_then(|text| Url::parse(text).ok())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unsupported {source_type} source value"
            ))
        })
}

/// `_is_valid_url`-checked progressive formats (`ext`, `format_id`, ...).
fn progressive_format(
    client: &Client,
    id: &str,
    aspect: Option<f64>,
    source: &Value,
) -> Result<Option<crate::VideoFormat>> {
    let Some(src) = source
        .get("src")
        .and_then(Value::as_str)
        .filter(|src| !src.is_empty())
    else {
        return Ok(None);
    };
    if !super::is_valid_url(client, src, NAME, id, "video")? {
        return Ok(None);
    }
    let ext = super::direct::determine_ext(src).unwrap_or_else(|| "unknown_video".into());
    let height = super::streamable::int_or_none(source.get("height")).as_i64();
    let mut format_id = vec!["http".to_owned(), ext.clone()];
    if let Some(height) = height.filter(|height| *height != 0) {
        format_id.push(format!("{height}p"));
    }
    let width = match (height, aspect) {
        (Some(height), Some(aspect)) if height != 0 && aspect != 0.0 => {
            Value::from((height as f64 * aspect).trunc() as i64)
        }
        _ => Value::Null,
    };
    let vcodec = if height == Some(0) {
        Value::from("none")
    } else {
        Value::Null
    };
    dict_format(
        vec![
            ("ext", ext.into()),
            ("format_id", format_id.join("-").into()),
            ("height", height.map_or(Value::Null, Value::from)),
            ("source_preference", 0.into()),
            ("url", src.into()),
            ("vcodec", vcodec),
            ("width", width),
        ],
        NAME,
        id,
    )
    .map(Some)
}

impl Extractor for ThreeQsdn {
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
        let config_url = Url::parse(
            &url.as_str()
                .replace("://playout.3qsdn.com/", "://playout.3qsdn.com/config/"),
        )?;
        let config = match download_json(client, &config_url, NAME, &video_id) {
            Ok(config) => config,
            Err(error) if http_status(&error) == Some(401) => {
                return Err(crate::geo::restricted(
                    &format!(
                        "[{NAME}] {video_id}: This video is not available from your location due to geo restriction"
                    ),
                    &[],
                ));
            }
            Err(error) => return Err(error),
        };

        let live = config.get("streamContent").and_then(Value::as_str) == Some("live");
        let aspect = super::clyp::float_or_none(config.get("aspect")).as_f64();

        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        if let Some(sources) = config.get("sources").and_then(Value::as_object) {
            for (source_type, source) in sources {
                if !super::nuxt::truthy(source) {
                    continue;
                }
                match source_type.as_str() {
                    "dash" => {
                        let manifest = source_url(&video_id, source_type, source)?;
                        if let Some(mut stream) = super::extract_mpd_optional_note(
                            client,
                            &manifest,
                            NAME,
                            &video_id,
                            Some("mpd"),
                            "Downloading MPD manifest",
                        ) {
                            formats.append(&mut stream.formats);
                            super::html5::merge_subtitles(
                                &mut subtitles,
                                std::mem::take(&mut stream.subtitles),
                            );
                        }
                    }
                    "hls" => {
                        let manifest = source_url(&video_id, source_type, source)?;
                        if let Some(mut stream) = super::extract_m3u8_master_optional(
                            client,
                            &manifest,
                            NAME,
                            &video_id,
                            Some("hls"),
                            true,
                            live,
                        ) {
                            formats.append(&mut stream.formats);
                            super::html5::merge_subtitles(
                                &mut subtitles,
                                std::mem::take(&mut stream.subtitles),
                            );
                        }
                    }
                    "progressive" => {
                        let Some(files) = source.as_array() else {
                            return Err(Error::Reported(format!(
                                "[{NAME}] {video_id}: Unsupported progressive sources"
                            )));
                        };
                        for file in files {
                            if let Some(format) =
                                progressive_format(client, &video_id, aspect, file)?
                            {
                                formats.push(format);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        for subtitle in config
            .get("subtitles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(src) = subtitle
                .get("src")
                .and_then(Value::as_str)
                .filter(|src| !src.is_empty())
            else {
                continue;
            };
            let label = subtitle
                .get("label")
                .and_then(Value::as_str)
                .filter(|label| !label.is_empty())
                .unwrap_or("eng")
                .to_owned();
            let url = Url::parse(src)?;
            let ext = super::direct::determine_ext(src).unwrap_or_default();
            let track = crate::SubtitleFormat {
                name: None,
                url,
                ext,
                protocol: None,
                fragments: Vec::new(),
                extra: serde_json::Map::new(),
            };
            let language = Some(label);
            match subtitles
                .entries
                .iter_mut()
                .find(|(key, _)| *key == language)
            {
                Some((_, tracks)) => tracks.push(track),
                None => subtitles.entries.push((language, vec![track])),
            }
        }

        let title = config
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .unwrap_or(&video_id)
            .to_owned();
        let text_or_null = |key: &str| match config.get(key) {
            Some(Value::String(text)) if !text.is_empty() => Value::from(text.as_str()),
            _ => Value::Null,
        };
        let duration = super::clyp::float_or_none(config.get("vlength"))
            .as_f64()
            .filter(|duration| *duration != 0.0)
            .map_or(Value::Null, Value::from);
        let timestamp = config
            .get("upload_date")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);

        let fields = vec![
            ("id", Value::from(video_id)),
            ("title", Value::from(title)),
            ("thumbnail", text_or_null("poster")),
            ("description", text_or_null("description")),
            ("timestamp", timestamp),
            ("duration", duration),
            ("is_live", Value::Bool(live)),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("_format_sort_fields", json!(["res", "source_preference"])),
        ];
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
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
            "https://playout.3qsdn.com/7201c779-6b3c-11e7-a40e-002590c750be"
        ));
        assert!(suitable(
            "http://playout.3qsdn.com/0280d6b9-1215-11e6-b427-0cc47a188158?protocol=http"
        ));
        assert!(!suitable("https://playout.3qsdn.com/not-a-uuid"));
        assert!(!suitable("https://player.3qsdn.com/demo.html"));
    }
}
