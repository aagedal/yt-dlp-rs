//! Upstream `UnistraIE` (`yt_dlp/extractor/unistra.py`): a UTV page's
//! player `file` paths as SD/HD formats on the university's VOD host, with
//! the page's title, description and thumbnail.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Unistra;

const NAME: &str = "Unistra";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://utv\.unistra\.fr/(?:index|video)\.php\?id_video=(?P<id>\d+)",
    )
    .unwrap()
});
static FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"file\s*:\s*"(/[^"]+)""#).unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<title>UTV - (.*?)</").unwrap());
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<meta name="Description" content="(.*?)""#).unwrap());
static THUMBNAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"image: "(.*?)""#).unwrap());

/// `qualities(['SD', 'HD'])`.
const QUALITIES: [&str; 2] = ["SD", "HD"];

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Unistra {
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
        let webpage = download_webpage(client, url, NAME, &id)?;
        // `set(re.findall(...))`: the iteration order of a set is
        // unspecified, so the formats are kept in page order here (the
        // format sort orders SD and HD the same either way).
        let mut files: Vec<&str> = Vec::new();
        for found in FILE.captures_iter(&webpage) {
            let file = found.get(1).map_or("", |file| file.as_str());
            if !files.contains(&file) {
                files.push(file);
            }
        }
        let mut formats = Vec::new();
        for file in files {
            let format_id = if file.ends_with("-HD.mp4") {
                "HD"
            } else {
                "SD"
            };
            let quality = QUALITIES
                .iter()
                .position(|quality| *quality == format_id)
                .map_or(-1, |index| index as i64);
            let media = Url::parse(&format!("http://vod-flash.u-strasbg.fr:8080{file}"))?;
            let ext = super::direct::determine_ext(media.as_str())
                .unwrap_or_else(|| "unknown_video".into());
            let mut format: VideoFormat = serde_json::from_value(json!({
                "url": media.as_str(),
                "format_id": format_id,
                "quality": quality,
                "ext": ext,
            }))
            .map_err(|error| Error::Reported(format!("[{NAME}] {id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "dict:url,format_id,quality");
            crate::info_order::mark_int_quality(&mut format);
            formats.push(format);
        }
        let fatal = |pattern: &Regex, name: &str| {
            pattern
                .captures(&webpage)
                .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] {id}: Unable to extract {name}; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })
        };
        let title = fatal(&TITLE, "title")?;
        let description = fatal(&DESCRIPTION, "description")?;
        // `_search_regex` (not cleaned).
        let thumbnail = THUMBNAIL
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract thumbnail; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let no_formats = formats.is_empty();
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        if no_formats {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.metadata.description = Some(description);
        info.metadata.thumbnail = Some(thumbnail);
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,description,thumbnail,formats",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://utv.unistra.fr/video.php?id_video=154"));
        assert!(suitable("http://utv.unistra.fr/index.php?id_video=437"));
        assert!(!suitable("http://utv.unistra.fr/video.php?id_video=abc"));
    }
}
