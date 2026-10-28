//! Upstream `SverigesRadioPublicationIE` and `SverigesRadioEpisodeIE`
//! (`yt_dlp/extractor/sverigesradio.py`): the audio ID (else the page's
//! `audio-button` or `gtm-metadata` page ID), the `playerajax`
//! `audiometadata` item and one `getaudiourl` format per quality.
use super::{Extractor, dict_format, dict_info, download_json_note, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct SverigesRadioPublication;
pub struct SverigesRadioEpisode;

const BASE_URL: &str = "https://sverigesradio.se/sida/playerajax/";
const QUALITIES: [&str; 3] = ["low", "medium", "high"];

static PUBLICATION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?sverigesradio\.se/(?:sida/)?(?:artikel|gruppsida)(?:\.aspx\?.*?\bartikel=(?P<id>[0-9]+)|/(?P<slug>[\w-]+))",
    )
    .unwrap()
});
static EPISODE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?sverigesradio\.se/(?:sida/)?avsnitt/(?:(?P<id>\d+)|(?P<slug>[\w-]+))(?:$|[#?])",
    )
    .unwrap()
});
static AUDIO_BITRATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_a(\d+)\.m4a").unwrap());

/// `SverigesRadioPublicationIE._match_valid_url`.
pub fn publication_suitable(url: &str) -> bool {
    PUBLICATION_URL.is_match(url)
}

/// `SverigesRadioEpisodeIE._match_valid_url`.
pub fn episode_suitable(url: &str) -> bool {
    EPISODE_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Sveriges Radio {what} are not supported natively"))
}

/// `KeyError(key)` raised while extracting: the URL's ID (`get_temp_id`;
/// none for slug URLs) prefixes the message.
fn key_error(id: Option<&str>, key: &str) -> Error {
    let prefix = id.map(|id| format!("{id}: ")).unwrap_or_default();
    Error::Reported(format!(
        "{prefix}An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_download_json(url, audio_id, note, fatal=False) or {}`: HTTP errors
/// are warnings.
fn optional_json(
    client: &Client,
    url: &Url,
    name: &'static str,
    id: &str,
    note: &str,
) -> Result<Value> {
    match download_json_note(client, url, name, id, note) {
        Ok(value) => Ok(value),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{name}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(Value::Null)
        }
        // Upstream warns with the decoder's message instead.
        Err(Error::Reported(message)) if message.contains(": Failed to parse JSON ") => {
            Err(unsupported("audio URL responses that are not JSON"))
        }
        Err(error) => Err(error),
    }
}

/// `str(value)` of a JSON scalar ID.
fn id_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(number.to_string()),
        _ => None,
    }
}

/// The page's `audio-button` `data-audio-id`/`data-publication-id`, else
/// the `gtm-metadata` JSON's `pageId`; whether the ID was not a string.
fn page_audio_id(
    client: &Client,
    url: &Url,
    name: &'static str,
    display_id: &str,
) -> Result<(String, bool)> {
    let webpage = download_webpage(client, url, name, display_id)?;
    let html_error =
        |error: String| unsupported(&format!("pages whose HTML does not parse ({error})"));
    let button = crate::html_elements::elements_text_and_html_by_class("audio-button", &webpage)
        .map_err(html_error)?
        .into_iter()
        .next()
        .map(|(_, whole)| whole);
    if let Some(button) = button {
        let attributes = crate::html_elements::extract_attributes(&button);
        let found = ["data-audio-id", "data-publication-id"]
            .iter()
            .find_map(|key| attributes.get(*key).and_then(Value::as_str));
        if let Some(found) = found.filter(|found| !found.is_empty()) {
            return Ok((found.to_owned(), false));
        }
    }
    let metadata =
        crate::html_elements::elements_text_and_html_by_attribute("id", "gtm-metadata", &webpage)
            .map_err(html_error)?
            .into_iter()
            .next()
            .map(|(text, _)| text)
            .ok_or_else(|| unsupported("pages without an audio button or GTM metadata"))?;
    let metadata: Value = crate::json::from_str(&metadata)
        .map_err(|_| unsupported("GTM metadata that is not JSON"))?;
    match metadata.get("pageId") {
        None => Err(key_error(None, "pageId")),
        Some(Value::String(text)) => Ok((text.clone(), false)),
        Some(value) => id_text(value)
            .map(|text| (text, true))
            .ok_or_else(|| unsupported("non-scalar page IDs")),
    }
}

/// `SverigesRadioBaseIE._real_extract`.
fn extract(
    client: &Client,
    url: &Url,
    (name, key, audio_type): (&'static str, &'static str, &str),
    pattern: &Regex,
) -> Result<Option<VideoInfo>> {
    let Some(found) = pattern.captures(url.as_str()) else {
        return Ok(None);
    };
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    let temp_id = found.name("id").map(|id| id.as_str());
    let (audio_id, forced_string) = match found.name("id") {
        Some(id) => (id.as_str().to_owned(), false),
        None => page_audio_id(client, url, name, &found["slug"])?,
    };

    let endpoint = |path: &str, pairs: &[(&str, &str)]| -> Result<Url> {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();
        Ok(Url::parse(&format!("{BASE_URL}{path}?{query}"))?)
    };
    let metadata = download_json_note(
        client,
        &endpoint("audiometadata", &[("id", &audio_id), ("type", audio_type)])?,
        name,
        &audio_id,
        "Downloading audio JSON metadata",
    )?;
    let item = metadata
        .get("items")
        .ok_or_else(|| key_error(temp_id, "items"))?
        .get(0)
        .ok_or_else(|| unsupported("metadata without items"))?;

    let mut urls: Vec<String> = Vec::new();
    let mut formats = Vec::new();
    for quality in QUALITIES {
        let data = optional_json(
            client,
            &endpoint(
                "getaudiourl",
                &[
                    ("id", &audio_id),
                    ("type", audio_type),
                    ("format", "iis"),
                    ("quality", quality),
                ],
            )?,
            name,
            &audio_id,
            &format!("Downloading {quality} format JSON metadata"),
        )?;
        let audio_url = match data.get("audioUrl") {
            None | Some(Value::Null) => continue,
            Some(Value::String(text)) if text.is_empty() => continue,
            Some(Value::String(text)) => text.clone(),
            Some(_) => return Err(unsupported("non-string audio URLs")),
        };
        if urls.contains(&audio_url) {
            continue;
        }
        urls.push(audio_url.clone());
        let ext =
            super::direct::determine_ext(&audio_url).unwrap_or_else(|| "unknown_video".into());
        let coding_format = data.get("codingFormat").filter(|value| !value.is_null());
        let abr = AUDIO_BITRATE
            .captures(&audio_url)
            .and_then(|found| found[1].parse::<i64>().ok())
            .filter(|abr| *abr != 0)
            .or_else(|| {
                // `_CODING_FORMAT_TO_ABR_MAP.get(coding_format)` (integer keys).
                match coding_format.and_then(Value::as_i64) {
                    Some(5) => Some(128),
                    Some(11) => Some(192),
                    Some(12) => Some(32),
                    Some(13) => Some(96),
                    _ => None,
                }
            });
        let acodec = match ext.as_str() {
            "mp3" => Value::from("mp3"),
            "m4a" => Value::from("aac"),
            _ => Value::Null,
        };
        let format_id = match coding_format {
            None => Value::Null,
            Some(Value::String(text)) => text.as_str().into(),
            Some(value) => id_text(value)
                .ok_or_else(|| unsupported("non-integer coding formats"))?
                .into(),
        };
        let mut format = dict_format(
            vec![
                ("abr", abr.map_or(Value::Null, Value::from)),
                ("acodec", acodec),
                ("ext", ext.into()),
                ("format_id", format_id),
                ("vcodec", "none".into()),
                ("url", audio_url.into()),
            ],
            name,
            &audio_id,
        )?;
        if abr.is_some() {
            // An int `abr` (and the `tbr` derived from it).
            crate::info_order::mark_int_rates(&mut format);
        }
        formats.push(format);
    }

    // `traverse_obj(item, {...})`: `None` values are left out.
    let present = |value: Option<&Value>| value.filter(|value| !value.is_null()).cloned();
    let mut fields: Vec<(&str, Value)> = vec![("id", audio_id.into()), ("formats", Value::Null)];
    for (field, value) in [
        ("title", present(item.get("subtitle"))),
        ("series", present(item.get("title"))),
        (
            "duration",
            Some(super::streamable::int_or_none(item.get("duration"))).filter(|v| !v.is_null()),
        ),
        (
            "thumbnail",
            item.get("displayimageurl")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(Value::from),
        ),
        ("description", present(item.get("description"))),
    ] {
        if let Some(value) = value {
            fields.push((field, value));
        }
    }
    let info = dict_info(url, (name, key), fields, formats);
    if forced_string {
        crate::screen::warning(
            "\"id\" field is not a string - forcing string conversion, there is an error in extractor",
        );
    }
    Ok(Some(info))
}

impl Extractor for SverigesRadioPublication {
    fn key(&self) -> &'static str {
        "SverigesRadioPublication"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        extract(
            client,
            url,
            (
                "sverigesradio:publication",
                "SverigesRadioPublication",
                "publication",
            ),
            &PUBLICATION_URL,
        )
    }
}

impl Extractor for SverigesRadioEpisode {
    fn key(&self) -> &'static str {
        "SverigesRadioEpisode"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        extract(
            client,
            url,
            ("sverigesradio:episode", "SverigesRadioEpisode", "episode"),
            &EPISODE_URL,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert!(publication_suitable(
            "https://sverigesradio.se/sida/artikel.aspx?programid=83&artikel=7038546"
        ));
        assert!(publication_suitable(
            "https://sverigesradio.se/artikel/tysk-fotbollsfeber-bayern-munchens"
        ));
        assert!(publication_suitable(
            "https://sverigesradio.se/sida/gruppsida.aspx?programid=3304&grupp=6247&artikel=7146887"
        ));
        assert!(episode_suitable(
            "https://sverigesradio.se/avsnitt/1140922?programid=1300"
        ));
        assert!(episode_suitable(
            "https://sverigesradio.se/avsnitt/p4-live-med-first-aid-kit-scandinavium-mars-2023"
        ));
        assert!(!episode_suitable("https://sverigesradio.se/avsnitt/a/b"));
        assert!(!publication_suitable(
            "https://sverigesradio.se/avsnitt/1140922"
        ));
    }
}
