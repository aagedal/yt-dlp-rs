//! Upstream `Screen9IE` (`yt_dlp/extractor/screen9.py`): the embed page's
//! `config` HLS stream and progressive MP4 with its plugin metadata.
use super::{Extractor, download_webpage, extract_m3u8_fatal};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Screen9;

const NAME: &str = "Screen9";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:\w+\.screen9\.(?:tv|com)|play\.su\.se)/(?:embed|media)/(?P<id>[^?#/]+)",
    )
    .unwrap()
});
static CONFIG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"var\s+config\s*=\s*").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Screen9 {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let embed = Url::parse(&format!("https://api.screen9.com/embed/{id}"))?;
        let webpage = download_webpage(client, &embed, NAME, &id)?;
        // `_search_json(r'var\s+config\s*=', webpage, 'config', video_id)`.
        let config = match super::search_json(&CONFIG, &webpage, false) {
            Some(Some(config)) => config,
            Some(None) => {
                return Err(Error::UnsupportedHtml5(
                    "Screen9 configs that are not JSON".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract config; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        // `('src', lambda _, v: v['type'] == TYPE, 'src')`, the first.
        let source = |kind: &str| {
            config
                .get("src")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|source| source.get("type").and_then(Value::as_str) == Some(kind))
                .and_then(|source| source.get("src"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let (Some(manifest), Some(mp4)) = (source("application/x-mpegURL"), source("video/mp4"))
        else {
            return Err(Error::UnsupportedHtml5(
                "Screen9 configs without both HLS and MP4 sources".into(),
            ));
        };
        let manifest = Url::parse(&manifest)?;
        let mp4 = Url::parse(&mp4)
            .map_err(|_| Error::UnsupportedHtml5("Screen9 MP4 sources that are not URLs".into()))?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &id, None, true)?;
        let ext =
            super::direct::determine_ext(mp4.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut format: VideoFormat = serde_json::from_value(json!({
            "url": mp4.as_str(),
            "format": "mp4",
            "format_id": "",
            "ext": ext,
        }))
        .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "dict:url,format");
        info.formats.push(format);
        let plugins = config.get("plugins");
        let at = |path: &[&str]| {
            path.iter()
                .try_fold(plugins?, |value, key| value.get(key))
                .filter(|value| !value.is_null())
        };
        let title = [
            &["title", "title"][..],
            &["googleAnalytics", "title"],
            &["share", "mediaTitle"],
        ]
        .into_iter()
        .find_map(&at);
        let description = at(&["title", "description"]);
        let thumbnail = config.get("poster").filter(|value| !value.is_null());
        let as_text = |value: Option<&Value>| match value {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(Error::UnsupportedHtml5(
                "Screen9 metadata that is not a string".into(),
            )),
        };
        let title = as_text(title)?;
        let description = as_text(description)?;
        let thumbnail = as_text(thumbnail)?;
        info.id = id;
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = description;
        info.metadata.thumbnail = thumbnail;
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,description,thumbnail,formats,subtitles",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://api.screen9.com/embed/8kTNEjvoXGM33dmWwF0uDA"
        ));
        assert!(suitable(
            "https://folkhogskolekanalen.screen9.tv/media/gy35PKLHe-5K29RYHga2bw/ett-starkare"
        ));
        assert!(suitable(
            "https://play.su.se/media/H1YA0EYNCxiesrSU1kaRBQ/baltic-breakfast"
        ));
        assert!(!suitable("https://api.screen9.com/embed/"));
    }
}
