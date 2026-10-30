//! Upstream `OnsenIE` (`yt_dlp/extractor/onsen.py`): a program's
//! `web_api/programs` JSON as an HLS episode (`?c=` base64 IDs) or as a
//! playlist of its episodes.
use super::{
    Extractor, dict_info, download_json_headers, extract_m3u8_master_optional_headers,
    update_url_query,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Onsen;

const NAME: &str = "onsen";
const KEY: &str = "Onsen";
const BASE_URL: &str = "https://www.onsen.ag";
const HEADERS: [(&str, &str); 1] = [("Referer", "https://www.onsen.ag/")];
const API_HEADERS: [(&str, &str); 1] = [("X-Client", "onsen-web")];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:share|www)\.)onsen\.ag/program/(?P<id>[^/?#]+)").unwrap()
});

/// `_get_encoded_id`: the urlsafe base64 of the program's ID.
fn encoded_id(id: &Value) -> String {
    let text = match id {
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    base64::engine::general_purpose::URL_SAFE.encode(text)
}

/// `base64.urlsafe_b64decode(f'{c}===').decode()`: padding is optional.
fn decode_c(c: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(c.trim_end_matches('='))
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// `parse_qs(url)` keeping each key's last non-blank value.
fn last_query(url: &Url) -> Map<String, Value> {
    let mut query = Map::new();
    for (key, value) in url.query_pairs() {
        if !value.is_empty() {
            query.insert(key.into_owned(), Value::from(value.into_owned()));
        }
    }
    query
}

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

/// `clean_html` of a string value (`None` when absent or not a string).
fn clean(value: Option<&Value>) -> Option<String> {
    text(value).map(|text| crate::html::clean_html(&text))
}

/// `url_or_none` of a string value.
fn url_value(value: Option<&Value>) -> Option<String> {
    text(value).and_then(|text| super::json_ld::url_or_none(&text))
}

/// `url_result(url, OnsenIE, id)` as a playlist entry.
fn episode_entry(target: &str, id: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = id.to_owned();
    let entry = json!({
        "ie_key": KEY,
        "id": id,
        "_type": "url",
        "url": target,
    });
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": entry,
        }),
    );
    Ok(info)
}

impl Extractor for Onsen {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let program_id = found["id"].to_owned();
        crate::screen::set_extractor(KEY);
        crate::screen::screen(&format!(
            "[{KEY}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!("{BASE_URL}/web_api/programs/{program_id}"))?;
        let programs = download_json_headers(client, &api, NAME, &program_id, &API_HEADERS)?;

        let query = last_query(url);
        let contents: Vec<&Value> = programs
            .get("contents")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.get("id").is_some_and(super::nuxt::truthy))
                    .collect()
            })
            .unwrap_or_default();

        let Some(c) = query.get("c").and_then(Value::as_str) else {
            let mut entries = Vec::new();
            for program in &contents {
                let target = update_url_query(url, &[("c", &encoded_id(&program["id"]))]);
                let id = program["id"].to_string();
                let id = id.trim_matches('"').to_owned();
                entries.push(episode_entry(target.as_str(), &id)?);
            }
            // `playlist_result(entries, program_id, title)`.
            let title = clean(
                programs
                    .get("program_info")
                    .and_then(|info| info.get("title")),
            );
            let shape = if title.is_some() {
                "playlist-dict;id,title"
            } else {
                "playlist-dict;id"
            };
            let mut extra = Map::new();
            extra.insert(crate::info_order::INFO_SHAPE.into(), shape.into());
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id: program_id,
                title: title.unwrap_or_default(),
                webpage_url: url.clone(),
                extractor: NAME.into(),
                extractor_key: KEY.into(),
                nested: Vec::new(),
                pending: None,
                entries,
                extra,
            })));
        };

        let raw_id = decode_c(c).ok_or_else(|| {
            Error::UnsupportedHtml5(
                "Onsen `c` values that are not base64 are not supported natively".into(),
            )
        })?;
        let wanted: i64 = raw_id.parse().map_err(|_| {
            Error::UnsupportedHtml5(
                "Onsen IDs that are not integers are not supported natively".into(),
            )
        })?;
        let Some(program) = contents
            .iter()
            .find(|item| item.get("id").and_then(Value::as_i64) == Some(wanted))
            .copied()
        else {
            return Err(Error::Reported(format!(
                "[{NAME}] {raw_id}: This program is no longer available"
            )));
        };
        let Some(m3u8_url) = url_value(program.get("streaming_url")) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {raw_id}: {}",
                super::login::login_required_message(
                    "This program is only available for premium supporters",
                    Some(NAME),
                    None,
                )
            )));
        };
        let manifest = Url::parse(&m3u8_url)?;
        let display_id = encoded_id(&program["id"]);
        let date = Regex::new(&format!(r"{}0?(\d{{6}})", regex::escape(&program_id)))
            .ok()
            .and_then(|pattern| pattern.captures(&m3u8_url).map(|found| found[1].to_owned()));
        let upload_date = date.map_or(Value::Null, |date| Value::from(format!("20{date}")));
        let mut formats =
            extract_m3u8_master_optional_headers(client, &manifest, NAME, &raw_id, &HEADERS)
                .ok_or_else(|| {
                    Error::UnsupportedHtml5(
                        "Onsen HLS formats that fail to download are not supported natively".into(),
                    )
                })?;
        // `_extract_m3u8_formats` without an `ext`: audio-only variants are m4a.
        super::hls::without_ext(&mut formats);

        let section_start = query
            .get("t")
            .and_then(Value::as_str)
            .and_then(|text| text.parse::<i64>().ok())
            .map_or(Value::from(0), Value::from);
        let mut fields: Vec<(&str, Value)> = vec![
            ("display_id", Value::from(display_id.clone())),
            ("formats", Value::Null),
            ("http_headers", json!({"Referer": "https://www.onsen.ag/"})),
            ("section_start", section_start),
            ("upload_date", upload_date),
            (
                "webpage_url",
                Value::from(format!("{BASE_URL}/program/{program_id}?c={display_id}")),
            ),
        ];
        let id = program
            .get("id")
            .and_then(Value::as_i64)
            .map(|id| Value::from(id.to_string()));
        if let Some(id) = id {
            fields.push(("id", id));
        }
        if let Some(title) = clean(program.get("title")) {
            fields.push(("title", Value::from(title)));
        }
        if let Some(media_type) = text(program.get("media_type")) {
            fields.push(("media_type", Value::from(media_type)));
        }
        if let Some(mut thumbnail) = url_value(program.get("poster_image_url"))
            .and_then(|thumbnail| Url::parse(&thumbnail).ok())
        {
            thumbnail.set_query(None);
            fields.push(("thumbnail", Value::from(thumbnail.to_string())));
        }
        let mut cast: Vec<Value> = Vec::new();
        for person in programs
            .get("performers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .chain(
                program
                    .get("guests")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            )
        {
            if let Some(name) = text(person.get("name")).filter(|name| !name.is_empty()) {
                cast.push(Value::from(name));
            }
        }
        if !cast.is_empty() {
            fields.push(("cast", Value::Array(cast)));
        }
        if let Some(series_id) = text(programs.get("directory_name")) {
            fields.push(("series_id", Value::from(series_id)));
        }
        let info_block = programs.get("program_info");
        if let Some(description) = clean(info_block.and_then(|info| info.get("description")))
            .filter(|description| !description.is_empty())
        {
            fields.push(("description", Value::from(description)));
        }
        if let Some(series) = clean(info_block.and_then(|info| info.get("title"))) {
            fields.push(("series", Value::from(series)));
        }
        let tags: Vec<Value> = info_block
            .and_then(|info| info.get("hashtag_list"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|tag| !tag.is_empty())
            .map(Value::from)
            .collect();
        if !tags.is_empty() {
            fields.push(("tags", Value::Array(tags)));
        }

        let mut info = dict_info(url, (NAME, KEY), fields, formats.formats);
        info.subtitles = formats.subtitles;
        Ok(Some(ExtractResult::Video(info)))
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            Some(_) => Ok(None),
            None => Ok(None),
        }
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suitable(url: &str) -> bool {
        VALID_URL.is_match(url)
    }

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://share.onsen.ag/program/onsenking?p=90&c=MTA0NjI"
        ));
        assert!(suitable("https://www.onsen.ag/program/uma"));
        assert!(!suitable("https://www.onsen.ag/"));
    }

    #[test]
    fn episode_ids_round_trip_through_base64() {
        assert_eq!(encoded_id(&json!(10462)), "MTA0NjI=");
        assert_eq!(decode_c("MTA0NjI").as_deref(), Some("10462"));
    }
}
