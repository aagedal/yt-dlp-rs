//! Upstream `XVideosIE` and `XVideosQuickiesIE` (`yt_dlp/extractor/xvideos.py`):
//! the video page's player calls (title, thumbnails, low/high URLs and a
//! checked HLS playlist), the legacy `flv_url`, and quickies links as `url`
//! results for the video page.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct XVideos;
pub struct XVideosQuickies;

const NAME: &str = "XVideos";
const QUICKIES_NAME: &str = "XVideosQuickies";
const QUICKIES_IE_NAME: &str = "xvideos:quickies";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:[^/]+\.)?xvideos2?\.com/video\.?|(?:www\.)?xvideos\.es/video\.?|(?:www|flashservice)\.xvideos\.com/embedframe/|static-hw\.xvideos\.com/swf/xv-player\.swf\?.*?\bid_video=)(?P<id>[0-9a-z]+)",
    )
    .unwrap()
});
static QUICKIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?P<domain>(?:[^/?#]+\.)?xvideos2?\.com)/(?:profiles/|amateur-channels/)?[^/?#]+#quickies/a/(?P<id>\w+)").unwrap()
});
static INLINE_ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<h1 class="inlineError">(.+?)</h1>"#).unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<title>(.+?)\s+-\s+XVID").unwrap());
// `setVideoTitle\s*\(\s*(["\'])(?P<title>(?:(?!\1).)+)\1`.
static SCRIPT_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"setVideoTitle\s*\(\s*(?:"([^"\n]+)"|'([^'\n]+)')"#).unwrap());
static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<span[^>]+class=["']duration["'][^>]*>.*?(\d[^<]+)"#).unwrap());
static FLV_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"flv_url=(.+?)&").unwrap());
// `setVideo([^(]+)\((["\'])(http.+?)\2\)`.
static VIDEO_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"setVideo([^(]+)\((?:"(http.+?)"\)|'(http.+?)'\))"#).unwrap());

/// `XVideosIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `XVideosQuickiesIE._match_valid_url`.
pub fn quickies_suitable(url: &str) -> bool {
    QUICKIES_URL.is_match(url)
}

/// `setThumbUrl{suffix}\(\s*(["\'])(?P<thumbnail>(?:(?!\1).)+)\1`.
fn thumbnail_url(webpage: &str, suffix: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r#"setThumbUrl{suffix}\(\s*(?:"([^"\n]+)"|'([^'\n]+)')"#
    ))
    .expect("static suffixes");
    let found = pattern.captures(webpage)?;
    Some(found.get(1).or_else(|| found.get(2))?.as_str().to_owned())
}

/// `_html_search_regex`'s cleaning.
fn cleaned(text: &str) -> String {
    crate::html::clean_html(text).trim().to_owned()
}

impl Extractor for XVideos {
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
        if let Some(error) = INLINE_ERROR.captures(&webpage) {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: {NAME} said: {}",
                crate::html::clean_html(&error[1])
            )));
        }
        let title = TITLE
            .captures(&webpage)
            .map(|found| cleaned(&found[1]))
            .or_else(|| {
                SCRIPT_TITLE.captures(&webpage).and_then(|found| {
                    found
                        .get(1)
                        .or_else(|| found.get(2))
                        .map(|title| cleaned(title.as_str()))
                })
            })
            .filter(|title| !title.is_empty())
            .or_else(|| {
                // `_og_search_title` is not fatal.
                let title = crate::html::og_search_property(&webpage, "title");
                if title.is_none() {
                    super::search_warning(NAME, "OpenGraph title");
                }
                title
            });
        let mut thumbnails = Vec::new();
        for (preference, suffix) in ["", "169"].into_iter().enumerate() {
            if let Some(thumbnail) = thumbnail_url(&webpage, suffix) {
                thumbnails.push(json!({"url": thumbnail, "preference": preference}));
            }
        }
        let og_duration = crate::html::og_search_property(&webpage, "duration")
            .and_then(|duration| {
                super::streamable::int_or_none(Some(&Value::from(duration))).as_i64()
            })
            .filter(|duration| *duration != 0);
        let duration = match og_duration {
            Some(duration) => Value::from(duration),
            None => match DURATION.captures(&webpage) {
                Some(found) => crate::parse_duration(&found[1]).map_or(Value::Null, |duration| {
                    serde_json::to_value(duration).unwrap_or(Value::Null)
                }),
                None => {
                    super::search_warning(NAME, "duration");
                    Value::Null
                }
            },
        };
        let mut formats = Vec::new();
        let flv = FLV_URL
            .captures(&webpage)
            .map(|found| {
                percent_encoding::percent_decode_str(&found[1])
                    .decode_utf8_lossy()
                    .into_owned()
            })
            .unwrap_or_default();
        if !flv.is_empty() {
            formats.push(dict_format(
                vec![("url", flv.into()), ("format_id", "flv".into())],
                NAME,
                &video_id,
            )?);
        }
        for call in VIDEO_CALL.captures_iter(&webpage) {
            let format_id = call[1].to_lowercase();
            let Some(format_url) = call.get(2).or_else(|| call.get(3)) else {
                continue;
            };
            let format_url = format_url.as_str();
            if format_id == "hls" {
                let manifest = Url::parse(format_url)?;
                if let Some(stream) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &video_id,
                    Some("hls"),
                    false,
                    false,
                ) {
                    let mut hls = stream.formats;
                    super::check_formats(client, &mut hls, NAME, &video_id)?;
                    formats.extend(hls);
                }
            } else if format_id == "urllow" || format_id == "urlhigh" {
                let ext = super::direct::determine_ext(format_url).unwrap_or_else(|| "mp4".into());
                formats.push(dict_format(
                    vec![
                        ("url", format_url.into()),
                        ("format_id", format!("{ext}-{}", &format_id[3..]).into()),
                        (
                            "quality",
                            if format_id.ends_with("low") {
                                Value::from(-2)
                            } else {
                                Value::Null
                            },
                        ),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("formats", Value::Null),
                ("title", title.map_or(Value::Null, Value::from)),
                ("duration", duration),
                ("thumbnails", Value::Array(thumbnails)),
                ("age_limit", 18.into()),
            ],
            formats,
        );
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=url,preference");
        }
        Ok(Some(info))
    }
}

impl Extractor for XVideosQuickies {
    fn key(&self) -> &'static str {
        QUICKIES_NAME
    }

    fn extract(&self, _client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = QUICKIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(QUICKIES_IE_NAME);
        crate::screen::screen(&format!(
            "[{QUICKIES_IE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let id = &found["id"];
        let separator = if id.chars().all(|c| c.is_ascii_digit()) {
            ""
        } else {
            "."
        };
        let target = Url::parse(&format!(
            "https://{}/video{separator}{id}/_",
            &found["domain"]
        ))?;
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow
            .extra
            .insert(crate::FOLLOW_URL.into(), target.as_str().into());
        Ok(Some(follow))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.xvideos.com/video4588838/biker_takes_his_girl"
        ));
        assert!(suitable("https://www.xvideos.com/video.ufkkd1d1/_"));
        assert!(suitable(
            "https://flashservice.xvideos.com/embedframe/4588838"
        ));
        assert!(suitable(
            "https://static-hw.xvideos.com/swf/xv-player.swf?id_video=4588838"
        ));
        assert!(suitable("https://xvideos.es/video4588838/x"));
        assert!(!suitable("https://www.xvideos.com/profiles/someone"));
        assert!(quickies_suitable(
            "https://www.xvideos.com/profiles/someone#quickies/a/ufkkd1d1"
        ));
        assert!(quickies_suitable(
            "https://www.xvideos.com/amateur-channels/wifeluna#quickies/a/47258683"
        ));
        assert!(!quickies_suitable(
            "https://www.xvideos.com/profiles/someone"
        ));
    }
}
