//! Upstream `AdobeTVVideoIE` (`yt_dlp/extractor/adobetv.py`): the page's
//! `bridge` JSON, its non-playlist sources (HLS playlists or direct files)
//! and translation subtitles.
use super::clyp::float_or_none;
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct AdobeTvVideo;

const NAME: &str = "adobetv";
const KEY: &str = "AdobeTVVideo";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://video\.tv\.adobe\.com/v/(?P<id>\d+)").unwrap());
static BRIDGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"var\s+bridge\s*=").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Adobe TV {what} are not supported natively"))
}

/// `url_or_none`.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `traverse_obj(source, {...})`: the source's format fields, `None`
/// values dropped, in upstream's key order.
fn source_fields(source: &Map<String, Value>) -> Vec<(&'static str, Value)> {
    let duration = float_or_none(source.get("duration"))
        .as_f64()
        .map_or(Value::Null, |ms| Value::from(ms / 1000.0));
    let filesize = match int_or_none(source.get("kilobytes")) {
        Value::Number(kilobytes) => kilobytes
            .as_i64()
            .map_or(Value::Null, |kilobytes| Value::from(kilobytes * 1000)),
        _ => Value::Null,
    };
    // `(('format', 'label'), {str}, all, {lambda x: join_nonempty(*x)})`.
    let format_id = ["format", "label"]
        .iter()
        .filter_map(|key| source.get(*key).and_then(Value::as_str))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let format_id = if format_id.is_empty() {
        Value::Null
    } else {
        Value::from(format_id)
    };
    [
        ("duration", duration),
        ("filesize", filesize),
        ("format_id", format_id),
        ("height", int_or_none(source.get("height"))),
        ("tbr", int_or_none(source.get("bitrate"))),
        ("width", int_or_none(source.get("width"))),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_null())
    .collect()
}

/// `fmt.update(fields)` of a parsed HLS format.
fn update_format(format: &mut crate::VideoFormat, fields: &[(&str, Value)]) -> Result<()> {
    for (key, value) in fields {
        let in_place = match *key {
            "format_id" => true,
            "height" => format.height.is_some(),
            "width" => format.width.is_some(),
            "tbr" => format.tbr.is_some(),
            "filesize" => format.filesize.is_some(),
            _ => format.extra.contains_key(*key),
        };
        if in_place {
            crate::info_order::mark_format_update_in_place(format, key);
        } else {
            crate::info_order::mark_format_update(format, key);
        }
        match *key {
            "format_id" => format.format_id = value.as_str().unwrap_or_default().to_owned(),
            "height" => format.height = value.as_u64().and_then(|v| u32::try_from(v).ok()),
            "width" => format.width = value.as_u64().and_then(|v| u32::try_from(v).ok()),
            "tbr" => format.tbr = value.as_f64(),
            "filesize" => format.filesize = value.as_u64(),
            "duration" => {
                format.extra.insert("duration".into(), value.clone());
            }
            _ => return Err(unsupported("source fields")),
        }
    }
    Ok(())
}

impl Extractor for AdobeTvVideo {
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
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let video_data = super::search_json(&BRIDGE, &webpage, false)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract bridged data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .ok_or_else(|| unsupported("bridged data that are not JSON"))?;

        let mut formats = Vec::new();
        let sources = match video_data.get("sources") {
            Some(Value::Array(items)) => items.iter().collect(),
            Some(Value::Object(map)) => map.values().collect(),
            _ => Vec::new(),
        };
        for source in sources.into_iter().filter_map(Value::as_object) {
            // `lambda _, v: v['format'] != 'playlist' and url_or_none(v['src'])`.
            let Some(format) = source.get("format") else {
                continue;
            };
            if format.as_str() == Some("playlist") {
                continue;
            }
            let Some(source_url) = url_or_none(source.get("src")) else {
                continue;
            };
            let source_url = super::proto_relative(&source_url);
            let fields = source_fields(source);
            if super::direct::determine_ext(&source_url).as_deref() == Some("m3u8") {
                let manifest = Url::parse(&source_url)?;
                let Some(stream) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("hls"),
                    false,
                    false,
                ) else {
                    continue;
                };
                for mut format in stream.formats {
                    if format.ext.is_empty() || format.ext == "unknown_video" {
                        format.ext = "mp4".into();
                    }
                    update_format(&mut format, &fields)?;
                    if fields.iter().any(|(key, _)| *key == "tbr") {
                        crate::info_order::mark_int_rates(&mut format);
                    }
                    formats.push(format);
                }
            } else {
                let int_tbr = fields.iter().any(|(key, _)| *key == "tbr");
                let mut dict = vec![("url", Value::from(source_url))];
                dict.extend(fields);
                let mut format = dict_format(dict, KEY, &video_id)?;
                if int_tbr {
                    crate::info_order::mark_int_rates(&mut format);
                }
                formats.push(format);
            }
        }

        let mut subtitles = crate::Subtitles::default();
        let translations = match video_data.get("translations") {
            Some(Value::Array(items)) => items.iter().collect(),
            Some(Value::Object(map)) => map.values().collect(),
            _ => Vec::new(),
        };
        for translation in translations {
            let Some(vtt_path) = url_or_none(translation.get("vttPath")) else {
                continue;
            };
            let lang = match translation
                .get("language_w3c")
                .filter(|value| super::nuxt::truthy(value))
            {
                Some(Value::String(lang)) => lang.clone(),
                Some(_) => return Err(unsupported("subtitle languages that are not strings")),
                None if translation
                    .get("language_medium")
                    .is_some_and(super::nuxt::truthy) =>
                {
                    return Err(unsupported("ISO 639-2 subtitle languages"));
                }
                None => "und".to_owned(),
            };
            let track = crate::SubtitleFormat {
                name: None,
                url: Url::parse(&super::proto_relative(&vtt_path))?,
                ext: "vtt".into(),
                protocol: None,
                fragments: Vec::new(),
                extra: Map::new(),
            };
            let lang = Some(lang);
            match subtitles.entries.iter_mut().find(|(key, _)| *key == lang) {
                Some((_, tracks)) => tracks.push(track),
                None => subtitles.entries.push((lang, vec![track])),
            }
        }

        let clean = |key: &str| match video_data.get(key) {
            Some(Value::String(text)) => Ok(Some(crate::html::clean_html(text))),
            None | Some(Value::Null) => Ok(None),
            Some(_) => Err(unsupported("metadata that are not strings")),
        };
        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        if let Some(title) = clean("title")? {
            fields.push(("title", title.into()));
        }
        if let Some(description) = clean("description")?.filter(|text| !text.is_empty()) {
            fields.push(("description", description.into()));
        }
        if let Some(thumbnail) = url_or_none(
            video_data
                .get("video")
                .and_then(|video| video.get("poster"))
                .and_then(Value::as_str)
                .map(|poster| Value::from(super::proto_relative(poster)))
                .as_ref(),
        ) {
            fields.push(("thumbnail", thumbnail.into()));
        }
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        info.subtitles = subtitles;
        let shape = info
            .extra
            .get(crate::info_order::INFO_SHAPE)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("{shape};subtitle=ext,url"),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://video.tv.adobe.com/v/2456"));
        assert!(suitable(
            "https://video.tv.adobe.com/v/3463980/adobe-acrobat"
        ));
        assert!(!suitable("https://tv.adobe.com/v/2456"));
    }

    #[test]
    fn source_fields_follow_upstream_traversal() {
        let source = serde_json::json!({"format": "mpeg4", "label": "HD", "duration": 248522,
            "kilobytes": 36235, "bitrate": 1036, "width": 1280, "height": 720});
        let fields = source_fields(source.as_object().unwrap());
        assert_eq!(
            fields,
            vec![
                ("duration", Value::from(248.522)),
                ("filesize", Value::from(36_235_000)),
                ("format_id", Value::from("mpeg4-HD")),
                ("height", Value::from(720)),
                ("tbr", Value::from(1036)),
                ("width", Value::from(1280)),
            ]
        );
    }
}
