//! Upstream `HRFernsehenIE` (`yt_dlp/extractor/hrfensehen.py`): the
//! hr-fernsehen.de and hessenschau.de video page's `data-hr-mediaplayer-loader`
//! JSON, its progressive MP4 qualities and the optional German subtitle track.
use super::clyp::key_error;
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct HrFernsehen;

const NAME: &str = "hrfernsehen";
const KEY: &str = "HRFernsehen";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://www\.(?:hr-fernsehen|hessenschau)\.de/.*,video-(?P<id>[0-9]{6})\.html",
    )
    .unwrap()
});
static LOADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"data-(?:new-)?hr-mediaplayer-loader='([^']*)'").unwrap());
static QUALITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([0-9]{3,4})x([0-9]{3,4})-([0-9]{2})p-([0-9]{3,4})kbit").unwrap()
});
static DATETIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<time\sdatetime="(\d{4}\W\d{1,2}\W\d{1,2})"#).unwrap());
static THUMBNAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"thumbnailUrl\W*([^"]+)"#).unwrap());

/// `_match_id`-style suitability check.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("hrfernsehen {what} are not supported natively"))
}

/// `loader_data['mediaCollection']['streams'][0]['media']` with Python's
/// KeyError/IndexError behaviour for a missing or short path.
fn stream_media<'a>(loader: &'a Value, id: &str) -> Result<&'a Vec<Value>> {
    let streams = loader
        .get("mediaCollection")
        .and_then(|collection| collection.get("streams"))
        .ok_or_else(|| key_error(id, "streams"))?;
    let stream = streams
        .get(0)
        .ok_or_else(|| Error::Reported(format!("{id}: list index out of range")))?;
    stream
        .get("media")
        .ok_or_else(|| key_error(id, "media"))?
        .as_array()
        .ok_or_else(|| unsupported("media that are not lists"))
}

/// `extract_formats(loader_data)`: the stream's qualities after the first entry.
fn extract_formats(loader: &Value, id: &str) -> Result<Vec<crate::VideoFormat>> {
    let media = stream_media(loader, id)?;
    let mut formats = Vec::new();
    for inner in media.iter().skip(1) {
        let url = inner.get("url").ok_or_else(|| key_error(id, "url"))?;
        let max_height = inner
            .get("maxHResolutionPx")
            .cloned()
            .unwrap_or(Value::Null);
        // `try_call(lambda: f'{inner["maxHResolutionPx"]}p')`.
        let format_id = match &max_height {
            Value::Number(number) => Value::from(format!("{number}p")),
            _ => Value::Null,
        };
        let url_text = url
            .as_str()
            .ok_or_else(|| unsupported("URLs that are not strings"))?;
        let mut fields = vec![
            ("format_id", format_id),
            ("height", max_height),
            ("url", url.clone()),
        ];
        if let Some(found) = QUALITY.captures(url_text) {
            let number =
                |index: usize| found[index].parse::<i64>().map_or(Value::Null, Value::from);
            // `height` keeps its position and takes the URL's value.
            for (key, value) in fields.iter_mut() {
                if *key == "height" {
                    *value = number(2);
                }
            }
            fields.push(("width", number(1)));
            fields.push(("fps", number(3)));
            fields.push(("tbr", number(4)));
        }
        let mut format = dict_format(fields, NAME, id)?;
        // `int_or_none` quality values stay ints through processing.
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    Ok(formats)
}

impl Extractor for HrFernsehen {
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

        let meta = |props: &[&str]| {
            props
                .iter()
                .find_map(|prop| crate::html::html_search_meta(&webpage, prop))
        };
        let title = meta(&["og:title", "twitter:title", "name"]);
        let description = meta(&["description"]);

        let Some(loader) = LOADER.captures(&webpage) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract ardloader; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let loader_text = crate::html::unescape_html(&loader[1]);
        let loader_data: Value = serde_json::from_str(&loader_text)
            .map_err(|_| unsupported("player loaders that are not JSON"))?;

        let formats = extract_formats(&loader_data, &video_id)?;

        let subtitle = loader_data
            .get("mediaCollection")
            .and_then(|collection| collection.get("subTitles"))
            .and_then(|tracks| tracks.get(0))
            .and_then(|track| track.get("sources"))
            .and_then(|sources| sources.get(0))
            .and_then(|source| source.get("url"))
            .and_then(Value::as_str);
        let mut subtitles = Subtitles::default();
        if let Some(subtitle) = subtitle {
            // Processing names an untyped track by its URL's extension.
            let ext =
                super::direct::determine_ext(subtitle).unwrap_or_else(|| "unknown_video".into());
            let track = crate::SubtitleFormat {
                name: None,
                url: Url::parse(subtitle)?,
                ext,
                protocol: None,
                fragments: Vec::new(),
                extra: serde_json::Map::new(),
            };
            subtitles.entries.push((Some("de".into()), vec![track]));
        }

        let date = DATETIME.captures(&webpage).map(|found| found[1].to_owned());
        let timestamp = date
            .and_then(|date| crate::dates::unified_timestamp(&date, true))
            .map_or(Value::Null, Value::from);
        let duration = super::streamable::int_or_none(
            loader_data
                .get("playerConfig")
                .and_then(|config| config.get("pluginData"))
                .and_then(|plugin| plugin.get("trackingAti@all"))
                .and_then(|tracking| tracking.get("richMedia"))
                .and_then(|rich| rich.get("duration")),
        );
        let thumbnail = THUMBNAIL
            .captures(&webpage)
            .map_or(Value::Null, |found| Value::from(found[1].to_owned()));

        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.clone().into()),
                ("title", title.map_or(Value::Null, Value::from)),
                ("description", description.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("timestamp", timestamp),
                ("duration", duration),
                ("thumbnail", thumbnail),
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
            "https://www.hessenschau.de/tv-sendung/hessenschau-vom-26082020,video-130546.html"
        ));
        assert!(suitable(
            "https://www.hr-fernsehen.de/sendungen-a-z/mex/sendungen/fair-und-gut---was-hinter-aldis-eigenem-guetesiegel-steckt,video-130544.html"
        ));
        assert!(!suitable(
            "https://www.hessenschau.de/tv-sendung/hessenschau,video-13054.html"
        ));
        assert!(!suitable("https://example.com/video-130546.html"));
    }

    #[test]
    fn quality_fields_come_from_the_url() {
        let loader: Value = serde_json::json!({
            "mediaCollection": {"streams": [{"media": [
                {"kind": "main"},
                {"url": "https://h/x_480x270-25p-250kbit.mp4", "maxHResolutionPx": 480}
            ]}]}
        });
        let formats = extract_formats(&loader, "1").unwrap();
        assert_eq!(formats.len(), 1);
        let json = serde_json::to_value(&formats[0]).unwrap();
        assert_eq!(json["height"], 270);
        assert_eq!(json["width"], 480);
        assert_eq!(json["fps"], 25);
        assert_eq!(json["tbr"].as_f64(), Some(250.0));
    }
}
