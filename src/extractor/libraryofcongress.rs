//! Upstream `LibraryOfCongressIE` (`yt_dlp/extractor/libraryofcongress.py`):
//! loc.gov items' media player ID, the `media.loc.gov` media object's first
//! derivative (HLS and HTTP formats rewritten like the page's
//! `setAVSource`), the page's download options and the closed captions.
use super::{Extractor, dict_format, dict_info, download_json, download_webpage};
use crate::{Error, Result, SubtitleFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct LibraryOfCongress;

const NAME: &str = "loc";
const KEY: &str = "LibraryOfCongress";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?loc\.gov/(?:item/|today/cyberlc/feature_wdesc\.php\?.*\brec=)(?P<id>[0-9a-z_.]+)",
    )
    .unwrap()
});
/// The media ID patterns, in upstream's order (backreferences as two
/// quote alternatives).
static MEDIA_IDS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"id=(?:"media-player-(?P<dq>[^"\n]+?)"|'media-player-(?P<sq>[^'\n]+?)')"#,
        r#"<video[^>]+id=(?:"uuid-(?P<dq>[^"\n]+?)"|'uuid-(?P<sq>[^'\n]+?)')"#,
        r#"<video[^>]+data-uuid=(?:"(?P<dq>[^"\n]+?)"|'(?P<sq>[^'\n]+?)')"#,
        r#"mediaObjectId\s*:\s*(?:"(?P<dq>[^"\n]+?)"|'(?P<sq>[^'\n]+?)')"#,
        r#"data-tab="share-media-(?P<dq>[0-9A-F]{32})""#,
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});
static STREAM_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(://[^/]+/)(?:[^/]+/)*(?:mp4|mp3):").unwrap());
static DOWNLOAD_OPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<option[^>]+value=(?:"(?P<dq>[^"\n]+?)"|'(?P<sq>[^'\n]+?)')[^>]+data-file-download=[^>]+>\s*(?P<id>[^\n]+?)(?:(?:&nbsp;|\s+)\((?P<size>[^\n]+?)\))?\s*<"#,
    )
    .unwrap()
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
    Error::UnsupportedHtml5(format!(
        "Library of Congress {what} are not supported natively"
    ))
}

fn quoted<'a>(found: &regex::Captures<'a>) -> Option<&'a str> {
    found
        .name("dq")
        .or_else(|| found.name("sq"))
        .map(|value| value.as_str())
}

impl Extractor for LibraryOfCongress {
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
        let media_id = MEDIA_IDS
            .iter()
            .find_map(|regex| {
                regex
                    .captures(&webpage)
                    .and_then(|found| quoted(&found).map(str::to_owned))
            })
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract media id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let api = Url::parse(&format!(
            "https://media.loc.gov/services/v1/media?id={media_id}&context=json"
        ))?;
        let response = download_json(client, &api, NAME, &media_id)?;
        let data = response
            .get("mediaObject")
            .ok_or_else(|| key_error(&video_id, "mediaObject"))?;
        let derivative = match data.get("derivatives") {
            Some(Value::Array(items)) => items.first().ok_or_else(|| {
                // An uncaught `IndexError` is reported as its message.
                Error::Reported("list index out of range".into())
            })?,
            Some(_) => return Err(unsupported("derivative lists that are not lists")),
            None => return Err(key_error(&video_id, "derivatives")),
        };
        let mut media_url = match derivative.get("derivativeUrl") {
            Some(Value::String(url)) => url.clone(),
            Some(_) => return Err(unsupported("derivative URLs that are not strings")),
            None => return Err(key_error(&video_id, "derivativeUrl")),
        };
        let short_name = |value: &Value| {
            value
                .get("shortName")
                .filter(|name| super::nuxt::truthy(name))
                .cloned()
        };
        let title = match short_name(derivative).or_else(|| short_name(data)) {
            Some(title) => title,
            None => {
                let title = crate::html::og_search_property(&webpage, "title");
                if title.is_none() {
                    crate::screen::warning(&format!(
                        "[{NAME}] unable to extract OpenGraph title; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ));
                }
                title.map_or(Value::Null, Value::from)
            }
        };
        // `setAVSource`.
        media_url = media_url.replace("rtmp", "https");
        let is_video = data
            .get("mediaType")
            .and_then(Value::as_str)
            .unwrap_or("v")
            .to_lowercase()
            == "v";
        let ext = super::direct::determine_ext(&media_url);
        if !matches!(ext.as_deref(), Some("mp4" | "mp3")) {
            media_url.push_str(if is_video { ".mp4" } else { ".mp3" });
        }
        let mut formats = Vec::new();
        if media_url.contains("/vod/mp4:") {
            formats.push(dict_format(
                vec![
                    (
                        "url",
                        format!("{}.m3u8", media_url.replace("/vod/mp4:", "/hls-vod/media/"))
                            .into(),
                    ),
                    ("format_id", "hls".into()),
                    ("ext", "mp4".into()),
                    ("protocol", "m3u8_native".into()),
                    ("quality", 1.into()),
                ],
                NAME,
                &video_id,
            )?);
        }
        let mut http = vec![
            (
                "url",
                Value::from(STREAM_PREFIX.replace(&media_url, "${1}").into_owned()),
            ),
            ("format_id", "http".into()),
            ("quality", 1.into()),
        ];
        if !is_video {
            http.push(("vcodec", "none".into()));
        }
        formats.push(dict_format(http, NAME, &video_id)?);
        let mut seen: Vec<String> = Vec::new();
        for option in DOWNLOAD_OPTION.captures_iter(&webpage) {
            let format_id = option["id"].to_lowercase();
            if format_id == "gif" || format_id == "jpeg" {
                continue;
            }
            let Some(download_url) = quoted(&option) else {
                continue;
            };
            if seen.iter().any(|known| known == download_url) {
                continue;
            }
            seen.push(download_url.to_owned());
            let size = option
                .name("size")
                .and_then(|size| super::xboxclips::parse_filesize(size.as_str()));
            formats.push(dict_format(
                vec![
                    ("url", download_url.into()),
                    ("format_id", format_id.into()),
                    ("filesize_approx", size.map_or(Value::Null, Value::from)),
                ],
                NAME,
                &video_id,
            )?);
        }
        let duration = match data.get("duration") {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse().ok(),
            _ => None,
        };
        let view_count = match data.get("viewCount") {
            Some(Value::Number(number)) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
            Some(Value::String(text)) => text.trim().parse().ok(),
            _ => None,
        };
        let mut subtitles = crate::Subtitles::default();
        if let Some(cc_url) = data.get("ccUrl").filter(|value| super::nuxt::truthy(value)) {
            let cc_url = cc_url
                .as_str()
                .and_then(|cc_url| Url::parse(cc_url).ok())
                .ok_or_else(|| unsupported("caption URLs that are not URLs"))?;
            subtitles.entries.push((
                Some("en".into()),
                vec![SubtitleFormat {
                    name: None,
                    url: cc_url,
                    ext: "ttml".into(),
                    protocol: None,
                    fragments: Vec::new(),
                    extra: Map::new(),
                }],
            ));
        }
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", title),
                (
                    "thumbnail",
                    crate::html::og_search_property(&webpage, "image")
                        .map_or(Value::Null, Value::from),
                ),
                (
                    "duration",
                    duration.map_or(Value::Null, |duration| json!(duration)),
                ),
                ("view_count", view_count.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        info.subtitles = subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";subtitle=url,ext");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://loc.gov/item/90716351/"));
        assert!(suitable(
            "https://www.loc.gov/today/cyberlc/feature_wdesc.php?rec=5578"
        ));
        assert!(!suitable("https://www.loc.gov/collections/"));
    }

    #[test]
    fn streams_lose_their_application_prefix() {
        assert_eq!(
            STREAM_PREFIX.replace("https://media.loc.gov/vod/mp4:media/x.mp4", "${1}"),
            "https://media.loc.gov/media/x.mp4"
        );
    }
}
