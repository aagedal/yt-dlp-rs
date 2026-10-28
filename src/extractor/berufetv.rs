//! Upstream `BerufeTVIE` (`yt_dlp/extractor/berufetv.py`): BerufeTV films from
//! the movingimage player JSON (its `auto` HLS playlist and progressive
//! sources, and subtitle tracks), with the film metadata API's title,
//! description, thumbnail, category and topics.
use super::{Extractor, dict_format, dict_info, download_json_note, download_json_note_headers};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BerufeTv;

const NAME: &str = "BerufeTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?web\.arbeitsagentur\.de/berufetv/[^?#]+/film;filmId=(?P<id>[\w-]+)",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("BerufeTV {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for BerufeTv {
    fn key(&self) -> &'static str {
        NAME
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
        // `_download_json(..., fatal=False)`.
        let metadata = match download_json_note_headers(
            client,
            &Url::parse("https://rest.arbeitsagentur.de/infosysbub/berufetv/pc/v1/film-metadata")?,
            NAME,
            &video_id,
            "Downloading JSON metadata",
            &[("X-API-Key", "79089773-4892-4386-86e6-e8503669f426")],
        ) {
            Ok(metadata) => metadata,
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Unable to download JSON metadata: {}",
                    crate::upstream_error_text(&source)
                ));
                Value::Null
            }
            Err(error) => return Err(error),
        };
        // `('metadaten', lambda _, i: video_id == i['miId'])`, first.
        let empty = json!({});
        let meta = match metadata.get("metadaten") {
            Some(Value::Array(items)) => items
                .iter()
                .find(|item| item.get("miId").and_then(Value::as_str) == Some(video_id.as_str())),
            _ => None,
        }
        .unwrap_or(&empty);
        let video = download_json_note(
            client,
            &Url::parse(&format!(
                "https://d.video-cdn.net/play/player/8YRzUk6pTzmBdrsLe9Y88W/video/{video_id}"
            ))?,
            NAME,
            &video_id,
            "Downloading video JSON",
        )?;
        let sources = match video.pointer("/videoSources/html") {
            Some(Value::Object(sources)) => sources,
            Some(_) => return Err(unsupported("source maps that are not objects")),
            None => {
                let key = if video.get("videoSources").is_none() {
                    "videoSources"
                } else {
                    "html"
                };
                return Err(key_error(&video_id, key));
            }
        };
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        for (key, source) in sources {
            let first = source
                .get(0)
                .ok_or_else(|| unsupported("empty source lists"))?;
            let source_url = match first.get("source") {
                Some(Value::String(source_url)) => source_url.clone(),
                _ => return Err(unsupported("sources without a URL")),
            };
            if key == "auto" {
                let manifest = Url::parse(&source_url)
                    .map_err(|_| unsupported("playlist URLs that are not URLs"))?;
                let mut stream = super::extract_m3u8_fatal_master(
                    client, &manifest, NAME, &video_id, None, true,
                )?;
                super::hls::without_ext(&mut stream);
                formats.extend(stream.formats);
                subtitles = stream.subtitles;
            } else {
                let mime = first
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                formats.push(dict_format(
                    vec![
                        ("url", source_url.into()),
                        ("ext", super::direct::mimetype2ext_or(mime, None).into()),
                        ("format_id", key.clone().into()),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        if let Some(Value::Array(tracks)) = video.get("videoTracks") {
            for track in tracks {
                if track.get("type").and_then(Value::as_str) != Some("SUBTITLES") {
                    continue;
                }
                let language = track
                    .get("language")
                    .and_then(Value::as_str)
                    .ok_or_else(|| unsupported("subtitle tracks without a language"))?;
                let mut entry = json!({"url": track.get("source").cloned().unwrap_or(Value::Null), "ext": "vtt"});
                if let Some(label) = track.get("label").and_then(Value::as_str) {
                    entry["name"] = label.into();
                }
                let format = serde_json::from_value(entry)
                    .map_err(|_| unsupported("subtitle tracks without a URL"))?;
                let language = Some(language.to_owned());
                match subtitles
                    .entries
                    .iter_mut()
                    .find(|(key, _)| *key == language)
                {
                    Some((_, tracks)) => tracks.push(format),
                    None => subtitles.entries.push((language, vec![format])),
                }
            }
        }
        let get = |key: &str| meta.get(key).filter(|value| !value.is_null()).cloned();
        let title = get("titel")
            .filter(super::nuxt::truthy)
            .or_else(|| video.pointer("/videoMetaData/title").cloned())
            .unwrap_or(Value::Null);
        let thumbnail = get("thumbnail")
            .filter(super::nuxt::truthy)
            .unwrap_or_else(|| {
                format!("https://asset-out-cdn.video-cdn.net/private/videos/{video_id}/thumbnails/active").into()
            });
        let duration = video
            .get("duration")
            .and_then(Value::as_f64)
            .map_or(Value::Null, |millis| json!(millis / 1000.0));
        let categories = get("kategorie")
            .filter(super::nuxt::truthy)
            .map_or(Value::Null, |category| json!([category]));
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("description", get("beschreibung").unwrap_or(Value::Null)),
                ("thumbnail", thumbnail),
                ("duration", duration),
                ("categories", categories),
                ("tags", get("themengebiete").unwrap_or(Value::Null)),
                ("subtitles", Value::Null),
                ("formats", Value::Null),
            ],
            formats,
        );
        info.subtitles = subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";subtitle=url,name,ext");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://web.arbeitsagentur.de/berufetv/studienberufe/wirtschaftswissenschaften/wirtschaftswissenschaften-volkswirtschaft/film;filmId=DvKC3DUpMKvUZ_6fEnfg3u"
        ));
        assert!(!suitable("https://web.arbeitsagentur.de/berufetv/"));
    }
}
