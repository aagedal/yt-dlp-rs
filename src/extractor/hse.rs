//! Upstream `HSEShowIE` and `HSEProductIE` (`yt_dlp/extractor/hse.py`): a
//! TV show's or product's `window.__REDUX_DATA__` and its HLS sources.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct HseShow;
pub struct HseProduct;

const SHOW: &str = "HSEShow";
const PRODUCT: &str = "HSEProduct";

static SHOW_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?hse\.de/dpl/c/tv-shows/(?P<id>[0-9]+)").unwrap()
});
static PRODUCT_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?hse\.de/dpl/p/product/(?P<id>[0-9]+)").unwrap()
});
static REDUX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__REDUX_DATA__\s*=\s*(\{.*\});?").unwrap());
static CHANNEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"tvShow \| ([A-Z0-9]+)_").unwrap());

/// `HSEShowIE._match_id`.
pub fn show_suitable(url: &str) -> bool {
    SHOW_URL.is_match(url)
}

/// `HSEProductIE._match_id`.
pub fn product_suitable(url: &str) -> bool {
    PRODUCT_URL.is_match(url)
}

/// `traverse_obj(value, path)` over dict keys and list indices.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .try_fold(value, |value, key| match value {
            Value::Array(items) => key.parse::<usize>().ok().and_then(|index| items.get(index)),
            _ => value.get(key),
        })
        .filter(|value| !value.is_null())
}

/// `_extract_redux_data`.
fn redux_data(client: &Client, url: &Url, extractor: &'static str, id: &str) -> Result<Value> {
    let webpage = download_webpage(client, url, extractor, id)?;
    // `_html_search_regex` (fatal).
    let redux = REDUX
        .captures(&webpage)
        .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{extractor}] {id}: Unable to extract redux data; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    crate::json::from_str(&redux.replace('\n', "")).map_err(|error| {
        Error::Reported(format!(
            "[{extractor}] {id}: {id}: Failed to parse JSON (caused by {error}); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })
}

/// `_extract_formats_and_subtitles`.
fn formats_and_subtitles(
    client: &Client,
    sources: Option<&Value>,
    extractor: &'static str,
    id: &str,
) -> Result<(Vec<crate::VideoFormat>, crate::Subtitles)> {
    let sources = match sources {
        Some(Value::Array(sources)) if !sources.is_empty() => sources,
        None => {
            return Err(Error::Reported(format!(
                "[{extractor}] {id}: No video found"
            )));
        }
        Some(Value::Array(_)) => {
            return Err(Error::Reported(format!(
                "[{extractor}] {id}: No video found"
            )));
        }
        Some(_) => {
            return Err(Error::UnsupportedHtml5(
                "HSE video sources that are not lists are not supported natively".into(),
            ));
        }
    };
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for source in sources {
        let (Some(mimetype), Some(manifest)) = (
            source.get("mimetype").and_then(Value::as_str),
            source.get("url").and_then(Value::as_str),
        ) else {
            return Err(Error::UnsupportedHtml5(
                "HSE video sources without a string mimetype and URL are not supported natively"
                    .into(),
            ));
        };
        if mimetype != "application/x-mpegURL" {
            continue;
        }
        let mut stream =
            extract_m3u8_fatal(client, &Url::parse(manifest)?, extractor, id, None, true)?;
        for format in &mut stream.formats {
            format.ext = "mp4".into();
        }
        formats.extend(stream.formats);
        super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
    }
    Ok((formats, subtitles))
}

fn start(url: &Url, pattern: &Regex, extractor: &'static str) -> Option<String> {
    let id = pattern.captures(url.as_str())?["id"].to_owned();
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    Some(id)
}

fn finish(
    url: &Url,
    extractor: &'static str,
    fields: Vec<(&str, Value)>,
    (formats, subtitles): (Vec<crate::VideoFormat>, crate::Subtitles),
) -> VideoInfo {
    let mut info = dict_info(url, (extractor, extractor), fields, formats);
    info.subtitles = subtitles;
    info
}

fn or_id(value: Option<&Value>, id: &str) -> Value {
    match value {
        Some(value) if super::lnk::truthy(Some(value)) => value.clone(),
        _ => id.into(),
    }
}

fn cloned(value: Option<&Value>) -> Value {
    value.cloned().unwrap_or(Value::Null)
}

impl Extractor for HseShow {
    fn key(&self) -> &'static str {
        SHOW
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(id) = start(url, &SHOW_URL, SHOW) else {
            return Ok(None);
        };
        let data = redux_data(client, url, SHOW, &id)?;
        let streams = formats_and_subtitles(
            client,
            path(&data, &["tvShowPage", "tvShowVideo", "sources"]),
            SHOW,
            &id,
        )?;
        let empty = Value::Object(Default::default());
        let show = path(&data, &["tvShowPage", "tvShow"])
            .filter(|show| super::lnk::truthy(Some(show)))
            .unwrap_or(&empty);
        let text = |value: Option<&Value>| match value {
            None | Some(Value::Null) => "None".to_owned(),
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
        };
        // `unified_timestamp(f'{show.get("date")} {show.get("hour")}:00')`.
        let timestamp = crate::dates::unified_timestamp(
            &format!("{} {}:00", text(show.get("date")), text(show.get("hour"))),
            true,
        )
        .map_or(Value::Null, Value::from);
        // `_search_regex(..., video_id, fatal=False)` names the field `ID`.
        let action = show
            .get("actionFieldText")
            .filter(|action| super::lnk::truthy(Some(action)))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let channel = match CHANNEL.captures(action) {
            Some(found) => Value::from(&found[1]),
            None => {
                super::search_warning(SHOW, &id);
                Value::Null
            }
        };
        Ok(Some(finish(
            url,
            SHOW,
            vec![
                ("id", id.clone().into()),
                ("title", or_id(show.get("title"), &id)),
                ("formats", Value::Null),
                ("timestamp", timestamp),
                ("thumbnail", cloned(path(&data, &["tvShowVideo", "poster"]))),
                ("channel", channel),
                ("uploader", cloned(show.get("presenter"))),
                ("subtitles", Value::Null),
            ],
            streams,
        )))
    }
}

impl Extractor for HseProduct {
    fn key(&self) -> &'static str {
        PRODUCT
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(id) = start(url, &PRODUCT_URL, PRODUCT) else {
            return Ok(None);
        };
        let data = redux_data(client, url, PRODUCT, &id)?;
        let empty = Value::Object(Default::default());
        let video = path(&data, &["productContent", "productContent", "videos", "0"])
            .filter(|video| super::lnk::truthy(Some(video)))
            .unwrap_or(&empty);
        let streams = formats_and_subtitles(client, video.get("sources"), PRODUCT, &id)?;
        Ok(Some(finish(
            url,
            PRODUCT,
            vec![
                ("id", id.clone().into()),
                (
                    "title",
                    or_id(
                        path(&data, &["productDetail", "product", "name", "short"]),
                        &id,
                    ),
                ),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("thumbnail", cloned(video.get("poster"))),
                (
                    "uploader",
                    cloned(path(
                        &data,
                        &["productDetail", "product", "brand", "brandName"],
                    )),
                ),
            ],
            streams,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(show_suitable("https://www.hse.de/dpl/c/tv-shows/505350"));
        assert!(product_suitable("https://www.hse.de/dpl/p/product/408630"));
        assert!(!show_suitable("https://www.hse.de/dpl/p/product/408630"));
        assert!(!product_suitable(
            "https://www.hse.de/dpl/c/tv-shows/505350"
        ));
    }
}
