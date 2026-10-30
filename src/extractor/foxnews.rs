//! Upstream `FoxNewsIE`, `FoxNewsVideoIE` and `FoxNewsArticleIE`
//! (`yt_dlp/extractor/foxnews.py`). `FoxNewsIE` reads the Akamai AMP feed
//! (upstream `amp.py`'s `AMPIE._extract_feed_info`, ported here for this one
//! caller): thumbnails, the feed's subtitles, and its media renditions (HLS
//! manifests, HDS and direct files). The video and article pages hand off to
//! `FoxNewsIE` as followed URL results.
use super::{
    Extractor, dict_format, dict_info, direct, download_json_transformed, download_webpage,
};
use crate::{Error, ExtractResult, Result, SubtitleFormat, Subtitles, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct FoxNews;
pub struct FoxNewsVideo;
pub struct FoxNewsArticle;

const NAME: &str = "foxnews";
const VIDEO_NAME: &str = "FoxNewsVideo";
const ARTICLE_NAME: &str = "foxnews:article";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://video\.(?:insider\.)?fox(?:news|business)\.com/v/(?:video-embed\.html\?video_id=)?(?P<id>\d+)")
        .unwrap()
});
static VIDEO_PAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?foxnews\.com/video/(?P<id>\d+)").unwrap());
/// `(?!v)` is checked by hand: the first path segment must not start with `v`.
static ARTICLE_PAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:insider\.)?foxnews\.com/(?P<path>(?:[^/]+/)+)(?P<id>[a-z-]+)",
    )
    .unwrap()
});
static DATA_VIDEO_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-video-id=(?:'|")([^'"]+)"#).unwrap());
static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<(?:script|(?:amp-)?iframe)[^>]+\bsrc=["'](?:https?:)?//video\.foxnews\.com/v/(?:video-embed\.html|embed\.js)\?(?:[^>"']+&)?(?:video_)?id=(?P<video_id>\d+)"#,
    )
    .unwrap()
});

/// `FoxNewsIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `FoxNewsVideoIE.suitable`.
pub fn video_suitable(url: &str) -> bool {
    VIDEO_PAGE.is_match(url)
}

/// `FoxNewsArticleIE.suitable`.
pub fn article_suitable(url: &str) -> bool {
    article_id(url).is_some()
}

fn article_id(url: &str) -> Option<String> {
    let found = ARTICLE_PAGE.captures(url)?;
    if found["path"].starts_with('v') {
        return None;
    }
    Some(found["id"].to_owned())
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// `get_media_node(name)` of `AMPIE._extract_feed_info`.
fn media_node<'a>(item: &'a Value, name: &str) -> Option<&'a Value> {
    let media_name = format!("media-{name}");
    let media_group = item
        .get("media-group")
        .filter(|group| truthy(group))
        .unwrap_or(item);
    media_group
        .get(&media_name)
        .filter(|value| truthy(value))
        .or_else(|| item.get(&media_name).filter(|value| truthy(value)))
        .or_else(|| item.get(name))
}

/// A node as a list: a single object is one entry.
fn as_list(node: Option<&Value>) -> Vec<Value> {
    match node {
        Some(Value::Object(object)) => vec![Value::Object(object.clone())],
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// `@attributes` of a feed node (an empty object when absent).
fn attributes(node: &Value) -> Value {
    node.get("@attributes")
        .cloned()
        .unwrap_or_else(|| json!({}))
}

/// `subtitles.setdefault(lang, []).append(track)`.
fn add_subtitle(subtitles: &mut Subtitles, lang: String, track: SubtitleFormat) {
    let lang = Some(lang);
    match subtitles.entries.iter_mut().find(|(key, _)| *key == lang) {
        Some((_, tracks)) => tracks.push(track),
        None => subtitles.entries.push((lang, vec![track])),
    }
}

/// `AMPIE._extract_feed_info(url)` for `FoxNewsIE`: the feed's info under
/// the URL's `video_id`.
fn feed_info(client: &Client, url: &Url, video_id: &str) -> Result<VideoInfo> {
    let feed_url = Url::parse(&format!(
        "https://api.foxnews.com/v3/video-player/{video_id}?callback=uid_{video_id}"
    ))?;
    let feed = download_json_transformed(
        client,
        &feed_url,
        NAME,
        video_id,
        "Downloading Akamai AMP feed",
        &|text: &str| Ok(super::weibo::strip_jsonp(text)),
    )?;
    let item = feed
        .get("channel")
        .and_then(|channel| channel.get("item"))
        .filter(|item| truthy(item));
    let Some(item) = item else {
        let error = feed
            .get("error")
            .ok_or_else(|| key_error(video_id, "error"))?;
        let said = error
            .as_str()
            .map_or_else(|| error.to_string(), str::to_owned);
        return Err(Error::Reported(format!(
            "[{NAME}] {video_id}: {NAME} said: {said}; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let guid = item
        .get("guid")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(video_id, "guid"))?
        .to_owned();

    let mut thumbnails = Vec::new();
    for thumbnail_data in as_list(media_node(item, "thumbnail")) {
        let thumbnail = attributes(&thumbnail_data);
        let Some(thumbnail_url) = thumbnail
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        thumbnails.push(json!({
            "url": super::proto_relative_http(&thumbnail_url),
            "width": int_or_none(thumbnail.get("width")),
            "height": int_or_none(thumbnail.get("height")),
        }));
    }

    let mut subtitles = Subtitles::default();
    for subtitle_data in as_list(media_node(item, "subTitle")) {
        let subtitle = attributes(&subtitle_data);
        let Some(href) = subtitle
            .get("href")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let lang = subtitle
            .get("lang")
            .and_then(Value::as_str)
            .filter(|lang| !lang.is_empty())
            .unwrap_or("en")
            .to_owned();
        let ext = super::vidyard::mimetype2ext(subtitle.get("type"))
            .as_str()
            .map(str::to_owned)
            .or_else(|| direct::determine_ext(&href))
            .unwrap_or_default();
        add_subtitle(
            &mut subtitles,
            lang,
            SubtitleFormat {
                name: None,
                url: Url::parse(&href)?,
                ext,
                protocol: None,
                fragments: Vec::new(),
                extra: Map::new(),
            },
        );
    }

    let contents = as_list(media_node(item, "content"));
    if contents.is_empty() {
        return Err(Error::UnsupportedHtml5(
            "Fox News feeds without media content are not supported natively".into(),
        ));
    }
    let mut formats: Vec<VideoFormat> = Vec::new();
    for media_data in &contents {
        let media = attributes(media_data);
        let Some(media_url) = media
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let mime = super::vidyard::mimetype2ext(media.get("type"));
        let ext = mime
            .as_str()
            .map(str::to_owned)
            .or_else(|| direct::determine_ext(&media_url))
            .unwrap_or_default();
        let manifest = Url::parse(&media_url)?;
        match ext.as_str() {
            "f4m" => {
                let f4m = Url::parse(&format!(
                    "{media_url}?hdcore=3.4.0&plugin=aasp-3.4.0.132.124"
                ))?;
                formats.extend(super::extract_f4m_optional(
                    client,
                    NAME,
                    &f4m,
                    &guid,
                    Some("hds"),
                )?);
            }
            "m3u8" => {
                // `_extract_m3u8_formats_and_subtitles(..., fatal=False)`.
                if let Some(info) = super::extract_m3u8_master_optional(
                    client,
                    &manifest,
                    NAME,
                    &guid,
                    Some("hls"),
                    true,
                    false,
                ) {
                    formats.extend(info.formats);
                    super::html5::merge_subtitles(&mut subtitles, info.subtitles);
                }
            }
            _ => {
                let label = media_data
                    .get("media-category")
                    .and_then(|category| category.get("@attributes"))
                    .and_then(|attributes| attributes.get("label"))
                    .cloned()
                    .unwrap_or(Value::Null);
                formats.push(dict_format(
                    vec![
                        ("format_id", label),
                        ("url", media_url.clone().into()),
                        (
                            "tbr",
                            int_or_none(media.get("bitrate")).map_or(Value::Null, Value::from),
                        ),
                        (
                            "filesize",
                            int_or_none(media.get("fileSize")).map_or(Value::Null, Value::from),
                        ),
                        ("ext", ext.into()),
                    ],
                    NAME,
                    &guid,
                )?);
            }
        }
    }

    let timestamp = item
        .get("pubDate")
        .and_then(Value::as_str)
        .and_then(|date| crate::dates::unified_timestamp(date, false))
        .or_else(|| {
            item.get("dc-date")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
        });
    let duration = int_or_none(attributes(&contents[0]).get("duration"));
    let mut info = dict_info(
        url,
        (NAME, "FoxNews"),
        vec![
            ("id", video_id.into()),
            (
                "title",
                media_node(item, "title").cloned().unwrap_or(Value::Null),
            ),
            (
                "description",
                media_node(item, "description")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            ("thumbnails", Value::Array(thumbnails)),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
            ("duration", duration.map_or(Value::Null, Value::from)),
            ("subtitles", Value::Null),
            ("formats", Value::Null),
        ],
        formats,
    );
    if let Some(Value::String(shape)) = info.extra.get(crate::info_order::INFO_SHAPE).cloned() {
        info.extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("{shape};thumbnail=url,width,height").into(),
        );
    }
    info.subtitles = subtitles;
    Ok(info)
}

/// `url_result(target, FoxNewsIE)`, followed by the downloader.
fn follow_url_result(url: &Url, target: &str) -> ExtractResult {
    let mut follow = direct::build_at(url.clone(), url, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
    ExtractResult::Video(follow)
}

/// The video id of a `video.foxnews.com/v/…` URL.
fn video_id_of(url: &Url) -> Option<String> {
    VALID_URL
        .captures(url.as_str())
        .map(|found| found["id"].to_owned())
}

impl Extractor for FoxNews {
    fn key(&self) -> &'static str {
        "FoxNews"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(video_id) = video_id_of(url) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        Ok(Some(feed_info(client, url, &video_id)?))
    }
}

impl Extractor for FoxNewsVideo {
    fn key(&self) -> &'static str {
        VIDEO_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VIDEO_PAGE.captures(url.as_str()) else {
            return Ok(None);
        };
        let target = format!("https://video.foxnews.com/v/{}", &found["id"]);
        Ok(Some(follow_url_result(url, &target)))
    }
}

impl Extractor for FoxNewsArticle {
    fn key(&self) -> &'static str {
        "FoxNewsArticle"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(display_id) = article_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(ARTICLE_NAME);
        crate::screen::screen(&format!(
            "[{ARTICLE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, ARTICLE_NAME, &display_id)?;
        if let Some(found) = DATA_VIDEO_ID.captures(&webpage) {
            let target = format!("http://video.foxnews.com/v/{}", &found[1]);
            return Ok(Some(follow_url_result(url, &target)));
        }
        let embed = EMBED_URL.captures(&webpage).ok_or_else(|| {
            Error::UnsupportedHtml5(
                "Fox News articles without a video ID or embed are not supported natively".into(),
            )
        })?;
        let target = format!(
            "https://video.foxnews.com/v/video-embed.html?video_id={}",
            &embed["video_id"]
        );
        Ok(Some(follow_url_result(url, &target)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://video.foxnews.com/v/6320653836112"));
        assert!(suitable(
            "http://video.insider.foxnews.com/v/video-embed.html?video_id=5099377331001&autoplay=true"
        ));
        assert!(suitable("http://video.foxbusiness.com/v/4442309889001"));
        assert!(video_suitable(
            "https://www.foxnews.com/video/6328632286112"
        ));
        assert!(article_suitable(
            "https://www.foxnews.com/politics/2016/09/08/buzz-about-bud-clinton-camp-denies-claims-wore-earpiece-at-forum.html"
        ));
        assert!(!article_suitable(
            "https://www.foxnews.com/video/6328632286112"
        ));
        assert!(!article_suitable(
            "https://video.foxnews.com/v/6320653836112"
        ));
    }
}
