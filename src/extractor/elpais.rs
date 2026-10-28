//! Upstream `ElPaisIE` (`yt_dlp/extractor/elpais.py`): El País videos from
//! the page's `url_cache` prefix and its multimedia JSONP (else the page's
//! media-file variable), with the still, title, date and description.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct ElPais;

const NAME: &str = "ElPais";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:[^.]+\.)?elpais\.com/.*/(?P<id>[^/#?]+)\.html(?:$|[?#])").unwrap()
});
static PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+url_cache\s*=\s*"([^"]+)";"#).unwrap());
static MULTIMEDIA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"id_multimedia\s*=\s*'([^']+)'").unwrap());
static MEDIA_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:URLMediaFile|urlVideo_\d+)\s*=\s*url_cache\s*\+\s*'([^']+)'").unwrap()
});
static STILL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:URLMediaStill|urlFotogramaFijo_\d+)\s*=\s*url_cache\s*\+\s*'([^']+)'").unwrap()
});
static TITLES: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r"tituloVideo\s*=\s*'([^']+)'").unwrap(),
        Regex::new(r#"<h2 class="entry-header entry-title.*?>(.*?)</h2>"#).unwrap(),
        Regex::new(r#"<h1[^>]+class="titulo"[^>]*>([^<]+)"#).unwrap(),
    ]
});
static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<p class="date-header date-int updated"\s+title="([^"]+)">"#).unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn first(regex: &Regex, text: &str) -> Option<String> {
    regex.captures(text).map(|found| found[1].to_owned())
}

impl Extractor for ElPais {
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
        let fail = |name: &str| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let prefix = first(&PREFIX, &webpage)
            .map(|prefix| crate::html::clean_html(&prefix))
            .ok_or_else(|| fail("URL prefix"))?;
        let suffix = match first(&MULTIMEDIA, &webpage) {
            Some(id_multimedia) => {
                let api = Url::parse(&format!("http://elpais.com/vdpep/1/?pepid={id_multimedia}"))?;
                let info = super::download_json_transformed(
                    client,
                    &api,
                    NAME,
                    &video_id,
                    "Downloading JSON metadata",
                    &|text| Ok(super::weibo::strip_jsonp(text)),
                )?;
                match info.get("mp4") {
                    Some(Value::String(mp4)) => mp4.clone(),
                    Some(_) => {
                        return Err(Error::UnsupportedHtml5(
                            "El País media paths that are not strings are not supported natively"
                                .into(),
                        ));
                    }
                    None => {
                        return Err(Error::Reported(format!(
                            "{video_id}: An extractor error has occurred. (caused by KeyError('mp4')); {}",
                            crate::BUG_REPORTS_MESSAGE
                        )));
                    }
                }
            }
            None => first(&MEDIA_FILE, &webpage).ok_or_else(|| fail("video URL"))?,
        };
        let video_url = format!("{prefix}{suffix}");
        let thumbnail = first(&STILL, &webpage)
            .map(|still| format!("{prefix}{still}"))
            .filter(|thumbnail| !thumbnail.is_empty())
            .or_else(|| {
                let og = crate::html::og_search_property(&webpage, "image");
                if og.is_none() {
                    super::search_warning(NAME, "thumbnail URL");
                }
                og
            });
        let title = TITLES
            .iter()
            .find_map(|pattern| first(pattern, &webpage))
            .map(|title| crate::html::clean_html(&title))
            .filter(|title| !title.is_empty())
            .or_else(|| {
                let og = crate::html::og_search_property(&webpage, "title");
                if og.is_none() {
                    super::search_warning(NAME, "OpenGraph title");
                }
                og
            });
        let date = first(&DATE, &webpage)
            .filter(|date| !date.is_empty())
            .or_else(|| {
                let meta = crate::html::html_search_meta(&webpage, "datePublished");
                if meta.is_none() {
                    super::search_warning(NAME, "timestamp");
                }
                meta
            });
        let upload_date = date.and_then(|date| crate::dates::unified_strdate(&date, false));
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            super::search_warning(NAME, "OpenGraph description");
        }
        let ext = super::direct::determine_ext(&video_url)
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        let mut format: crate::VideoFormat =
            serde_json::from_value(json!({"format_id": "", "url": video_url, "ext": ext}))
                .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "url-only");
        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let fields = vec![
            ("id", Value::from(video_id)),
            ("url", video_url.into()),
            ("title", optional(title)),
            ("description", optional(description)),
            ("thumbnail", optional(thumbnail)),
            ("upload_date", optional(upload_date)),
        ];
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("untyped-dict;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(
            dict_info(url, (NAME, NAME), fields, vec![format]),
            &shape,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://blogs.elpais.com/la-voz-de-inaki/2014/02/tiempo-nuevo-recetas-viejas.html"
        ));
        assert!(suitable(
            "https://elpais.com/elpais/2017/01/26/ciencia/1485456786_417876.html"
        ));
        assert!(!suitable("https://elpais.com/"));
    }
}
