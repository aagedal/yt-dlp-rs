//! Upstream `JStreamIE` (`yt_dlp/extractor/jstream.py`): J-Stream Equipmedia
//! videos (`jstream:HOST:PUBLISHER:MID`) from the `eq_meta` JSONP's movie:
//! its `auto` HLS playlists, title, duration and thumbnail; pages loading
//! the J-Stream player `if.js` embed their `PlayerFactoryIF.create` movies.
use super::{Extractor, dict_info, download_webpage_note};
use crate::{Error, Result, Subtitles, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct JStream;

pub(crate) const NAME: &str = "JStream";
pub(crate) const KEY: &str = "JStream";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^jstream:(?P<host>www\d+):(?P<id>(?P<publisher>[a-z0-9]+):(?P<mid>\d+))",
    )
    .unwrap()
});
static METADATA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*metaDataResult\s*\(").unwrap());
static SCRIPT_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<script\s*[^>]+?src="https://ssl-cache\.stream\.ne\.jp/(?P<host>www\d+)/(?P<publisher>[a-z0-9]+)/[^"]+?/if\.js""#,
    )
    .unwrap()
});
static PLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)PlayerFactoryIF\.create\(\s*(\{[^\}]+?\})\s*\)\s*;").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls` under `orderedSet`. Player options that do not
/// parse, or whose `m` is not base64 UTF-8, are skipped here; upstream's
/// generic extraction fails on them.
pub(crate) fn embed_urls(_url: &Url, webpage: &str) -> Vec<Url> {
    let Some(script) = SCRIPT_TAG.captures(webpage) else {
        return Vec::new();
    };
    let (host, publisher) = (&script["host"], &script["publisher"]);
    let mut found: Vec<Url> = Vec::new();
    for player in PLAYER.captures_iter(webpage) {
        let text = crate::js_to_json::js_to_json(&player[1]);
        let Ok(info) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(mid) = info
            .get("m")
            .and_then(Value::as_str)
            .and_then(|m| base64::engine::general_purpose::STANDARD.decode(m).ok())
            .and_then(|mid| String::from_utf8(mid).ok())
        else {
            continue;
        };
        let Ok(embed) = Url::parse(&format!("jstream:{host}:{publisher}:{mid}")) else {
            continue;
        };
        if !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("J-Stream {what} are not supported natively"))
}

/// `video_info.get(KEY)` as an optional string.
fn text(info: &Value, key: &str) -> Result<Value> {
    match info.get(key) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::String(text)) => Ok(text.clone().into()),
        Some(_) => Err(unsupported(&format!("{key} values that are not strings"))),
    }
}

/// `float_or_none` of a JSON value.
fn float_or_none(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        _ => None,
    }
}

impl Extractor for JStream {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (host, publisher, mid) = (&found["host"], &found["publisher"], &found["mid"]);
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let base = format!("https://{publisher}.eq.webcdn.stream.ne.jp/{host}/{publisher}/jmc_pub");
        let jsonp = download_webpage_note(
            client,
            &Url::parse(&format!("{base}/eq_meta/v1/{mid}.jsonp"))?,
            NAME,
            &video_id,
            "Requesting video info",
        )?;
        let result = super::search_json(&METADATA, &jsonp, false)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract metaDataResult; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .ok_or_else(|| unsupported("metadata that do not parse"))?;
        let video_info = result.get("movie").ok_or_else(|| {
            Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('movie')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let movie_list_hls = match video_info.get("movie_list_hls") {
            Some(Value::Array(items)) => items,
            // Iterating `None`: upstream's uncaught `TypeError`.
            None | Some(Value::Null) => {
                return Err(Error::Reported("'NoneType' object is not iterable".into()));
            }
            Some(_) => return Err(unsupported("HLS lists that are not lists")),
        };
        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        for value in movie_list_hls {
            // `value.get('text') or ''`.
            let text = match value.get("text") {
                Some(Value::String(text)) => text.as_str(),
                Some(value) if super::nuxt::truthy(value) => {
                    return Err(unsupported("HLS list texts that are not strings"));
                }
                _ => "",
            };
            let Some(rest) = text.strip_prefix("auto") else {
                continue;
            };
            let m3u8_id = rest.strip_prefix('_').unwrap_or(rest);
            let path = match value.get("url") {
                Some(Value::String(path)) => path.as_str(),
                _ => return Err(unsupported("HLS list URLs that are not strings")),
            };
            let manifest = Url::parse(&format!("{base}/{path}"))
                .map_err(|_| unsupported("HLS list URLs that are not URLs"))?;
            let stream = super::extract_m3u8_fatal_master(
                client,
                &manifest,
                NAME,
                &video_id,
                (!m3u8_id.is_empty()).then_some(m3u8_id),
                true,
            )?;
            super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            formats.extend(stream.formats);
        }
        super::steam::remove_duplicate_formats(&mut formats);
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", text(video_info, "title")?),
                (
                    "duration",
                    float_or_none(video_info.get("duration")).map_or(Value::Null, Value::from),
                ),
                ("thumbnail", text(video_info, "thumbnail_url")?),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("jstream:www50:eqd638pvwx:752"));
        assert!(!suitable("jstream:www50:eqd638pvwx"));
        assert_eq!(
            temp_id("jstream:www50:eqd638pvwx:752").as_deref(),
            Some("eqd638pvwx:752")
        );
    }

    #[test]
    fn embeds_need_the_player_script() {
        let page = Url::parse("https://example.test/").unwrap();
        let player = "<script>PlayerFactoryIF.create({ b: 'x', m: 'NzUy' });</script>";
        assert!(embed_urls(&page, player).is_empty());
        let html = format!(
            r#"<script src="https://ssl-cache.stream.ne.jp/www50/eqd638pvwx/jmc_pub/jmc_swf/player/t3/if.js"></script>{player}{player}"#
        );
        assert_eq!(
            embed_urls(&page, &html),
            [Url::parse("jstream:www50:eqd638pvwx:752").unwrap()]
        );
    }
}
