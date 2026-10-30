//! Upstream `RTVSIE` (`yt_dlp/extractor/rtvs.py`): STVR (formerly RTVS)
//! radio and television archive items from the page's player iframe and its
//! playlist JSON: an MP3 source or the HLS playlist, with the item's title,
//! description, length, image and creation time.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Rtvs;

const NAME: &str = "stvr";
const KEY: &str = "RTVS";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?(?:rtvs|stvr)\.sk/(?:radio|televizia)/archiv(?:/\d+)?/(?P<id>\d+)/?(?:[#?]|$)",
    )
    .unwrap()
});
static IFRAME_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<iframe[^>]+id\s*=\s*"player_[^_]+_([0-9]+)""#).unwrap());
static JSON_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+url\s*=\s*"([^"]+)"\s*\+\s*ruurl"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn search(regex: &Regex, text: &str, id: &str, name: &str) -> Result<String> {
    regex
        .captures(text)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("STVR {what} are not supported natively"))
}

impl Extractor for Rtvs {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let iframe_id = search(&IFRAME_ID, &webpage, &video_id, "Iframe ID")?;
        let iframe_pattern = Regex::new(&format!(
            r#"<iframe[^>]+id\s*=\s*"player_[^_]+_{}"[^>]+src\s*=\s*"([^"]+)""#,
            regex::escape(&iframe_id)
        ))
        .map_err(|_| unsupported("iframe IDs"))?;
        let iframe_url = search(&iframe_pattern, &webpage, &video_id, "Iframe URL")?;
        let iframe_url = url
            .join(&iframe_url)
            .map_err(|_| unsupported("iframe URLs that are not URLs"))?;
        let iframe = super::download_webpage_note(
            client,
            &iframe_url,
            NAME,
            &video_id,
            "Downloading iframe",
        )?;
        let json_url = search(&JSON_URL, &iframe, &video_id, "json URL")?;
        let api = Url::parse(&format!("https:{json_url}b=mozilla&p=win&v=97&f=0&d=1"))
            .map_err(|_| unsupported("playlist JSON URLs that are not URLs"))?;
        let mut data = download_json(client, &api, NAME, &video_id)?;
        if let Some(clip) = data
            .get("clip")
            .filter(|clip| super::nuxt::truthy(clip))
            .cloned()
            && let Some(data) = data.as_object_mut()
        {
            data.insert("playlist".into(), json!([clip]));
        }
        let item = data.pointer("/playlist/0");
        let source = item.and_then(|item| item.pointer("/sources/0"));
        let src = source.and_then(|source| source.get("src"));
        let formats = if source
            .and_then(|source| source.get("type"))
            .and_then(Value::as_str)
            == Some("audio/mp3")
        {
            vec![dict_format(
                vec![("url", src.cloned().unwrap_or(Value::Null))],
                NAME,
                &video_id,
            )?]
        } else {
            let manifest = src
                .and_then(Value::as_str)
                .and_then(|src| Url::parse(src).ok())
                .ok_or_else(|| unsupported("sources without a playlist URL"))?;
            let mut stream =
                super::extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, false)?;
            super::hls::without_ext(&mut stream);
            stream.formats
        };
        let get = |key: &str| {
            item.and_then(|item| item.get(key))
                .cloned()
                .unwrap_or(Value::Null)
        };
        let duration = item
            .and_then(|item| item.get("length"))
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
            .map_or(Value::Null, |duration| json!(duration));
        let timestamp = item
            .and_then(|item| item.get("datetime_create"))
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, false))
            .map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("display_id", iframe_id.into()),
                ("title", get("title")),
                ("description", get("description")),
                ("duration", duration),
                ("thumbnail", get("image")),
                ("timestamp", timestamp),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.rtvs.sk/radio/archiv/11224/414872"));
        assert!(suitable(
            "https://www.stvr.sk/televizia/archiv/18083?utm_source=x"
        ));
        assert!(!suitable("https://www.stvr.sk/televizia/program"));
    }
}
