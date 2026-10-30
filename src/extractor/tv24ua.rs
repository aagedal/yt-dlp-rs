//! Upstream `TV24UAVideoIE` (`yt_dlp/extractor/tv24ua.py`): 24 Kanal player
//! pages from their `vPlayConfig.sources` (HLS or direct files), poster and
//! page title and description; 24tv.ua player iframes are embeds.
use super::{Extractor, dict_format, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Tv24UaVideo;

pub(crate) const NAME: &str = "24tv.ua";
pub(crate) const KEY: &str = "TV24UAVideo";

const VALID: &str = r"https?://24tv\.ua/news/showPlayer\.do.*?(?:\?|&)objectId=(?P<id>\d+)";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| crate::extractor::url_regex(&format!("^{VALID}")).unwrap());
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r#"<iframe[^>]+?src=["']?(?P<url>{VALID})["']?"#)).unwrap()
});
static SOURCES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"vPlayConfig\.sources\s*=\s*(?P<json>\[\{\s*(?s:.+?)\s*\}\])").unwrap()
});
static CONFIG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"var\s*vPlayConfig\s*=\s*").unwrap());

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
        let Ok(embed) = url.join(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("24tv.ua {what} are not supported natively"))
}

impl Extractor for Tv24UaVideo {
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
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        for captures in SOURCES.captures_iter(&webpage) {
            let text = crate::js_to_json::js_to_json(&captures["json"]);
            // `_parse_json(..., ignore_extra=True)`: the first JSON value.
            let sources = serde_json::Deserializer::from_str(&text)
                .into_iter::<Value>()
                .next()
                .and_then(std::result::Result::ok)
                .ok_or_else(|| unsupported("source lists that do not parse"))?;
            let Value::Array(sources) = sources else {
                return Err(unsupported("source lists that are not lists"));
            };
            for source in &sources {
                let src = match source.get("src") {
                    Some(Value::String(src)) => src.clone(),
                    _ => return Err(unsupported("sources without a URL")),
                };
                let mime = source
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if super::direct::mimetype2ext_or(mime, None) == "m3u8" {
                    let manifest = Url::parse(&src)
                        .map_err(|_| unsupported("playlist URLs that are not URLs"))?;
                    let mut stream =
                        extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, true)?;
                    super::hls::without_ext(&mut stream);
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                } else {
                    let ext = super::direct::determine_ext(&src).map_or(Value::Null, Value::from);
                    formats.push(dict_format(
                        vec![("url", src.into()), ("ext", ext)],
                        NAME,
                        &video_id,
                    )?);
                }
            }
        }
        let poster = super::search_json(&CONFIG, &webpage, true)
            .flatten()
            .and_then(|config| config.get("poster").cloned())
            .filter(|poster| !poster.is_null());
        let thumbnail = poster.or_else(|| {
            let og = crate::html::og_search_property(&webpage, "image");
            if og.is_none() {
                super::search_warning(NAME, "thumbnail URL");
            }
            og.map(Value::from)
        });
        // `_generic_title('', webpage)`.
        let title = crate::html::og_search_property(&webpage, "title")
            .filter(|title| !title.is_empty())
            .or_else(|| {
                crate::html::html_extract_title(&webpage).filter(|title| !title.is_empty())
            });
        let description = crate::html::og_search_property(&webpage, "description");
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("thumbnail", thumbnail.unwrap_or(Value::Null)),
                ("title", optional(title)),
                ("description", optional(description)),
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
        assert!(suitable(
            "https://24tv.ua/news/showPlayer.do?objectId=2074790&videoUrl=2022/07/2074790&w=640&h=360"
        ));
        assert!(suitable(
            "https://24tv.ua/news/showPlayer.do?videoUrl=x&objectId=1"
        ));
        assert!(!suitable("https://24tv.ua/news/x"));
    }
}
