//! Upstream `MagellanTVIE` (`yt_dlp/extractor/magellantv.py`): MagellanTV
//! videos and series episodes from the page's Next.js `reactContext`: the
//! video detail (or the series' current episode) HLS manifests, metadata,
//! and the error page's message when there are no formats.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MagellanTv;

const NAME: &str = "MagellanTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?magellantv\.com/(?:watch|video)/(?P<id>[\w-]+)",
    )
    .unwrap()
});

/// `url_or_none`'s scheme check.
static URL_OR_NONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:https?|rt(?:m(?:pt?[es]?|fp)|sp[su]?)|mms|ftps?):)?//").unwrap()
});

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

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MagellanTV {what} are not supported natively"))
}

/// `url_or_none(value)`.
fn url_or_none(value: &Value) -> Option<&str> {
    let url = value.as_str()?.trim();
    URL_OR_NONE.is_match(url).then_some(url)
}

/// `traverse_obj(data, ((('manifests', ..., 'hls'), 'jwp_video_url'),
/// {url_or_none}))` in order, deduplicated by `set()` (whose iteration
/// order upstream is hash-dependent; the first occurrence is kept here).
fn manifest_urls(data: &Value) -> Vec<String> {
    let mut found = Vec::new();
    let manifests: Vec<&Value> = match data.get("manifests") {
        Some(Value::Array(manifests)) => manifests.iter().collect(),
        Some(Value::Object(manifests)) => manifests.values().collect(),
        _ => Vec::new(),
    };
    for manifest in manifests {
        if let Some(url) = manifest.get("hls").and_then(url_or_none) {
            found.push(url.to_owned());
        }
    }
    if let Some(url) = data.get("jwp_video_url").and_then(url_or_none) {
        found.push(url.to_owned());
    }
    let mut unique: Vec<String> = Vec::new();
    for url in found {
        if !unique.contains(&url) {
            unique.push(url);
        }
    }
    unique
}

impl Extractor for MagellanTv {
    fn key(&self) -> &'static str {
        NAME
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
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        let mut context = &next_data;
        for key in ["props", "pageProps", "reactContext"] {
            context = context.get(key).ok_or_else(|| key_error(&video_id, key))?;
        }
        // `((('video', 'detail'), ('series', 'currentEpisode')), {dict}, any)`.
        let data = [("video", "detail"), ("series", "currentEpisode")]
            .iter()
            .filter_map(|(outer, inner)| context.get(*outer)?.get(*inner))
            .find(|data| data.as_object().is_some_and(|data| !data.is_empty()))
            .cloned()
            .unwrap_or(Value::Null);
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        for manifest in manifest_urls(&data) {
            let manifest = Url::parse(&manifest)
                .map_err(|_| unsupported("playlist URLs that are not absolute URLs"))?;
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("hls"),
                true,
                false,
            ) {
                formats.extend(stream.formats);
                super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        }
        if formats.is_empty()
            && let Some(Value::String(error)) = context.pointer("/errorDetailPage/errorMessage")
        {
            if error.contains("available in your country") {
                return Err(crate::geo::restricted(
                    &format!("[{NAME}] {video_id}: {error}"),
                    &[],
                ));
            }
            crate::no_formats::raise_expected(
                NAME,
                None,
                Some(&video_id),
                &format!("{NAME} said: {error}"),
            )?;
        }
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        if let Some(title) = data.get("title").filter(|title| title.is_string()) {
            fields.push(("title", title.clone()));
        }
        if let Some(description) = data
            .pointer("/metadata/description")
            .filter(|description| description.is_string())
        {
            fields.push(("description", description.clone()));
        }
        // `parse_duration`: only strings parse.
        if let Some(duration) = data
            .get("duration")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
        {
            fields.push((
                "duration",
                match duration {
                    crate::info::PyNumber::Int(seconds) => seconds.into(),
                    crate::info::PyNumber::Float(seconds) => seconds.into(),
                },
            ));
        }
        let age_limit = super::videopress::parse_age_limit(data.get("ratingCategory"));
        if !age_limit.is_null() {
            fields.push(("age_limit", age_limit));
        }
        let tags: Vec<Value> = match data.get("tags") {
            Some(Value::Array(tags)) => {
                tags.iter().filter(|tag| tag.is_string()).cloned().collect()
            }
            Some(Value::Object(tags)) => tags
                .values()
                .filter(|tag| tag.is_string())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        if !tags.is_empty() {
            fields.push(("tags", Value::Array(tags)));
        }
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.magellantv.com/watch/incas-the-new-story?type=v"
        ));
        assert!(suitable(
            "https://magellantv.com/video/tortured-to-death-murdering-the-nanny"
        ));
        assert!(!suitable("https://www.magellantv.com/series/x"));
    }
}
