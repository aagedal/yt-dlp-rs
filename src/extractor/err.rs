//! Upstream `ERRJupiterIE` and `ERRArhiivIE` (`yt_dlp/extractor/err.py`):
//! ERR's Jupiter, Jupiter+ and Lasteekraan content through the
//! `vodContent/getContentPageData` API (the first media's HLS, DASH and file
//! sources, the DRM error, metadata and episode fields) and the ERR archive's
//! `content/video` API.
//!
//! Upstream iterates a media's distinct HLS and DASH URLs as Python sets
//! (hash order); this port keeps their first-seen order.
use super::{
    Extractor, dict_format, dict_info, download_json, extract_m3u8_master_optional,
    extract_mpd_optional_note,
};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct ErrJupiter;
pub struct ErrArhiiv;

const JUPITER_NAME: &str = "ERRJupiter";
const ARHIIV_NAME: &str = "ERRArhiiv";

static JUPITER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:jupiter(?:pluss)?|lasteekraan)\.err\.ee/(?P<id>\d+)").unwrap()
});
static ARHIIV_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https://arhiiv\.err\.ee/video/(?:vaata/)?(?P<id>[^/?#]+)").unwrap()
});

/// `ERRJupiterIE._match_id`.
pub fn jupiter_suitable(url: &str) -> bool {
    JUPITER_URL.is_match(url)
}

/// `ERRArhiivIE._match_id`.
pub fn arhiiv_suitable(url: &str) -> bool {
    ARHIIV_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

fn int(value: Option<&Value>) -> Option<Value> {
    Some(super::streamable::int_or_none(value)).filter(|value| !value.is_null())
}

/// `str_or_none`: strings and numbers.
fn str_or_none(value: Option<&Value>) -> Option<Value> {
    match value {
        Some(Value::String(text)) => Some(text.clone().into()),
        Some(Value::Number(number)) => Some(number.to_string().into()),
        Some(Value::Bool(flag)) => Some(if *flag { "True" } else { "False" }.into()),
        _ => None,
    }
}

/// `{str}` (with `filter`: a non-empty string).
fn text(value: Option<&Value>, filter: bool) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .filter(|text| !filter || !text.is_empty())
        .map(Value::from)
}

/// Non-fatal HLS (`hls`) and DASH (`dash`) manifests' formats and subtitles.
fn manifests(
    client: &Client,
    name: &'static str,
    id: &str,
    hls: &[String],
    dash: &[String],
    formats: &mut Vec<crate::VideoFormat>,
    subtitles: &mut Subtitles,
) -> Result<()> {
    for url in hls {
        if let Some(stream) = extract_m3u8_master_optional(
            client,
            &Url::parse(url)?,
            name,
            id,
            Some("hls"),
            true,
            false,
        ) {
            formats.extend(stream.formats);
            super::svt::merge_subtitles(subtitles, stream.subtitles);
        }
    }
    for url in dash {
        if let Some(stream) = extract_mpd_optional_note(
            client,
            &Url::parse(url)?,
            name,
            id,
            Some("dash"),
            "Downloading MPD manifest",
        ) {
            formats.extend(stream.formats);
            super::svt::merge_subtitles(subtitles, stream.subtitles);
        }
    }
    Ok(())
}

/// `set(traverse_obj(src, (KEYS, {url_or_none})))` in first-seen order.
fn distinct_urls(src: Option<&Value>, keys: &[&str]) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for key in keys {
        if let Some(url) = url_or_none(src.and_then(|src| src.get(*key)))
            && !urls.contains(&url)
        {
            urls.push(url);
        }
    }
    urls
}

impl Extractor for ErrJupiter {
    fn key(&self) -> &'static str {
        JUPITER_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = JUPITER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(JUPITER_NAME, url);
        let mut api = Url::parse("https://services.err.ee/api/v2/vodContent/getContentPageData")?;
        api.query_pairs_mut().append_pair("contentId", &id);
        let response = download_json(client, &api, JUPITER_NAME, &id)?;
        let data = response
            .get("data")
            .ok_or_else(|| key_error(&id, "data"))?
            .get("mainContent")
            .ok_or_else(|| key_error(&id, "mainContent"))?;

        // `traverse_obj(data, ('medias', ..., {dict}), get_all=False)`.
        let media = match data.get("medias") {
            Some(Value::Array(medias)) => medias.iter().find(|media| media.is_object()),
            Some(Value::Object(medias)) => medias.values().find(|media| media.is_object()),
            _ => None,
        };
        let src = media.and_then(|media| media.get("src"));
        if media
            .and_then(|media| media.get("restrictions"))
            .and_then(|restrictions| restrictions.get("drm"))
            == Some(&Value::Bool(true))
        {
            // `report_drm(video_id)`.
            crate::no_formats::raise_expected(
                JUPITER_NAME,
                Some(&id),
                None,
                "This video is DRM protected",
            )?;
        }
        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        manifests(
            client,
            JUPITER_NAME,
            &id,
            &distinct_urls(src, &["hls", "hls2", "hlsNew"]),
            &distinct_urls(src, &["dash", "dashNew"]),
            &mut formats,
            &mut subtitles,
        )?;
        if let Some(file) = url_or_none(src.and_then(|src| src.get("file"))) {
            formats.push(dict_format(
                vec![("url", file.into()), ("format_id", "http".into())],
                JUPITER_NAME,
                &id,
            )?);
        }

        let description = ["lead", "body"]
            .iter()
            .filter_map(|key| data.get(*key).and_then(Value::as_str))
            .map(crate::html::clean_html)
            .find(|text| !text.is_empty())
            .map(Value::from);
        let release = ["scheduleStart", "publicStart"]
            .iter()
            .find_map(|key| int(data.get(*key)));
        let mut fields = vec![
            ("id", Value::from(id.as_str())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        let add = |fields: &mut Vec<(&str, Value)>, items: Vec<(&'static str, Option<Value>)>| {
            fields.extend(
                items
                    .into_iter()
                    .filter_map(|(key, value)| value.map(|value| (key, value))),
            );
        };
        add(
            &mut fields,
            vec![
                ("title", text(data.get("heading"), false)),
                ("alt_title", text(data.get("subHeading"), false)),
                ("description", description),
                ("timestamp", int(data.get("created"))),
                ("modified_timestamp", int(data.get("updated"))),
                ("release_timestamp", release),
                ("release_year", int(data.get("year"))),
            ],
        );
        if data.get("type").and_then(Value::as_str) == Some("episode") {
            add(
                &mut fields,
                vec![
                    ("series", text(data.get("heading"), false)),
                    ("series_id", str_or_none(data.get("rootContentId"))),
                    ("episode", text(data.get("subHeading"), false)),
                    ("season_number", int(data.get("season"))),
                    ("episode_number", int(data.get("episode"))),
                    ("episode_id", str_or_none(data.get("id"))),
                ],
            );
        }
        let mut info = dict_info(url, (JUPITER_NAME, JUPITER_NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

impl Extractor for ErrArhiiv {
    fn key(&self) -> &'static str {
        ARHIIV_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = ARHIIV_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(ARHIIV_NAME, url);
        let api = Url::parse(&format!("https://arhiiv.err.ee/api/v1/content/video/{id}"))?;
        let data = download_json(client, &api, ARHIIV_NAME, &id)?;
        let src = data.get("media").and_then(|media| media.get("src"));
        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        manifests(
            client,
            ARHIIV_NAME,
            &id,
            &distinct_urls(src, &["hls"]),
            &distinct_urls(src, &["dash"]),
            &mut formats,
            &mut subtitles,
        )?;
        let info = data.get("info");
        let get = |key: &str| info.and_then(|info| info.get(key));
        let date = |key: &str| {
            get(key)
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .map(Value::from)
        };
        let mut fields = vec![
            ("id", Value::from(id.as_str())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        let items: [(&str, Option<Value>); 9] = [
            ("title", text(get("title"), false)),
            ("series", text(get("seriesTitle"), true)),
            ("series_id", text(get("seriesId"), true)),
            ("episode_id", text(get("episode"), true)),
            ("description", text(get("synopsis"), true)),
            ("timestamp", date("uploadDate")),
            ("modified_timestamp", date("dateModified")),
            ("release_timestamp", date("date")),
            ("release_year", int(get("year"))),
        ];
        fields.extend(
            items
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        );
        let mut info = dict_info(url, (ARHIIV_NAME, ARHIIV_NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(jupiter_suitable(
            "https://jupiter.err.ee/1211107/siin-me-oleme"
        ));
        assert!(jupiter_suitable(
            "https://jupiterpluss.err.ee/1609180445/bolee-zelenyj-tallinn"
        ));
        assert!(jupiter_suitable("https://lasteekraan.err.ee/1092243/patu"));
        assert!(arhiiv_suitable(
            "https://arhiiv.err.ee/video/vaata/koalitsioonileppe-allkirjastamine"
        ));
        assert!(!arhiiv_suitable("http://arhiiv.err.ee/video/x"));
    }
}
