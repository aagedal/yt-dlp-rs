//! Upstream `S4CIE` and `S4CSeriesIE` (`yt_dlp/extractor/s4c.py`): S4C
//! Clic programmes from the player configuration (subtitles, poster) and
//! streaming-URL APIs (fatal HLS formats) with the non-fatal programme
//! details, and series as playlists of their programmes' `url` results.
use super::{Extractor, dict_info, download_json, download_json_note, extract_m3u8_fatal_master};
use crate::{Error, ExtractResult, PlaylistInfo, Result, SubtitleFormat, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct S4c;
pub struct S4cSeries;

const NAME: &str = "S4C";
const SERIES_NAME: &str = "S4CSeries";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?s4c\.cymru/clic/programme/(?P<id>\d+)").unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?s4c\.cymru/clic/series/(?P<id>\d+)").unwrap()
});

/// `S4CIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `S4CSeriesIE._match_id`.
pub fn series_suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("S4C {what} are not supported natively"))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// A `KeyError(key)` raised while extracting.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `str_or_none(value)`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.into()),
        _ => None,
    }
}

/// `int(x) * 60` (`None` when `int` raises).
fn duration_minutes(value: Option<&Value>) -> Option<i64> {
    let minutes = match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))?,
        Value::String(text) => text.trim().replace('_', "").parse().ok()?,
        Value::Bool(value) => i64::from(*value),
        _ => return None,
    };
    minutes.checked_mul(60)
}

/// `_download_json(url, video_id, fatal=False)`: HTTP errors are warnings.
fn optional_json(client: &Client, url: &Url, id: &str) -> Result<Option<Value>> {
    match download_json(client, url, NAME, id) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(Error::Reported(message)) if message.contains(": Failed to parse JSON ") => {
            Err(unsupported("programme details that are not JSON"))
        }
        Err(error) => Err(error),
    }
}

/// `url_result(url, S4CIE, episode_id)`, resolved when processed.
fn url_entry(target: &str, episode_id: &Value) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), NAME.into());
    entry.insert("id".into(), episode_id.clone());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for S4c {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(NAME, url);
        let details_url = Url::parse(&format!(
            "https://www.s4c.cymru/df/full_prog_details?lang=e&programme_id={video_id}"
        ))?;
        let details = optional_json(client, &details_url, &video_id)?;

        let mut config_url =
            Url::parse("https://player-api.s4c-cdn.co.uk/player-configuration/prod")?;
        config_url
            .query_pairs_mut()
            .append_pair("programme_id", &video_id)
            .append_pair("signed", "0")
            .append_pair("lang", "en")
            .append_pair("mode", "od")
            .append_pair("appId", "clic")
            .append_pair("streamName", "");
        let player_config = download_json_note(
            client,
            &config_url,
            NAME,
            &video_id,
            "Downloading player config JSON",
        )?;
        let Value::Object(player_config) = player_config else {
            return Err(unsupported("player configurations that are not objects"));
        };
        let mut subtitles = Subtitles::default();
        let subs: Vec<&Value> = match player_config.get("subtitles") {
            Some(Value::Array(items)) => items.iter().collect(),
            Some(Value::Object(items)) => items.values().collect(),
            _ => Vec::new(),
        };
        for sub in subs {
            let Some(sub_url) = sub
                .get("0")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let language = match sub.get("3") {
                None => Some("en".to_owned()),
                Some(Value::String(language)) => Some(language.clone()),
                Some(_) => return Err(unsupported("subtitle languages that are not strings")),
            };
            let name = match sub.get("1") {
                Some(Value::String(name)) => Some(name.clone()),
                // Upstream keeps a `None` name in the track dict.
                _ => return Err(unsupported("subtitles without names")),
            };
            let track = SubtitleFormat {
                name,
                url: Url::parse(&sub_url)?,
                ext: super::direct::determine_ext(&sub_url)
                    .unwrap_or_else(|| "unknown_video".into()),
                protocol: None,
                fragments: Vec::new(),
                extra: Default::default(),
            };
            match subtitles
                .entries
                .iter_mut()
                .find(|(key, _)| *key == language)
            {
                Some((_, tracks)) => tracks.push(track),
                None => subtitles.entries.push((language, vec![track])),
            }
        }
        let region = if player_config.get("application").and_then(Value::as_str) == Some("s4chttpl")
        {
            "UK"
        } else {
            "WW"
        };
        let filename = match player_config.get("filename") {
            None => return Err(key_error(&video_id, "filename")),
            Some(Value::String(filename)) => filename.clone(),
            Some(Value::Number(number)) => number.to_string(),
            Some(_) => return Err(unsupported("file names that are not strings")),
        };
        let mut streaming_url = Url::parse("https://player-api.s4c-cdn.co.uk/streaming-urls/prod")?;
        streaming_url
            .query_pairs_mut()
            .append_pair("mode", "od")
            .append_pair("application", "clic")
            .append_pair("region", region)
            .append_pair("extra", "false")
            .append_pair("thirdParty", "false")
            .append_pair("filename", &filename);
        let streaming = download_json_note(
            client,
            &streaming_url,
            NAME,
            &video_id,
            "Downloading streaming urls JSON",
        )?;
        let m3u8_url = match streaming.get("hls") {
            None => return Err(key_error(&video_id, "hls")),
            Some(Value::String(m3u8_url)) => m3u8_url.clone(),
            Some(_) => return Err(unsupported("HLS URLs that are not strings")),
        };
        let stream = extract_m3u8_fatal_master(
            client,
            &Url::parse(&m3u8_url).map_err(|_| unsupported("relative HLS URLs"))?,
            NAME,
            &video_id,
            Some("hls"),
            false,
        )?;

        let thumbnail = player_config
            .get("poster")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map_or(Value::Null, Value::from);
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", Value::from(video_id.clone())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("thumbnail", thumbnail),
        ];
        // `**traverse_obj(details, ('full_prog_details', 0, {...}),
        // get_all=False)`: only found keys.
        if let Some(details) = details
            .as_ref()
            .and_then(|details| details.pointer("/full_prog_details/0"))
        {
            let text = |key: &str| details.get(key).and_then(Value::as_str);
            let date = |key: &str| {
                text(key)
                    .and_then(|date| crate::dates::unified_strdate(date, true))
                    .map(Value::from)
            };
            let found = [
                (
                    "title",
                    text("programme_title")
                        .or_else(|| text("series_title"))
                        .map(Value::from),
                ),
                (
                    "description",
                    text("full_billing").map(|text| text.trim().into()),
                ),
                (
                    "duration",
                    duration_minutes(details.get("duration")).map(Value::from),
                ),
                ("upload_date", date("clic_aired")),
                ("release_date", date("last_tx")),
                (
                    "series",
                    text("series_title")
                        .filter(|series| !series.is_empty())
                        .map(Value::from),
                ),
                (
                    "series_id",
                    str_or_none(details.get("series_id"))
                        .filter(|id| !id.is_empty())
                        .map(Value::from),
                ),
            ];
            fields.extend(
                found
                    .into_iter()
                    .filter_map(|(key, value)| Some((key, value?))),
            );
        }
        let mut info = dict_info(url, (NAME, NAME), fields, stream.formats);
        // Track dicts are `{url, name}`, then processing's `ext`.
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";subtitle=url,name");
        }
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

impl Extractor for S4cSeries {
    fn key(&self) -> &'static str {
        SERIES_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let series_id = found["id"].to_owned();
        announce(SERIES_NAME, url);
        let mut api = Url::parse("https://www.s4c.cymru/df/series_details")?;
        api.query_pairs_mut()
            .append_pair("lang", "e")
            .append_pair("series_id", &series_id)
            .append_pair("show_prog_in_series", "Y");
        let series_details = download_json_note(
            client,
            &api,
            SERIES_NAME,
            &series_id,
            "Downloading series details JSON",
        )?;
        let programmes: Vec<&Value> = match series_details.get("other_progs_in_series") {
            Some(Value::Array(items)) => items.iter().collect(),
            Some(Value::Object(items)) => items.values().collect(),
            _ => Vec::new(),
        };
        let mut entries = Vec::new();
        for programme in programmes {
            let Some(episode_id) = programme.get("id") else {
                continue;
            };
            let id_text = match episode_id {
                Value::Null => continue,
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                _ => return Err(unsupported("programme IDs that are not strings")),
            };
            entries.push(url_entry(
                &format!("https://www.s4c.cymru/clic/programme/{id_text}"),
                episode_id,
            )?);
        }
        let title = series_details
            .pointer("/full_prog_details/0/series_title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .map(str::to_owned);
        let keys = if title.is_some() { "id,title" } else { "id" };
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: series_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: SERIES_NAME.into(),
            extractor_key: SERIES_NAME.into(),
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
    fn urls_follow_upstream_valid_urls() {
        assert!(suitable("https://www.s4c.cymru/clic/programme/893068169"));
        assert!(!suitable("https://www.s4c.cymru/clic/series/864982911"));
        assert!(series_suitable(
            "https://www.s4c.cymru/clic/series/864982911"
        ));
    }

    #[test]
    fn durations_are_minutes() {
        assert_eq!(duration_minutes(Some(&json!("15"))), Some(900));
        assert_eq!(duration_minutes(Some(&json!(23))), Some(1380));
        assert_eq!(duration_minutes(Some(&json!("x"))), None);
    }
}
