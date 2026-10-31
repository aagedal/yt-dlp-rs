//! Upstream `TVAIE` (`yt_dlp/extractor/tva.py`): TVA+ episodes as
//! transparent `url` results of their Brightcove video (smuggled with the
//! `CA` geo countries), with the Next.js entity's episode metadata.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Tva;

const NAME: &str = "tvaplus";
const KEY: &str = "TVA";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?tvaplus\.ca/(?:[^/?#]+/)*[\w-]+-(?P<id>\d+)(?:$|[#?])",
    )
    .unwrap()
});
static SEASON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/s(?:ai|ea)son-(\d+)/").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

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

impl Extractor for Tva {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let entity_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &entity_id)?;
        let next_data = super::search_nextjs_data(&webpage, NAME, &entity_id)?;
        let mut entity = &next_data;
        for key in ["props", "pageProps", "staticEntity"] {
            entity = entity.get(key).ok_or_else(|| key_error(&entity_id, key))?;
        }
        let video_id = match entity.get("videoId") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "TVA+ video IDs that are not strings are not supported natively".into(),
                ));
            }
            None => return Err(key_error(&entity_id, "videoId")),
        };
        let episode = entity
            .get("name")
            .and_then(Value::as_str)
            .map(|name| Value::from(name.trim()))
            .unwrap_or(Value::Null);
        let text = |value: Option<&Value>| value.filter(|value| value.is_string()).cloned();
        let mut fields = Map::new();
        fields.insert("id".into(), video_id.clone().into());
        fields.insert("title".into(), episode.clone());
        fields.insert("episode".into(), episode);
        let optional = [
            ("description", text(entity.get("longDescription"))),
            (
                "duration",
                match entity.get("durationMillis") {
                    Some(Value::Number(number)) => number.as_f64(),
                    Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
                    _ => None,
                }
                .map(|millis| json!(millis / 1000.0)),
            ),
            (
                "channel",
                text(entity.pointer("/knownEntities/channel/name")),
            ),
            (
                "series",
                text(entity.pointer("/knownEntities/videoShow/name")),
            ),
            (
                "season_number",
                entity
                    .get("slug")
                    .and_then(Value::as_str)
                    .and_then(|slug| SEASON.captures(slug))
                    .and_then(|found| found[1].parse::<i64>().ok())
                    .map(Value::from),
            ),
            (
                "episode_number",
                int_or_none(entity.get("episodeNumber")).map(Value::from),
            ),
        ];
        for (key, value) in optional {
            if let Some(value) = value {
                fields.insert(key.into(), value);
            }
        }
        let target = super::brightcove::smuggle(
            &format!(
                "https://players.brightcove.net/5481942443001/default_default/index.html?videoId={video_id}"
            ),
            &json!({"geo_countries": ["CA"]}),
        );
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.tvaplus.ca/tva/alerte-amber/saison-1/episode-01-1000036619"
        ));
        assert!(suitable(
            "https://www.tvaplus.ca/tva/le-baiser-du-barbu/le-baiser-du-barbu-886644190"
        ));
        assert!(!suitable("https://www.tvaplus.ca/tva/alerte-amber"));
    }
}
