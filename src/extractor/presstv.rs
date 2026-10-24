//! Upstream `PressTVIE` (`yt_dlp/extractor/presstv.py`): the page's hidden
//! `inpPlayback` path as four fixed-height MP4 formats, with the meta title
//! and OpenGraph thumbnail and description.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct PressTv;

const NAME: &str = "PressTV";
const BASE_URL: &str = "http://192.99.219.222:82/presstv";
const FORMATS: [(u32, &str); 4] = [
    (180, "_low200.mp4"),
    (360, "_low400.mp4"),
    (720, "_low800.mp4"),
    (1080, ".mp4"),
];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?presstv\.ir/[^/]+/(?P<y>\d+)/(?P<m>\d+)/(?P<d>\d+)/(?P<id>\d+)/(?P<display_id>[^/]+)?",
    )
    .unwrap()
});
static INPUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(<input[^>]+>)").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `re.sub(r'<!--(?:(?!<!--).)*-->', '', html)`: each comment start runs
/// greedily to the last `-->` before a newline or the next `<!--`.
fn strip_comments(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<!--") {
        let body = &rest[start + 4..];
        let limit = [body.find('\n'), body.find("<!--")]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(body.len());
        // `-->` may start at any content position up to the limit.
        let window = &body[..(limit + 3).min(body.len())];
        match window.rfind("-->").filter(|&end| end <= limit) {
            Some(end) => {
                out.push_str(&rest[..start]);
                rest = &body[end + 3..];
            }
            None => {
                out.push_str(&rest[..start + 1]);
                rest = &rest[start + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `InfoExtractor._hidden_inputs(html)`: the dict's items in insertion
/// order.
pub(crate) fn hidden_inputs(html: &str) -> Vec<(String, String)> {
    let html = strip_comments(html);
    let mut inputs: Vec<(String, String)> = Vec::new();
    for found in INPUT.captures_iter(&html) {
        let attributes = crate::html_elements::extract_attributes(&found[1]);
        let kind = attributes.get("type").and_then(Value::as_str);
        if !matches!(kind, Some("hidden" | "submit")) {
            continue;
        }
        let truthy = |key: &str| {
            attributes
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
        };
        let name = truthy("name").or_else(|| truthy("id"));
        let value = attributes.get("value").and_then(Value::as_str);
        if let (Some(name), Some(value)) = (name, value) {
            match inputs.iter_mut().find(|(existing, _)| existing == name) {
                Some((_, slot)) => *slot = value.to_owned(),
                None => inputs.push((name.to_owned(), value.to_owned())),
            }
        }
    }
    inputs
}

/// `'%04d%02d%02d' % (int(y), int(m), int(d))`.
fn upload_date(year: &str, month: &str, day: &str) -> String {
    let int = |text: &str| {
        let trimmed = text.trim_start_matches('0');
        if trimmed.is_empty() { "0" } else { trimmed }.to_owned()
    };
    format!("{:0>4}{:0>2}{:0>2}", int(year), int(month), int(day))
}

impl Extractor for PressTv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        let display_id = found
            .name("display_id")
            .map_or_else(|| id.clone(), |value| value.as_str().to_owned());
        let date = upload_date(&found["y"], &found["m"], &found["d"]);
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let video_url = hidden_inputs(&webpage)
            .into_iter()
            .find_map(|(name, value)| (name == "inpPlayback").then_some(value))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "{id}: An extractor error has occurred. (caused by KeyError('inpPlayback')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // `video_url[:-4]`.
        let chars: Vec<char> = video_url.chars().collect();
        let stem: String = chars[..chars.len().saturating_sub(4)].iter().collect();
        let mut formats = Vec::new();
        for (height, extension) in FORMATS {
            let media = format!("{BASE_URL}{stem}{extension}");
            let mut format: VideoFormat = serde_json::from_value(json!({
                "url": media,
                "format_id": format!("{height}p"),
                "height": height,
                "ext": super::direct::determine_ext(&media)
                    .unwrap_or_else(|| "unknown_video".into()),
            }))
            .map_err(|error| Error::Reported(format!("[{NAME}] {display_id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "dict:url,format_id,height");
            formats.push(format);
        }
        let title = crate::html::html_search_meta(&webpage, "title").ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract title; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let title = title
            .strip_prefix("PressTV-")
            .map_or_else(|| title.clone(), str::to_owned);
        let og = |prop: &str, name: &str| {
            let value = crate::html::og_search_property(&webpage, prop);
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let thumbnail = og("image", "thumbnail URL");
        let description = og("description", "OpenGraph description");
        let mut info = super::direct::build_at(url.clone(), url, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.thumbnail = thumbnail;
        info.metadata.description = description;
        info.extra.insert("display_id".into(), display_id.into());
        info.metadata.upload_date = Some(date);
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,display_id,title,formats,thumbnail,upload_date,description",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.presstv.ir/Detail/2016/04/09/459911/Australian-sewerage-treatment-facility-/"
        ));
        assert!(suitable("https://presstv.ir/Detail/2016/04/09/459911/"));
        assert!(!suitable("https://www.presstv.ir/Detail/2016/04/09/"));
    }

    #[test]
    fn comments_and_dates_follow_upstream() {
        assert_eq!(strip_comments("a<!-- x --> b --> c"), "a c");
        assert_eq!(strip_comments("a<!-- x\n-->b"), "a<!-- x\n-->b");
        assert_eq!(strip_comments("a<!-- x <!-- y -->b"), "a<!-- x b");
        assert_eq!(upload_date("2016", "4", "009"), "20160409");
    }
}
