//! Upstream `CCMAIE` (`yt_dlp/extractor/ccma.py`): 3Cat, TV3 and Catalunya
//! Ràdio video and audio through the `api-media.3cat.cat` media JSON (MP4
//! renditions, DASH manifests, the audio file and the still image).
//! Subtitle tracks are unsupported (upstream lists them without an
//! extension).
use super::{Extractor, dict_format, dict_info, download_json, update_url_query};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Ccma;

const NAME: &str = "CCMA";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?3cat\.cat/(?:3cat|tv3/sx3)/[^/?#]+/(?P<type>video|audio)/(?P<id>\d+)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Upstream's thumbnail dicts keep `url`, `width`, `height` before the
/// `id` and `resolution` the sanitizer adds.
fn order_thumbnails(info: &mut VideoInfo) {
    if let Some(Value::String(shape)) = info.extra.get(crate::info_order::INFO_SHAPE).cloned() {
        info.extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("{shape};thumbnail=url,width,height").into(),
        );
    }
}

/// `codi_etic` ids such as `CE_TP` (all audiences) or `CE_13`.
fn codi_etic_age_limit(code: &str) -> Option<i64> {
    let parts: Vec<&str> = code.split('_').collect();
    match parts.as_slice() {
        [_, "TP"] => Some(0),
        [_, rating] => rating.parse::<i64>().ok(),
        _ => None,
    }
}

/// `int_or_none(value, scale)`.
fn int_or_none(value: Option<&Value>, scale: i64) -> Option<i64> {
    let number = match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64))?,
        Value::String(text) => text.trim().parse::<i64>().ok()?,
        _ => return None,
    };
    Some(number / scale)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `try_get(value, lambda x: x[a][b])`: a nested lookup, `None` on any gap.
fn nested<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, key| value.get(*key))
}

impl Extractor for Ccma {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let media_type = found["type"].to_owned();
        let media_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse("http://api-media.3cat.cat/pvideo/media.jsp")?;
        let api = update_url_query(
            &api,
            &[
                ("media", media_type.as_str()),
                ("idint", media_id.as_str()),
                ("format", "dm"),
            ],
        );
        let media = download_json(client, &api, NAME, &media_id)?;

        let mut formats = Vec::new();
        match media.get("media").and_then(|media| media.get("url")) {
            Some(Value::Array(renditions)) => {
                for rendition in renditions {
                    let Some(format_url) = rendition
                        .get("file")
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                    else {
                        continue;
                    };
                    if super::direct::determine_ext(&format_url).as_deref() == Some("mpd") {
                        let manifest = Url::parse(&format_url)?;
                        if let Some(info) = super::extract_mpd_optional_note(
                            client,
                            &manifest,
                            NAME,
                            &media_id,
                            Some("dash"),
                            "Downloading MPD manifest",
                        ) {
                            formats.extend(info.formats);
                        }
                        continue;
                    }
                    let label = rendition.get("label").and_then(Value::as_str);
                    // `parse_resolution(label)` only has the keys it found.
                    let (width, height) = super::html5::parse_resolution(label.unwrap_or_default());
                    let mut fields = Vec::new();
                    if let Some(width) = width {
                        fields.push(("width", Value::from(width)));
                    }
                    if let Some(height) = height {
                        fields.push(("height", Value::from(height)));
                    }
                    fields.push(("url", format_url.into()));
                    fields.push(("format_id", label.map_or(Value::Null, Value::from)));
                    formats.push(dict_format(fields, NAME, &media_id)?);
                }
            }
            Some(media_url) => {
                let vcodec = (media_type == "audio").then_some("none");
                formats.push(dict_format(
                    vec![
                        ("url", media_url.clone()),
                        ("vcodec", vcodec.map_or(Value::Null, Value::from)),
                    ],
                    NAME,
                    &media_id,
                )?);
            }
            None => return Err(key_error(&media_id, "url")),
        }

        let informacio = media
            .get("informacio")
            .ok_or_else(|| key_error(&media_id, "informacio"))?;
        let title = informacio
            .get("titol")
            .cloned()
            .ok_or_else(|| key_error(&media_id, "titol"))?;
        let durada = informacio
            .get("durada")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let duration =
            match int_or_none(durada.get("milisegons"), 1000).filter(|seconds| *seconds != 0) {
                Some(seconds) => Value::from(seconds),
                None => durada
                    .get("text")
                    .and_then(Value::as_str)
                    .and_then(crate::parse_duration)
                    .map_or(Value::Null, |duration| {
                        serde_json::to_value(duration).unwrap_or(Value::Null)
                    }),
            };
        let tematica = nested(informacio, &["tematica", "text"]).and_then(Value::as_str);
        let timestamp = nested(informacio, &["data_emissio", "utc"])
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, false))
            .map_or(Value::Null, Value::from);

        let subtitols = match media.get("subtitols") {
            Some(Value::Object(subtitle)) => vec![Value::Object(subtitle.clone())],
            Some(Value::Array(subtitles)) => subtitles.clone(),
            _ => Vec::new(),
        };
        if subtitols
            .iter()
            .any(|subtitle| subtitle.get("url").is_some_and(super::nuxt::truthy))
        {
            return Err(Error::UnsupportedHtml5(
                "CCMA subtitle tracks are not supported natively".into(),
            ));
        }

        let mut thumbnails = Vec::new();
        if let Some(imatges) = media
            .get("imatges")
            .filter(|images| super::nuxt::truthy(images))
            && let Some(thumbnail_url) = imatges.get("url").and_then(Value::as_str)
        {
            thumbnails.push(json!({
                "url": thumbnail_url,
                "width": int_or_none(imatges.get("amplada"), 1),
                "height": int_or_none(imatges.get("alcada"), 1),
            }));
        }

        let age_limit = nested(informacio, &["codi_etic", "id"])
            .and_then(Value::as_str)
            .and_then(codi_etic_age_limit);
        let description = informacio
            .get("descripcio")
            .and_then(Value::as_str)
            .map(crate::html::clean_html);
        let categories = tematica.map(|tematica| json!([tematica]));
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", media_id.clone().into()),
                ("title", title),
                ("description", description.map_or(Value::Null, Value::from)),
                ("duration", duration),
                ("timestamp", timestamp),
                ("thumbnails", Value::Array(thumbnails)),
                ("subtitles", Value::Null),
                ("formats", Value::Null),
                ("age_limit", age_limit.map_or(Value::Null, Value::from)),
                (
                    "alt_title",
                    informacio
                        .get("titol_complet")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "episode_number",
                    int_or_none(informacio.get("capitol"), 1).map_or(Value::Null, Value::from),
                ),
                ("categories", categories.unwrap_or(Value::Null)),
                (
                    "series",
                    informacio.get("programa").cloned().unwrap_or(Value::Null),
                ),
            ],
            formats,
        );
        order_thumbnails(&mut info);
        info.subtitles = crate::Subtitles::default();
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.3cat.cat/3cat/lespot-de-la-marato-de-tv3/video/5630208/"
        ));
        assert!(suitable(
            "https://www.3cat.cat/tv3/sx3/una-mosca-volava-per-la-llum/video/5759227/"
        ));
        assert!(suitable(
            "https://www.3cat.cat/3cat/el-consell-de-savis-analitza-el-derbi/audio/943685/"
        ));
        assert!(!suitable(
            "https://www.3cat.cat/3cat/lespot/programa/5630208/"
        ));
    }

    #[test]
    fn codi_etic_age_limits() {
        assert_eq!(codi_etic_age_limit("CE_TP"), Some(0));
        assert_eq!(codi_etic_age_limit("CE_13"), Some(13));
        assert_eq!(codi_etic_age_limit("CE"), None);
    }
}
