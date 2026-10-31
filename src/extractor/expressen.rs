//! Upstream `ExpressenIE` (`yt_dlp/extractor/expressen.py`): Expressen and
//! Dagens industri videos from the page's `data-video-tracking-info` and
//! `data-article-data` attributes (an HLS playlist or a direct stream);
//! player iframes on other pages are followed.
use super::{Extractor, dict_format, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Expressen;

pub(crate) const NAME: &str = "Expressen";
pub(crate) const KEY: &str = "Expressen";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?(?:expressen|di)\.se/(?:(?:tvspelare/video|video-?player/embed)/)?(?:tv|nyheter)/(?:[^/?#]+/)*(?P<id>[^/?#&]+)",
    )
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+\bsrc=(?:"(?P<dq>(?:https?:)?//(?:www\.)?(?:expressen|di)\.se/(?:tvspelare/video|videoplayer/embed)/tv/[^"\n]+?)"|'(?P<sq>(?:https?:)?//(?:www\.)?(?:expressen|di)\.se/(?:tvspelare/video|videoplayer/embed)/tv/[^'\n]+?)')"#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Some(raw) = captures.name("dq").or_else(|| captures.name("sq")) else {
            continue;
        };
        let Ok(embed) = url.join(&crate::html::unescape_html(raw.as_str())) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Expressen {what} are not supported natively"))
}

/// `extract_data(name)`: the quoted `data-NAME` attribute's unescaped JSON.
fn extract_data(webpage: &str, name: &str, display_id: &str) -> Result<Value> {
    let pattern = Regex::new(&format!(
        r#"data-{name}=(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')"#
    ))
    .expect("static pattern");
    let raw = pattern
        .captures(webpage)
        .and_then(|found| found.name("dq").or_else(|| found.name("sq")))
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract info; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    crate::json::from_str(&crate::html::unescape_html(raw.as_str()))
        .map_err(|_| unsupported("player data that does not parse"))
}

fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| super::nuxt::truthy(value))
}

impl Extractor for Expressen {
    fn key(&self) -> &'static str {
        KEY
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
        let info = extract_data(&webpage, "video-tracking-info", &display_id)?;
        let forced_string = matches!(info.get("contentId"), Some(Value::Number(_)));
        let video_id = match info.get("contentId") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            Some(_) => return Err(unsupported("content IDs that are not strings")),
            None => return Err(key_error(&display_id, "contentId")),
        };
        let data = extract_data(&webpage, "article-data", &display_id)?;
        let stream = match data.get("stream") {
            Some(Value::String(stream)) => stream.clone(),
            Some(_) => return Err(unsupported("streams that are not strings")),
            None => return Err(key_error(&display_id, "stream")),
        };
        let formats = if super::direct::determine_ext(&stream).as_deref() == Some("m3u8") {
            let manifest =
                Url::parse(&stream).map_err(|_| unsupported("stream URLs that are not URLs"))?;
            extract_m3u8_fatal_master(client, &manifest, NAME, &display_id, Some("hls"), false)?
                .formats
        } else {
            vec![dict_format(
                vec![("url", stream.into())],
                NAME,
                &display_id,
            )?]
        };
        let title = match truthy(info.get("titleRaw")) {
            Some(title) => title.clone(),
            None => data
                .get("title")
                .cloned()
                .ok_or_else(|| key_error(&display_id, "title"))?,
        };
        let thumbnail = truthy(info.get("socialMediaImage"))
            .or_else(|| data.get("image"))
            .cloned()
            .unwrap_or(Value::Null);
        let duration = truthy(info.get("videoTotalSecondsDuration"))
            .or_else(|| data.get("totalSecondsDuration"))
            .and_then(|value| match value {
                Value::Number(number) => number
                    .as_i64()
                    .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
                Value::String(text) => text.trim().parse().ok(),
                _ => None,
            });
        let timestamp = info
            .get("publishDate")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, false));
        let info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("display_id", display_id.into()),
                ("title", title),
                (
                    "description",
                    info.get("descriptionRaw").cloned().unwrap_or(Value::Null),
                ),
                ("thumbnail", thumbnail),
                ("duration", duration.map_or(Value::Null, Value::from)),
                ("timestamp", timestamp.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
            ],
            formats,
        );
        // Processing's `_fill_common_fields` warning for a non-string ID.
        if forced_string {
            crate::screen::warning(
                "\"id\" field is not a string - forcing string conversion, there is an error in extractor",
            );
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "https://www.expressen.se/tv/ledare/ledarsnack/ledarsnack-om-arbetslosheten/",
            "https://www.expressen.se/tvspelare/video/tv/ditv/ekonomistudion/experterna/?embed=true",
            "https://www.di.se/videoplayer/embed/tv/ditv/borsmorgon/implantica/?embed=true",
            "https://www.expressen.se/video-player/embed/tv/nyheter/ekero-fodda",
            "https://www.expressen.se/nyheter/efter-egna-telefonbluffen-escobar-stammer-klarna/",
        ] {
            assert!(suitable(url), "{url}");
        }
        assert!(!suitable("https://www.expressen.se/sport/"));
    }
}
