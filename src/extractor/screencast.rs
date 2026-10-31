//! Upstream `ScreencastIE` (`yt_dlp/extractor/screencast.py`): the page's
//! QuickTime embed, Flash variables, `og:video` meta or `MediaContentUrl`
//! media URL, with OpenGraph metadata.
use super::{Extractor, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Screencast;

const NAME: &str = "Screencast";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?screencast\.com/t/(?P<id>[a-zA-Z0-9]+)")
        .unwrap()
});
static EMBED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<embed name="Video".*?src="([^"]+)""#).unwrap());
static FLASH_VARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<param name="flashVars" value="([^"]+)""#).unwrap());
static INIT_PARAMS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<param name="initParams" value="([^"]+)""#).unwrap());
static META_SRC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"src=(.*?)(?:$|&)").unwrap());
static MEDIA_CONTENT_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"MediaContentUrl["']\s*:(?:"([^"]+)"|'([^']+)')"#).unwrap());
static TITLES: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r"<b>Title:</b> ([^<]+)</div>").unwrap(),
        Regex::new(r#"class="tabSeperator">></span><span class="tabText">(.+?)<"#).unwrap(),
        Regex::new(r"<title>([^<]+)</title>").unwrap(),
    ]
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Screencast {what} are not supported natively"))
}

/// `_html_search_regex(pattern, webpage, default=None)`: the cleaned group.
fn html_search(pattern: &Regex, webpage: &str) -> Option<String> {
    pattern.captures(webpage).map(|found| {
        let group = found
            .iter()
            .skip(1)
            .flatten()
            .next()
            .map_or("", |group| group.as_str());
        crate::html::clean_html(group)
    })
}

/// `urllib.parse.quote(text)` (`safe='/'`).
fn quote(text: &str) -> String {
    let mut quoted = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"_.-~/".contains(&byte) {
            quoted.push(byte as char);
        } else {
            quoted.push_str(&format!("%{byte:02X}"));
        }
    }
    quoted
}

/// `parse_qs(flash_vars)['content'][0]`: the first non-blank `content`.
fn flash_content(flash_vars: &str, video_id: &str) -> Result<String> {
    url::form_urlencoded::parse(flash_vars.as_bytes())
        .find(|(key, value)| key == "content" && !value.is_empty())
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('content')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

/// The media URL, in upstream's order of lookups.
fn video_url(webpage: &str, video_id: &str) -> Result<Option<String>> {
    if let Some(video_url) = html_search(&EMBED, webpage) {
        return Ok(Some(video_url));
    }
    let flash_vars = html_search(&FLASH_VARS, webpage)
        .filter(|vars| !vars.is_empty())
        .or_else(|| html_search(&INIT_PARAMS, webpage).map(|vars| vars.replace(',', "&")));
    if let Some(flash_vars) = flash_vars.filter(|vars| !vars.is_empty()) {
        let raw = quote(&flash_content(&flash_vars, video_id)?);
        return Ok(Some(raw.replace("http%3A", "http:")));
    }
    let meta = crate::html::html_search_meta(webpage, "og:video");
    if let Some(meta) = meta.as_deref().filter(|meta| !meta.is_empty())
        && let Some(found) = META_SRC.captures(meta)
    {
        return Ok(Some(found[1].to_owned()));
    }
    if let Some(video_url) = html_search(&MEDIA_CONTENT_URL, webpage) {
        return Ok(Some(video_url));
    }
    Ok(meta)
}

impl Extractor for Screencast {
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

        let Some(video_url) = video_url(&webpage, &video_id)? else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Cannot find video"
            )));
        };

        let title = match crate::html::og_search_property(&webpage, "title") {
            Some(title) => title,
            None => TITLES
                .iter()
                .find_map(|pattern| html_search(pattern, &webpage))
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] {video_id}: Unable to extract title; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?,
        };
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: unable to extract thumbnail URL; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let description = crate::html::og_search_property(&webpage, "description")
            .or_else(|| crate::html::html_search_meta(&webpage, "description"));
        if description.is_none() {
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: unable to extract description; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }

        // Processing's `sanitize_url`.
        let video_url = super::proto_relative_http(&video_url);
        let parsed = Url::parse(&video_url).map_err(|_| unsupported("media URLs"))?;
        let fields = vec![
            ("id", Value::from(video_id.clone())),
            ("url", Value::from(video_url.clone())),
            ("title", Value::from(title)),
            ("description", description.map_or(Value::Null, Value::from)),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
        ];
        // The single format is the info dict itself.
        let mut format_fields = serde_json::Map::new();
        format_fields.insert("format_id".into(), "".into());
        format_fields.insert("url".into(), parsed.as_str().into());
        format_fields.insert(
            "ext".into(),
            super::direct::determine_ext(parsed.as_str())
                .unwrap_or_else(|| "unknown_video".into())
                .to_lowercase()
                .into(),
        );
        let mut format: crate::VideoFormat =
            serde_json::from_value(Value::Object(format_fields))
                .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        crate::info_order::mark_format(&mut format, "url-only");
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!("untyped-dict;{}", keys.join(","));
        let fields = fields
            .into_iter()
            .filter(|(key, value)| *key == "title" || !value.is_null())
            .collect();
        let info = dict_info(url, (NAME, NAME), fields, vec![format]);
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.screencast.com/t/3ZEjQXlT"));
        assert!(suitable("http://screencast.com/t/aAB3iowa"));
        assert!(!suitable("https://app.screencast.com/qpYDGgjdOfG6X"));
    }

    #[test]
    fn flash_variables_follow_upstream_quoting() {
        let page = r#"<param name="initParams" value="a=1,content=http://e.invalid/a b.mp4">"#;
        assert_eq!(
            video_url(page, "x").unwrap().as_deref(),
            Some("http://e.invalid/a%20b.mp4")
        );
    }
}
