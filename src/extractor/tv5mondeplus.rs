//! Upstream `TV5MondePlusIE` (`yt_dlp/extractor/tv5mondeplus.py`): a
//! tv5monde.com video page (requested with impersonation), its
//! `video_player_loader` element's `data-broadcast` files (deferred files
//! resolved through the player API with their bearer token), metadata,
//! captions, the France-only geo error and the page title and summary.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, SubtitleFormat, Subtitles, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Tv5MondePlus;

const NAME: &str = "TV5MONDE";
const KEY: &str = "TV5MondePlus";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?tv5monde\.com/tv/video/(?P<id>[^/?#]+)")
        .unwrap()
});
static LOADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(<[^>]+class="video_player_loader"[^>]+>)"#).unwrap());
static MATERIAL_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"materials/([\da-zA-Z]{10}_[\da-fA-F]{7})/").unwrap());
static PAGE_IDS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"data-guid=["']([\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12})"#)
            .unwrap(),
        Regex::new(r#"id_contenu["']\s:\s*(\d+)"#).unwrap(),
    ]
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("TV5MONDE {what} are not supported natively"))
}

/// `with impersonation`: a request upstream makes with `impersonate=True`.
fn impersonated<T>(request: impl FnOnce() -> Result<T>) -> Result<T> {
    let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
        crate::screen::warning_once(&format!("[{NAME}] {message}"));
    });
    request()
}

/// `_parse_json(text, display_id, fatal=False)`: `None` after the warning.
fn parse_json_optional(text: &str, id: &str) -> Option<Value> {
    match crate::json::from_str::<Value>(text) {
        Ok(value) => Some(value),
        Err(error) => {
            // `LenientJSONDecoder`'s error text.
            let text =
                crate::json::python_decode_error_text(text).unwrap_or_else(|| error.to_string());
            crate::screen::warning(&format!("[{NAME}] {id}: Failed to parse JSON: {text}"));
            None
        }
    }
}

/// `get_element_by_class(class, html)`: the first element's text.
fn element_text(class: &str, html: &str) -> Result<Option<String>> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .map(|elements| elements.into_iter().next().map(|(text, _)| text))
        .map_err(|error| unsupported(&format!("pages whose HTML does not parse ({error})")))
}

/// `get_element_html_by_class(class, html)`: the first element's HTML.
fn element_html(class: &str, html: &str) -> Result<Option<String>> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .map(|elements| elements.into_iter().next().map(|(_, whole)| whole))
        .map_err(|error| unsupported(&format!("pages whose HTML does not parse ({error})")))
}

/// The deferred asset's `resolve` JSON (`fatal=False`): `None` after
/// upstream's warning.
fn deferred(client: &Client, asset: &str, token: &str, id: &str) -> Result<Option<Value>> {
    use std::io::Read;
    // `urllib.parse.quote(v_url)` (`/` stays).
    let quoted: String = asset
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"_.-~/".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    let url = Url::parse(&format!(
        "https://api.tv5monde.com/player/asset/{quoted}/resolve?condenseKS=true"
    ))?;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading deferred info"));
    let authorization = format!("Bearer {token}");
    let response = impersonated(|| {
        super::direct::request_with_headers(
            client,
            &url,
            None,
            &[("Authorization", &authorization)],
        )
    });
    let mut response = match response {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    Ok(parse_json_optional(&text, id))
}

/// `process_video_files(v)`.
fn process_files(
    client: &Client,
    files: &Value,
    id: &str,
    formats: &mut Vec<VideoFormat>,
    video_id: &mut Option<String>,
) -> Result<()> {
    let Value::Array(files) = files else {
        return Err(unsupported("broadcast data that are not lists"));
    };
    for file in files {
        let Some(mut file_url) = file
            .get("url")
            .filter(|url| super::nuxt::truthy(url))
            .map(|url| {
                url.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| unsupported("file URLs that are not strings"))
            })
            .transpose()?
        else {
            continue;
        };
        if file.get("type").and_then(Value::as_str) == Some("application/deferred") {
            let Some(token) = file
                .get("token")
                .filter(|token| super::nuxt::truthy(token))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let Some(resolved) = deferred(client, &file_url, token, id)? else {
                continue;
            };
            // `traverse_obj(deferred_json, (0, 'url', {url_or_none}))`.
            let Some(resolved_url) = resolved
                .get(0)
                .and_then(|entry| entry.get("url"))
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            // The material ID of the resolved URL (the page's `data-guid`
            // is not stable).
            *video_id = MATERIAL_ID
                .captures(&resolved_url)
                .map(|found| found[1].to_owned());
            process_files(client, &resolved, id, formats, video_id)?;
            file_url = resolved_url;
        }
        let format = file
            .get("format")
            .filter(|format| super::nuxt::truthy(format))
            .map(|format| {
                format
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| unsupported("file formats that are not strings"))
            })
            .transpose()?
            .or_else(|| super::direct::determine_ext(&file_url))
            .unwrap_or_else(|| "unknown_video".into());
        match format.as_str() {
            "m3u8" => {
                if let Some(stream) = super::extract_m3u8_master_optional(
                    client,
                    &Url::parse(&file_url)?,
                    NAME,
                    id,
                    Some("hls"),
                    false,
                    false,
                ) {
                    formats.extend(stream.formats);
                }
            }
            "mpd" => {
                if let Some(stream) =
                    super::extract_mpd_optional(client, &Url::parse(&file_url)?, NAME, id, false)
                {
                    formats.extend(stream.formats);
                }
            }
            _ => formats.push(dict_format(
                vec![("url", file_url.into()), ("format_id", format.into())],
                NAME,
                id,
            )?),
        }
    }
    Ok(())
}

/// `_extract_subtitles(data_captions)`.
fn subtitles(captions: Option<&Value>) -> Result<Subtitles> {
    let mut subtitles = Subtitles::default();
    let files = captions
        .and_then(|captions| captions.get("files"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for file in files {
        let Some(url) = file
            .get("file")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let language = file
            .get("label")
            .filter(|label| super::nuxt::truthy(label))
            .and_then(Value::as_str)
            .unwrap_or("fra")
            .to_owned();
        let parsed =
            Url::parse(&url).map_err(|_| unsupported("caption URLs that are not absolute"))?;
        let ext = super::direct::determine_ext(&url)
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
        let language = Some(language);
        match subtitles
            .entries
            .iter_mut()
            .find(|(key, _)| *key == language)
        {
            Some((_, tracks)) => tracks.push(track),
            None => subtitles.entries.push((language, vec![track])),
        }
    }
    Ok(subtitles)
}

impl Extractor for Tv5MondePlus {
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
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = impersonated(|| download_webpage(client, url, NAME, &display_id))?;
        if webpage.contains(
            ">Ce programme n'est malheureusement pas disponible pour votre zone géographique.<",
        ) {
            return Err(crate::geo::restricted(
                &format!(
                    "[{NAME}] {display_id}: This video is not available from your location due to geo restriction"
                ),
                &["FR"],
            ));
        }
        let loader = LOADER
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract video player loader; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let attributes = crate::html_elements::extract_attributes(&loader);
        let attribute = |name: &str| attributes.get(name).and_then(Value::as_str);
        let broadcast = attribute("data-broadcast").ok_or_else(|| {
            Error::Reported(format!(
                "{display_id}: An extractor error has occurred. (caused by KeyError('data-broadcast')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let files: Value = crate::json::from_str(broadcast).map_err(|_| {
            let cause = crate::json::python_decode_error_text(broadcast)
                .map(|text| format!(" (caused by JSONDecodeError({text:?}))"))
                .unwrap_or_default();
            Error::Reported(format!(
                "[{NAME}] {display_id}: {display_id}: Failed to parse JSON{cause}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let mut formats = Vec::new();
        let mut video_id = None;
        process_files(client, &files, &display_id, &mut formats, &mut video_id)?;

        let metadata = parse_json_optional(
            attribute("data-metadata")
                .filter(|text| !text.is_empty())
                .unwrap_or("{}"),
            &display_id,
        );
        let video_id = video_id.unwrap_or_else(|| {
            PAGE_IDS
                .iter()
                .find_map(|pattern| pattern.captures(&webpage))
                .map_or_else(|| display_id.clone(), |found| found[1].to_owned())
        });

        // `traverse_obj(metadata, ('content', {...}))`.
        let content = metadata
            .as_ref()
            .and_then(|metadata| metadata.get("content"));
        let get = |key: &str| content.and_then(|content| content.get(key));
        let text = |key: &str| get(key).filter(|value| value.is_string()).cloned();
        let int = |key: &str| {
            Some(super::streamable::int_or_none(get(key))).filter(|value| !value.is_null())
        };
        let mut fields: Vec<(&str, Value)> = [
            ("id", text("id")),
            ("title", text("title")),
            ("episode", text("title")),
            ("series", text("series")),
            ("timestamp", int("publishDate_ts")),
            ("duration", int("duration")),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect();
        let title =
            element_text("main-title", &webpage)?.map(|title| crate::html::clean_html(&title));
        let summary = element_html("ep-summary", &webpage)?.unwrap_or_default();
        let description =
            element_text("text", &summary)?.map(|text| crate::html::clean_html(&text));
        let thumbnail = attribute("data-image").and_then(super::json_ld::url_or_none);
        let captions = attribute("data-captions")
            .map(|captions| parse_json_optional(captions, &display_id))
            .unwrap_or_else(|| Some(Value::Object(Map::new())));
        let subtitles = subtitles(captions.as_ref())?;
        for (key, value) in [
            ("id", Value::from(video_id)),
            ("display_id", Value::from(display_id.as_str())),
            ("title", title.map_or(Value::Null, Value::from)),
            ("description", description.map_or(Value::Null, Value::from)),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ] {
            match fields.iter_mut().find(|(known, _)| *known == key) {
                Some(field) => field.1 = value,
                None => fields.push((key, value)),
            }
        }
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
            "https://www.tv5monde.com/tv/video/65931-baudouin-l-heritage-d-un-roi-baudouin-l-heritage-d-un-roi"
        ));
        assert!(!suitable("https://www.tv5monde.com/tv/emissions"));
    }
}
