//! Upstream `Vbox7IE` (`yt_dlp/extractor/vbox7.py`): Vbox7 videos from the
//! player options API (an HLS playlist and the MP4 resolutions next to its
//! source, the unavailable and geo errors), merged with the play page's
//! JSON-LD; player iframes on other pages are followed.
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Vbox7;

pub(crate) const NAME: &str = "Vbox7";
pub(crate) const KEY: &str = "Vbox7";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:[^/]+\.)?vbox7\.com/(?:play:|(?:emb/external\.php|player/ext\.swf)\?.*?\bvid=)(?P<id>[\da-fA-F]+)",
    )
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+src=(?:"(?P<dq>(?:https?:)?//vbox7\.com/emb/external\.php[^"\n]+?)"|'(?P<sq>(?:https?:)?//vbox7\.com/emb/external\.php[^'\n]+?)')"#,
    )
    .unwrap()
});
static BASE_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://[^?#]+/").unwrap());

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

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Vbox7 {what} are not supported natively"))
}

/// `url_basename(url)`: the path's last component.
fn url_basename(url: &str) -> String {
    let path = Url::parse(url).map_or_else(|_| url.to_owned(), |url| url.path().to_owned());
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

impl Extractor for Vbox7 {
    fn key(&self) -> &'static str {
        KEY
    }

    /// `_GEO_COUNTRIES = ['BG']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["BG"])
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
        let api = super::update_url_query(
            &Url::parse("https://www.vbox7.com/aj/player/item/options")?,
            &[("vid", video_id.as_str())],
        );
        let response = download_json(client, &api, NAME, &video_id)?;
        let data = response.get("options").ok_or_else(|| {
            Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('options')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let src_url = match data.get("src") {
            None | Some(Value::Null) => None,
            Some(Value::String(src)) if src.is_empty() || src == "blank" => None,
            Some(Value::String(src)) => Some(src.clone()),
            Some(_) => return Err(unsupported("sources that are not strings")),
        };
        let Some(src_url) = src_url else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Video is unavailable"
            )));
        };
        let basename = url_basename(&src_url);
        let stem = basename
            .rsplit_once('.')
            .map_or(basename.as_str(), |(stem, _)| stem);
        let fmt_base = stem
            .rsplit_once('_')
            .map_or(stem, |(base, _)| base)
            .to_owned();
        if fmt_base == "vn" {
            // `raise_geo_restricted()`: no countries of its own.
            return Err(crate::geo::restricted(
                &format!(
                    "[{NAME}] {video_id}: This video is not available from your location due to geo restriction"
                ),
                &[],
            ));
        }
        let base = BASE_URL
            .find(&src_url)
            .map(|found| found.as_str().to_owned())
            .ok_or_else(|| unsupported("sources without a base URL"))?;
        let fmt_base = format!("{base}{fmt_base}");
        let mut formats = super::extract_m3u8_master_optional(
            client,
            &Url::parse(&format!("{fmt_base}.m3u8"))?,
            NAME,
            &video_id,
            Some("hls"),
            false,
            false,
        )
        .map(|stream| stream.formats)
        .unwrap_or_default();
        if let Some(Value::Array(resolutions)) = data.get("resolutions") {
            for res in resolutions {
                let Some(res) = res.as_i64().filter(|res| *res != 0) else {
                    continue;
                };
                formats.push(dict_format(
                    vec![
                        ("url", format!("{fmt_base}_{res}.mp4").into()),
                        ("format_id", format!("http-{res}").into()),
                        ("height", res.into()),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        // `_search_json_ld(self._download_webpage(..., fatal=False) or '',
        // video_id, fatal=False)`.
        let page = Url::parse(&format!("https://www.vbox7.com/play:{video_id}"))?;
        let webpage = match super::download_webpage(client, &page, NAME, &video_id) {
            Ok(webpage) => webpage,
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Unable to download webpage: {}",
                    crate::upstream_error_text(&source)
                ));
                String::new()
            }
            Err(error) => return Err(error),
        };
        if super::json_ld::yield_json_ld(&webpage).is_empty() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract JSON-LD ; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let json_ld = super::json_ld::search(&webpage);
        let mut fields: Vec<(String, Value)> = vec![
            ("id".into(), Value::from(video_id.clone())),
            ("formats".into(), Value::Null),
        ];
        for (key, value) in json_ld {
            if key == "id" || key == "formats" {
                continue;
            }
            fields.push((key, value));
        }
        let mut set = |key: &str, value: Option<Value>| {
            let Some(value) = value else {
                return;
            };
            match fields.iter_mut().find(|(name, _)| name == key) {
                Some((_, old)) => *old = value,
                None => fields.push((key.to_owned(), value)),
            }
        };
        set(
            "title",
            data.get("title").filter(|value| value.is_string()).cloned(),
        );
        set(
            "uploader",
            data.get("uploader")
                .filter(|value| value.is_string())
                .cloned(),
        );
        set(
            "duration",
            match data.get("duration") {
                Some(Value::Number(number)) => number
                    .as_i64()
                    .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
                    .map(Value::from),
                Some(Value::String(text)) => text.trim().parse::<i64>().ok().map(Value::from),
                _ => None,
            },
        );
        let fields: Vec<(&str, Value)> = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        Ok(Some(dict_info(url, (NAME, KEY), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://vbox7.com/play:0946fff23c"));
        assert!(suitable(
            "http://vbox7.com/emb/external.php?vid=a240d20f9c&autoplay=1"
        ));
        assert!(suitable(
            "http://i49.vbox7.com/player/ext.swf?vid=0946fff23c&autoplay=1"
        ));
        assert!(!suitable("http://vbox7.com/user:x"));
    }

    #[test]
    fn source_bases_follow_upstream() {
        assert_eq!(
            url_basename("https://edge1.vbox7.com/sl/0946fff23c/0946fff23c_576.mpd"),
            "0946fff23c_576.mpd"
        );
    }
}
