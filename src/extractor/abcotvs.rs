//! Upstream `ABCOTVSIE` and `ABCOTVSClipsIE` (`yt_dlp/extractor/abcotvs.py`):
//! an ABC owned station's story from the content API (its HLS master, which
//! is optional, and its MP4 rendition), and an ABC clip from the clips API's
//! HLS master.
use super::{
    Extractor, dict_format, dict_info, download_json, download_json_note, update_url_query,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct AbcOtvs;
pub struct AbcOtvsClips;

const NAME: &str = "abcotvs";
const KEY: &str = "ABCOTVS";
const CLIPS_NAME: &str = "abcotvs:clips";
const CLIPS_KEY: &str = "ABCOTVSClips";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?P<site>abc(?:7(?:news|ny|chicago)?|11|13|30)|6abc)\.com(?:(?:/[^/]+)*/(?P<display_id>[^/]+))?/(?P<id>\d+)",
    )
    .unwrap()
});
static CLIPS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://clips\.abcotvs\.com/(?:[^/]+/)*video/(?P<id>\d+)").unwrap()
});

/// The station code for each site (`_SITE_MAP`).
fn station(site: &str) -> Option<&'static str> {
    Some(match site {
        "6abc" => "wpvi",
        "abc11" => "wtvd",
        "abc13" => "ktrk",
        "abc30" => "kfsn",
        "abc7" => "kabc",
        "abc7chicago" => "wls",
        "abc7news" => "kgo",
        "abc7ny" => "wabc",
        _ => return None,
    })
}

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `ABCOTVSClipsIE`'s `_VALID_URL`.
pub fn clips_suitable(url: &str) -> bool {
    CLIPS_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `video['m3u8'].split('?')[0]`.
fn without_query(url: &str) -> Result<Url> {
    let bare = url.split('?').next().unwrap_or_default();
    Ok(Url::parse(bare)?)
}

/// Python's `str()` of an ID.
fn id_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `dict_get(d, keys)`: the first truthy value.
fn dict_get<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find(|value| super::nuxt::truthy(value))
}

impl Extractor for AbcOtvs {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = found["id"].to_owned();
        let display_id = found
            .name("display_id")
            .map_or_else(|| video_id.clone(), |display| display.as_str().to_owned());
        let site = &found["site"];
        let station = station(site).ok_or_else(|| key_error(&video_id, "site"))?;

        let key = format!("otv.web.{station}.story");
        let api = update_url_query(
            &Url::parse("https://api.abcotvs.com/v2/content")?,
            &[
                ("id", video_id.as_str()),
                ("key", key.as_str()),
                ("station", station),
            ],
        );
        let data = download_json(client, &api, NAME, &display_id)?;
        // Upstream's KeyErrors name the URL's ID (`get_temp_id`), not the display ID.
        let data = data
            .get("data")
            .ok_or_else(|| key_error(&video_id, "data"))?;
        // `try_get(data, lambda x: x['featuredMedia']['video'], dict) or data`.
        let video = data
            .pointer("/featuredMedia/video")
            .filter(|video| video.is_object() && super::nuxt::truthy(video))
            .unwrap_or(data);
        let video_id = dict_get(video, &["id", "publishedKey"])
            .and_then(id_text)
            .unwrap_or(video_id);
        let title = match video
            .get("title")
            .filter(|title| super::nuxt::truthy(title))
        {
            Some(title) => title.clone(),
            None => video
                .get("linkText")
                .ok_or_else(|| key_error(&video_id, "linkText"))?
                .clone(),
        };

        let mut formats = Vec::new();
        if let Some(m3u8) = video
            .get("m3u8")
            .filter(|m3u8| super::nuxt::truthy(m3u8))
            .and_then(Value::as_str)
        {
            let manifest = without_query(m3u8)?;
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &display_id,
                Some("hls"),
                false,
                false,
            ) {
                for mut format in stream.formats {
                    format.ext = "mp4".into();
                    formats.push(format);
                }
            }
        }
        if let Some(mp4) = video
            .get("mp4")
            .filter(|mp4| super::nuxt::truthy(mp4))
            .and_then(Value::as_str)
        {
            let mut format = dict_format(
                vec![
                    ("abr", 128.into()),
                    ("format_id", "https".into()),
                    ("height", 360.into()),
                    ("url", mp4.into()),
                    ("width", 640.into()),
                ],
                NAME,
                &display_id,
            )?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }

        let image = video
            .get("image")
            .filter(|image| super::nuxt::truthy(image))
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        let description = dict_get(video, &["description", "caption"])
            .or_else(|| video.pointer("/meta/description"))
            .cloned()
            .unwrap_or(Value::Null);
        let thumbnail = dict_get(&image, &["source", "dynamicSource"])
            .cloned()
            .unwrap_or(Value::Null);

        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("display_id", display_id.into()),
                ("title", title),
                ("description", description),
                ("thumbnail", thumbnail),
                (
                    "timestamp",
                    super::streamable::int_or_none(video.get("date")),
                ),
                (
                    "duration",
                    super::streamable::int_or_none(video.get("length")),
                ),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

impl Extractor for AbcOtvsClips {
    fn key(&self) -> &'static str {
        CLIPS_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = CLIPS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(CLIPS_NAME);
        crate::screen::screen(&format!(
            "[{CLIPS_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = found["id"].to_owned();
        let api = Url::parse(&format!(
            "https://clips.abcotvs.com/vogo/video/getByIds?ids={video_id}"
        ))?;
        let response = download_json_note(
            client,
            &api,
            CLIPS_NAME,
            &video_id,
            "Downloading JSON metadata",
        )?;
        let video_data = response
            .get("results")
            .and_then(Value::as_array)
            .and_then(|results| results.first())
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let title = video_data
            .get("title")
            .ok_or_else(|| key_error(&video_id, "title"))?
            .clone();
        let video_url = video_data
            .get("videoURL")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&video_id, "videoURL"))?;
        let manifest = without_query(video_url)?;
        let stream = super::extract_m3u8_fatal_master(
            client, &manifest, CLIPS_NAME, &video_id, None, false,
        )?;
        let mut formats = stream.formats;
        for format in &mut formats {
            format.ext = "mp4".into();
        }

        Ok(Some(dict_info(
            url,
            (CLIPS_NAME, CLIPS_KEY),
            vec![
                ("id", video_id.clone().into()),
                ("title", title),
                (
                    "description",
                    video_data
                        .get("description")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "thumbnail",
                    video_data
                        .get("thumbnailURL")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "duration",
                    super::streamable::int_or_none(video_data.get("duration")),
                ),
                (
                    "timestamp",
                    super::streamable::int_or_none(video_data.get("pubDate")),
                ),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert!(suitable(
            "http://abc7news.com/entertainment/east-bay-museum-celebrates-vintage-synthesizers/472581/"
        ));
        assert!(suitable("http://abc7news.com/472581"));
        assert!(suitable(
            "https://6abc.com/man-75-killed-after-being-struck-by-vehicle-in-chester/5725182/"
        ));
        assert!(!suitable("https://abc.com/shows/some-show"));
        assert!(clips_suitable(
            "https://clips.abcotvs.com/kabc/video/214814"
        ));
        assert!(!clips_suitable("https://clips.abcotvs.com/kabc/"));
    }
}
