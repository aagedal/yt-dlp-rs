//! Upstream `STVPlayerIE` (`yt_dlp/extractor/stv.py`): an STV Player page's
//! Next.js `playerApiCache` (or the player API) result as a
//! `url_transparent` result for the Brightcove player, with the episode's
//! description, duration, subtitles, views and programme name.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct StvPlayer;

const NAME: &str = "stv:player";
const KEY: &str = "STVPlayer";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://player\.stv\.tv/(?P<type>episode|video)/(?P<id>[a-z0-9]{4})",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("STV Player {what} are not supported natively"))
}

/// Python `str(value)` of a JSON scalar.
fn py_str(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        _ => Err(unsupported("IDs that are not strings or numbers")),
    }
}

/// `float_or_none(value, 1000)`.
fn milliseconds(value: Option<&Value>) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    number.map_or(Value::Null, |number| json!(number / 1000.0))
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|number| number.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

impl Extractor for StvPlayer {
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
        let ptype = found["type"].to_owned();
        let url_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_download_webpage(url, video_id, fatal=False) or ''`.
        let webpage = match super::download_webpage(client, url, NAME, &url_id) {
            Ok(webpage) => webpage,
            // A non-fatal request's warning has no `(caused by ...)`.
            Err(Error::ExtractorRequest { source, .. }) => {
                let text = super::request_error_text(NAME, &url_id, "webpage", &source);
                let text = text
                    .split_once(" (caused by ")
                    .map_or(text.as_str(), |(head, _)| head);
                crate::screen::warning(text);
                String::new()
            }
            Err(error) => return Err(error),
        };
        // `_search_nextjs_data(webpage, video_id, default={}).get('props')`.
        let props = super::search_nextjs_data(&webpage, NAME, &url_id)
            .ok()
            .and_then(|data| data.get("props").cloned())
            .filter(super::nuxt::truthy)
            .unwrap_or_else(|| json!({}));
        let cache = props
            .pointer("/initialReduxState/playerApiCache")
            .filter(|cache| super::nuxt::truthy(cache))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let cache = match cache {
            Value::Object(cache) => cache,
            _ => return Err(unsupported("player API caches that are not objects")),
        };
        let cached = cache
            .iter()
            .find(|(key, _)| key.starts_with("/episodes/") || key.starts_with("/shortform/"));
        let (api_path, resp) = match cached {
            Some((key, value)) => (key.clone(), value.clone()),
            None => {
                let episode_id = props
                    .pointer("/pageProps/episodeId")
                    .filter(|id| !id.is_null())
                    .map(py_str)
                    .transpose()?
                    .filter(|id| !id.is_empty());
                let kind = if ptype == "episode" {
                    "episodes"
                } else {
                    "shortform"
                };
                (
                    format!("/{kind}/{}", episode_id.unwrap_or_else(|| url_id.clone())),
                    json!({}),
                )
            }
        };
        let Value::Object(resp) = resp else {
            return Err(unsupported(
                "cached player API responses that are not objects",
            ));
        };
        let key_error = |key: &str| {
            Error::Reported(format!(
                "{url_id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let result = match resp
            .get("results")
            .filter(|result| super::nuxt::truthy(result))
        {
            Some(result) => result.clone(),
            None => {
                let api = Url::parse(&format!("https://player.api.stv.tv/v1{api_path}"))?;
                let resp = download_json(client, &api, NAME, &url_id)?;
                resp.get("results")
                    .cloned()
                    .ok_or_else(|| key_error("results"))?
            }
        };
        let Value::Object(result) = result else {
            return Err(unsupported("player API results that are not objects"));
        };
        let video = result.get("video").ok_or_else(|| key_error("video"))?;
        let Value::Object(video) = video else {
            return Err(unsupported("videos that are not objects"));
        };
        let video_id = py_str(video.get("id").ok_or_else(|| key_error("id"))?)?;

        let mut subtitles: Vec<Value> = Vec::new();
        match result.get("_subtitles") {
            None | Some(Value::Null) => {}
            Some(Value::Object(tracks)) => {
                for (ext, sub_url) in tracks {
                    let ext = if ext == "webvtt" { "vtt" } else { ext.as_str() };
                    subtitles.push(json!({"ext": ext, "url": sub_url}));
                }
            }
            Some(value) if !super::nuxt::truthy(value) => {}
            Some(_) => return Err(unsupported("subtitle lists that are not objects")),
        }
        let subtitles = if subtitles.is_empty() {
            json!({})
        } else {
            json!({"en": subtitles})
        };

        let programme = result
            .get("programme")
            .filter(|programme| super::nuxt::truthy(programme))
            .cloned()
            .unwrap_or_else(|| json!({}));
        if programme.get("drmEnabled").is_some_and(super::nuxt::truthy) {
            crate::no_formats::raise_expected(
                NAME,
                Some(&video_id),
                Some(&url_id),
                "This video is DRM protected",
            )?;
        }
        // `programme.get('name') or programme.get('shortName')`.
        let series = programme
            .get("name")
            .filter(|name| super::nuxt::truthy(name))
            .or_else(|| programme.get("shortName"))
            .cloned()
            .unwrap_or(Value::Null);

        let target = super::brightcove::smuggle(
            &format!(
                "http://players.brightcove.net/1486976045/default_default/index.html?videoId={video_id}"
            ),
            &json!({"geo_countries": ["GB"]}),
        );
        let mut fields = Map::new();
        for (key, value) in [
            (
                "description",
                result.get("summary").cloned().unwrap_or(Value::Null),
            ),
            ("duration", milliseconds(video.get("length"))),
            ("subtitles", subtitles),
            ("view_count", int_or_none(result.get("views"))),
            ("series", series),
        ] {
            fields.insert(key.into(), value);
        }
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.id = video_id;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": Value::Object(fields), "subtitle_keys": "ext,url"}),
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
            "https://player.stv.tv/video/4gwd/emmerdale/60-seconds-on-set-with-laura-norton/"
        ));
        assert!(suitable(
            "https://player.stv.tv/episode/4125/jennifer-saunders-memory-lane"
        ));
        assert!(!suitable("https://player.stv.tv/summary/emmerdale"));
    }

    #[test]
    fn numbers_follow_upstream_helpers() {
        assert_eq!(milliseconds(Some(&json!(5000))), json!(5.0));
        assert_eq!(milliseconds(Some(&json!("1500"))), json!(1.5));
        assert_eq!(int_or_none(Some(&json!("12"))), json!(12));
        assert_eq!(int_or_none(Some(&json!(null))), Value::Null);
    }
}
