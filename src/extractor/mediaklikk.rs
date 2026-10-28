//! Upstream `MediaKlikkIE` (`yt_dlp/extractor/mediaklikk.py`): MediaKlikk,
//! M4 Sport and Híradó videos from the page's `loadPlayer(...)` data, sent
//! as the query of the `playernew/player.php` page, whose `pl.setup(...)`
//! playlist names the HLS master playlist.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct MediaKlikk;

const NAME: &str = "MediaKlikk";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:mediaklikk|m4sport|hirado)\.hu/.*?(?:videok?|cikk)/(?:(?P<year>[0-9]{4})/(?P<month>[0-9]{1,2})/(?P<day>[0-9]{1,2})/)?(?P<id>[^/#?_]+)",
    )
    .unwrap()
});
static LOAD_PLAYER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"loadPlayer\((?:\s*["'][^"']+["']\s*,)?"#).unwrap());
static ARTICLE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<h\d+\b[^>]+\bclass="article_title">([^<]+)<"#).unwrap());
static ARTICLE_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<p+\b[^>]+\bclass="article_date">([^<]+)<"#).unwrap());
static SETUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bpl\.setup\s*\(").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn regex_error(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MediaKlikk {what} are not supported natively"))
}

/// Python `str(value)` of a JSON scalar (as `urlencode` sends it).
fn py_str(value: &Value) -> Result<String> {
    Ok(match value {
        Value::String(text) => text.clone(),
        Value::Number(number) if number.is_f64() => {
            let float = number.as_f64().unwrap_or_default();
            if float.fract() == 0.0 && float.abs() < 1e16 {
                format!("{float:.1}")
            } else {
                float.to_string()
            }
        }
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => if *value { "True" } else { "False" }.to_owned(),
        Value::Null => "None".to_owned(),
        _ => return Err(unsupported("player data values that are not scalars")),
    })
}

/// `_search_json(r'\bpl\.setup\s*\(', page, ..., end_pattern=r'\);')`: the
/// greedy `{...}` up to the last `}` followed by `);`.
fn setup_json(page: &str) -> Option<Option<Value>> {
    let start = SETUP.find(page)?.end();
    let rest = page[start..].trim_start();
    if !rest.starts_with('{') {
        return None;
    }
    let mut end = None;
    for (index, _) in rest.match_indices('}') {
        if rest[index + 1..].trim_start().starts_with(");") {
            end = Some(index);
        }
    }
    let end = end?;
    Some(
        serde_json::Deserializer::from_str(&rest[..=end])
            .into_iter::<Value>()
            .next()
            .and_then(std::result::Result::ok),
    )
}

impl Extractor for MediaKlikk {
    fn key(&self) -> &'static str {
        NAME
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
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let mut player_data: Map<String, Value> =
            match super::search_json(&LOAD_PLAYER, &webpage, false) {
                Some(Some(Value::Object(data))) => data,
                Some(_) => return Err(unsupported("player data that does not parse")),
                None => return Err(regex_error(&display_id, "player data")),
            };
        let video_id = match player_data.get("contentId") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            Some(_) => return Err(unsupported("content IDs that are not strings")),
            None => return Err(key_error(&display_id, "contentId")),
        };
        let title = match player_data
            .get("title")
            .filter(|title| super::nuxt::truthy(title))
        {
            Some(title) => title.clone(),
            None => {
                let og = crate::html::og_search_property(&webpage, "title");
                if og.is_none() {
                    crate::screen::warning(&format!(
                        "[{NAME}] unable to extract OpenGraph title; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ));
                }
                match og.filter(|title| !title.is_empty()) {
                    Some(title) => title.into(),
                    None => ARTICLE_TITLE
                        .captures(&webpage)
                        .map(|found| Value::from(crate::html::clean_html(&found[1])))
                        .ok_or_else(|| regex_error(&display_id, "title"))?,
                }
            }
        };
        let date = |name: &str| found.name(name).map_or("None", |value| value.as_str());
        let upload_date = crate::dates::unified_strdate(
            &format!("{}-{}-{}", date("year"), date("month"), date("day")),
            true,
        )
        .or_else(|| {
            ARTICLE_DATE
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]))
                .and_then(|date| crate::dates::unified_strdate(&date, true))
        });
        // `player_data['video'] = unquote(player_data.pop('token'))`.
        let token = player_data
            .shift_remove("token")
            .ok_or_else(|| key_error(&display_id, "token"))?;
        let token = token
            .as_str()
            .ok_or_else(|| unsupported("tokens that are not strings"))?;
        let video = percent_encoding::percent_decode_str(token)
            .decode_utf8_lossy()
            .into_owned();
        player_data.insert("video".into(), video.into());
        let mut player_url = Url::parse("https://player.mediaklikk.hu/playernew/player.php")?;
        {
            let mut query = player_url.query_pairs_mut();
            for (key, value) in &player_data {
                query.append_pair(key, &py_str(value)?);
            }
        }
        let player_page = super::download_webpage_note_headers(
            client,
            &player_url,
            NAME,
            &video_id,
            "Downloading webpage",
            &[("Referer", url.as_str())],
        )?;
        let player_json = match setup_json(&player_page) {
            Some(Some(json)) => json,
            Some(None) => return Err(unsupported("player setups that do not parse")),
            None => return Err(regex_error(&video_id, "player json")),
        };
        let playlist_url = player_json
            .get("playlist")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("hls"))
            .find_map(|item| {
                item.get("file")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
            })
            .ok_or_else(|| {
                // Raised without a video ID: `get_temp_id`'s slug.
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract playlist url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let stream = extract_m3u8_fatal_master(
            client,
            &Url::parse(&playlist_url)?,
            NAME,
            &video_id,
            None,
            true,
        )?;
        let thumbnail = match player_data
            .get("bgImage")
            .filter(|value| super::nuxt::truthy(value))
        {
            Some(image) => image.clone(),
            None => {
                let og = crate::html::og_search_property(&webpage, "image");
                if og.is_none() {
                    crate::screen::warning(&format!(
                        "[{NAME}] unable to extract OpenGraph thumbnail; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ));
                }
                og.map_or(Value::Null, Value::from)
            }
        };
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("display_id", display_id.into()),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("upload_date", upload_date.map_or(Value::Null, Value::from)),
                ("thumbnail", thumbnail),
            ],
            stream.formats,
        );
        info.subtitles = stream.subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://mediaklikk.hu/ajanlo/video/2025/08/04/heviz-dzsungel-a-viz-alatt-ajanlo-08-10/"
        ));
        assert!(suitable("https://m4sport.hu/video/2025/08/07/foci-nb1"));
        assert!(suitable("https://hirado.hu/cikk/2025/08/09/x"));
        assert!(!suitable("https://mediaklikk.hu/musor/"));
    }

    #[test]
    fn setup_json_ends_at_the_last_call_close() {
        let page = r#"pl.setup( {"playlist": [{"file": "a"}], "f": "x});"} );"#;
        assert_eq!(
            setup_json(page),
            Some(Some(
                serde_json::json!({"playlist": [{"file": "a"}], "f": "x});"})
            ))
        );
    }
}
