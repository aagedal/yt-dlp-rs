//! Upstream `DLFIE` and `DLFCorpusIE` (`yt_dlp/extractor/dlf.py`): a
//! deutschlandfunk.de audio page's "Anhören" button attributes (its audio URL,
//! title, duration, image and series), and the multi-feed archive pages whose
//! such buttons form a playlist. Upstream's negative lookahead in the corpus
//! pattern is applied to the matched ID instead.
use super::json_ld::url_or_none;
use super::{Extractor, dict_format, dict_info, download_webpage, extract_m3u8_master_optional};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Dlf;
pub struct DlfCorpus;

const NAME: &str = "dlf";
const KEY: &str = "DLF";
const CORPUS_NAME: &str = "dlf:corpus";
const CORPUS_KEY: &str = "DLFCorpus";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?deutschlandfunk\.de/[\w-]+-dlf-(?P<id>[\da-f]{8})-100\.html")
        .unwrap()
});
static CORPUS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?deutschlandfunk\.de/(?P<id>[\w-]+-\d+)\.html").unwrap()
});
/// The `(?![\w-]+-dlf-[\da-f]{8})` lookahead of the corpus pattern.
static DLF_ID_MARK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\w-]+-dlf-[\da-f]{8}").unwrap());
static BUTTON_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(<button[^>]+alt="Anhören"[^>]+data-audio-diraid[^>]*>)"#).unwrap()
});

/// `_match_id` for the audio pattern.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id` for the corpus pattern (a `-dlf-` audio ID is not a corpus).
pub fn corpus_suitable(url: &str) -> bool {
    corpus_id(url).is_some()
}

fn corpus_id(url: &str) -> Option<String> {
    let found = CORPUS_URL.captures(url)?;
    let id = found["id"].to_owned();
    (!DLF_ID_MARK.is_match(&id)).then_some(id)
}

/// `_parse_button_attrs`: one audio entry from a button tag.
fn parse_button(
    client: &Client,
    button: &str,
    audio_id: Option<&str>,
    page: &Url,
    name: &'static str,
) -> Result<VideoInfo> {
    let attrs = crate::html_elements::extract_attributes(button);
    let attr = |key: &str| attrs.get(key).and_then(Value::as_str).map(str::to_owned);
    // The button regex requires `data-audio-diraid`.
    let audio_id = match audio_id {
        Some(id) => id.to_owned(),
        None => attr("data-audio-diraid").unwrap_or_default(),
    };
    let url = [
        "data-audio-download-src",
        "data-audio",
        "data-audioreference",
        "data-audio-src",
    ]
    .into_iter()
    .find_map(|key| attr(key).as_deref().and_then(url_or_none));
    let first = |keys: &[&str]| keys.iter().find_map(|key| attr(key));

    let mut fields: Vec<(&str, Value)> = vec![
        ("id", audio_id.clone().into()),
        ("extractor_key", KEY.into()),
        ("extractor", "dlf".into()),
    ];
    if let Some(title) = first(&[
        "data-audiotitle",
        "data-audio-title",
        "data-audio-download-tracking-title",
    ]) {
        fields.push(("title", title.into()));
    }
    if let Some(duration) = first(&["data-audioduration", "data-audio-duration"])
        .and_then(|text| text.trim().parse::<i64>().ok())
    {
        fields.push(("duration", duration.into()));
    }
    if let Some(thumbnail) = attr("data-audioimage").as_deref().and_then(url_or_none) {
        fields.push(("thumbnail", thumbnail.into()));
    }
    for (name, key) in [
        ("uploader", "data-audio-producer"),
        ("series", "data-audio-series"),
        ("channel", "data-audio-origin-site-name"),
    ] {
        if let Some(value) = attr(key) {
            fields.push((name, value.into()));
        }
    }
    if let Some(webpage_url) = attr("data-audio-download-tracking-path")
        .as_deref()
        .and_then(url_or_none)
    {
        fields.push(("webpage_url", webpage_url.into()));
    }

    let mut formats = Vec::new();
    if let Some(url) = url {
        let ext = super::direct::determine_ext(&url).unwrap_or_else(|| "unknown_video".into());
        if ext == "m3u8" {
            if let Some(stream) = extract_m3u8_master_optional(
                client,
                &Url::parse(&url)?,
                name,
                &audio_id,
                None,
                false,
                false,
            ) {
                formats = stream.formats;
            }
        } else {
            formats.push(dict_format(
                vec![
                    ("url", url.into()),
                    ("ext", ext.into()),
                    ("vcodec", "none".into()),
                ],
                name,
                &audio_id,
            )?);
        }
    }
    fields.push(("formats", Value::Null));
    Ok(dict_info(page, (NAME, KEY), fields, formats))
}

/// `_search_regex(_BUTTON_REGEX, webpage, 'button')` (fatal).
fn search_error(id: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract button; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for Dlf {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let audio_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &audio_id)?;
        let Some(button) = BUTTON_REGEX
            .captures(&webpage)
            .map(|found| found[1].to_owned())
        else {
            return Err(search_error(&audio_id));
        };
        parse_button(client, &button, Some(&audio_id), url, NAME).map(Some)
    }
}

impl Extractor for DlfCorpus {
    fn key(&self) -> &'static str {
        CORPUS_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(playlist_id) = corpus_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(CORPUS_NAME);
        crate::screen::screen(&format!(
            "[{CORPUS_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, CORPUS_NAME, &playlist_id)?;
        let meta = |names: &[&str]| {
            names
                .iter()
                .find_map(|name| crate::html::html_search_meta(&webpage, name))
        };
        let title = meta(&["og:title", "twitter:title"]);
        let description = meta(&["description", "og:description", "twitter:description"]);
        let entries = BUTTON_REGEX
            .captures_iter(&webpage)
            .map(|found| parse_button(client, &found[1], None, url, CORPUS_NAME))
            .collect::<Result<Vec<_>>>()?;

        // `{'_type': 'playlist', 'id', 'description', 'title', 'entries'}`:
        // a missing title stays a `null` key.
        let mut extra = Map::new();
        if title.is_none() {
            extra.insert("title".into(), Value::Null);
        }
        extra.insert(
            "description".into(),
            description.map_or(Value::Null, Value::from),
        );
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;_type,id,description,title{}",
                crate::info_order::GENERATOR
            )
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: CORPUS_NAME.into(),
            extractor_key: CORPUS_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.deutschlandfunk.de/tanz-der-saiteninstrumente-das-wild-strings-trio-aus-slowenien-dlf-03a3eb19-100.html"
        ));
        assert!(!suitable(
            "https://www.deutschlandfunk.de/kommentare-und-themen-der-woche-100.html"
        ));
        assert!(corpus_suitable(
            "https://www.deutschlandfunk.de/kommentare-und-themen-der-woche-100.html"
        ));
        assert!(corpus_suitable(
            "https://www.deutschlandfunk.de/fechten-russland-belarus-ukraine-protest-100.html"
        ));
        // The lookahead: an audio page is not a corpus.
        assert!(!corpus_suitable(
            "https://www.deutschlandfunk.de/tanz-der-saiteninstrumente-das-wild-strings-trio-aus-slowenien-dlf-03a3eb19-100.html"
        ));
    }
}
