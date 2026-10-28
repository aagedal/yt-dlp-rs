//! Upstream `CNBCVideoIE` (`yt_dlp/extractor/cnbc.py`): CNBC video pages'
//! `window.__s_data` clip player module (its Akamai stream as HDS and HLS
//! formats and its metadata) overlaid on the page's non-fatal JSON-LD.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct CnbcVideo;

const NAME: &str = "CNBCVideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?cnbc\.com/video/(?:[^/?#]+/)+(?P<id>[^./?#&]+)\.html").unwrap()
});
static S_DATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"window\.__s_data=").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("CNBC {what} are not supported natively"))
}

/// `traverse_obj(data, ('page', 'page', 'layout', ..., 'columns', ...,
/// 'modules', lambda _, v: v['name'] == 'clipPlayer', 'data', {dict}),
/// get_all=False)`.
fn player_data(data: &Value) -> Option<&Value> {
    fn each(value: &Value) -> Vec<&Value> {
        match value {
            Value::Array(items) => items.iter().collect(),
            Value::Object(items) => items.values().collect(),
            _ => Vec::new(),
        }
    }
    let layout = data.pointer("/page/page/layout")?;
    for row in each(layout) {
        for column in row.get("columns").map(each).unwrap_or_default() {
            for module in column.get("modules").map(each).unwrap_or_default() {
                if module.get("name").and_then(Value::as_str) == Some("clipPlayer")
                    && let Some(data) = module.get("data").filter(|data| data.is_object())
                {
                    return Some(data);
                }
            }
        }
    }
    None
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

impl Extractor for CnbcVideo {
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
        let data = match super::search_json(&S_DATA, &webpage, false) {
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract video data; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            Some(None) => return Err(unsupported("video data that do not parse")),
            Some(Some(data)) => data,
        };
        let Some(player_data) = player_data(&data) else {
            // A `TypeError`, reported without the extractor prefix.
            return Err(Error::Reported(
                "'NoneType' object is not subscriptable".into(),
            ));
        };
        let playback_url = match player_data.get("playbackURL") {
            None => {
                return Err(Error::Reported(format!(
                    "{display_id}: An extractor error has occurred. (caused by KeyError('playbackURL')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            Some(Value::String(playback_url)) => playback_url.clone(),
            Some(_) => return Err(unsupported("playback URLs that are not strings")),
        };
        let formats = super::akamai::formats(client, &playback_url, NAME, &display_id)?;

        let mut fields: Vec<(String, Value)> = vec![
            ("id".into(), Value::from(display_id.clone())),
            ("display_id".into(), Value::from(display_id.clone())),
            ("formats".into(), Value::Null),
        ];
        // `**self._search_json_ld(webpage, display_id, fatal=False)`.
        let json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract JSON-LD ; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        if ["url", "formats", "id", "display_id", "subtitles"]
            .iter()
            .any(|key| json_ld.contains_key(*key))
        {
            return Err(unsupported("JSON-LD media URLs"));
        }
        // Dict merging: a present key keeps its position, a new one follows.
        let set = |fields: &mut Vec<(String, Value)>, key: &str, value: Value| match fields
            .iter_mut()
            .find(|(name, _)| name == key)
        {
            Some((_, old)) => *old = value,
            None => fields.push((key.to_owned(), value)),
        };
        for (key, value) in json_ld {
            set(&mut fields, &key, value);
        }
        // `**traverse_obj(player_data, {...})`.
        let text = |key: &str| player_data.get(key).and_then(Value::as_str);
        let id = match player_data.get("id") {
            Some(Value::String(id)) => Some(id.clone()),
            Some(Value::Number(id)) => Some(id.to_string()),
            Some(Value::Bool(id)) => Some(if *id { "True" } else { "False" }.to_owned()),
            _ => None,
        };
        if let Some(id) = id {
            set(&mut fields, "id", id.into());
        }
        for key in ["title", "description"] {
            if let Some(value) = text(key) {
                set(&mut fields, key, value.into());
            }
        }
        let creators: Vec<Value> = match player_data.get("author") {
            Some(Value::Array(items)) => items.iter().collect::<Vec<_>>(),
            Some(Value::Object(items)) => items.values().collect(),
            _ => Vec::new(),
        }
        .into_iter()
        .filter_map(|author| author.get("name").and_then(Value::as_str))
        .map(Value::from)
        .collect();
        if !creators.is_empty() {
            set(&mut fields, "creators", Value::Array(creators));
        }
        for (key, source) in [
            ("timestamp", "datePublished"),
            ("release_timestamp", "uploadDate"),
            ("modified_timestamp", "dateLastPublished"),
        ] {
            if let Some(timestamp) = text(source).and_then(crate::dates::parse_iso8601) {
                set(&mut fields, key, timestamp.into());
            }
        }
        if let Some(thumbnail) = text("thumbnail").and_then(super::json_ld::url_or_none) {
            set(&mut fields, "thumbnail", thumbnail.into());
        }
        if let Some(duration) = int_or_none(player_data.get("duration")) {
            set(&mut fields, "duration", json!(duration));
        }
        if let Some(channel) = player_data
            .pointer("/section/title")
            .and_then(Value::as_str)
        {
            set(&mut fields, "channel", channel.into());
        }
        let fields = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.cnbc.com/video/2023/12/07/mcdonalds-just-unveiled-cosmcsits-new-spinoff-brand.html"
        ));
        assert!(!suitable("https://www.cnbc.com/video/"));
    }

    #[test]
    fn the_first_clip_player_module_is_used() {
        let data = json!({"page": {"page": {"layout": [
            {"columns": [{"modules": [{"name": "other", "data": {"a": 1}}]}]},
            {"columns": [{"modules": [{"name": "clipPlayer", "data": "x"},
                                      {"name": "clipPlayer", "data": {"id": 1}}]}]},
        ]}}});
        assert_eq!(player_data(&data), Some(&json!({"id": 1})));
    }
}
