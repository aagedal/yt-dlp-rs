//! Upstream `ToutiaoIE` (`yt_dlp/extractor/toutiao.py`): 今日头条 videos from
//! the page's `RENDER_DATA` script (`data.initialVideo`), with a `ttwid`
//! cookie fetched once per run from the ByteDance registration endpoint.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Toutiao;

const NAME: &str = "toutiao";
const KEY: &str = "Toutiao";
const REGISTER: &str = "https://ttwid.bytedance.com/ttwid/union/register/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://www\.toutiao\.com/video/(?P<id>\d+)/?(?:[?#]|$)")
        .unwrap()
});
static URL_OR_NONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:https?|rt(?:m(?:pt?[es]?|fp)|sp[su]?)|mms|ftps?):)?//").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `traverse_obj(data, key, {int_or_none})`: a missing or null value is absent.
fn present(value: Value) -> Option<Value> {
    (!value.is_null()).then_some(value)
}

/// `{url_or_none}` of a string.
fn url_or_none(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| URL_OR_NONE.is_match(text))
        .map(Value::from)
}

/// The `RENDER_DATA` script's `data.initialVideo` (percent-encoded JSON).
fn initial_video(webpage: &str) -> Result<Option<Value>> {
    let scripts =
        crate::html_elements::elements_text_and_html_by_attribute("id", "RENDER_DATA", webpage)
            .map_err(|_| Error::Reported(format!("[{NAME}] Unable to parse RENDER_DATA")))?;
    let Some((text, _)) = scripts.into_iter().next() else {
        return Ok(None);
    };
    let decoded = percent_encoding::percent_decode_str(&text).decode_utf8_lossy();
    let Ok(data) = serde_json::from_str::<Value>(&decoded) else {
        return Ok(None);
    };
    Ok(data
        .get("data")
        .and_then(|data| data.get("initialVideo"))
        .cloned())
}

/// `ttwid` for this run: an existing cookie, else the registration's.
fn fetch_ttwid(client: &Client) -> Result<()> {
    if crate::cookies::get_cookie("https://www.toutiao.com", "ttwid").is_some() {
        return Ok(());
    }
    let url = Url::parse(REGISTER)?;
    let body = serde_json::json!({
        "aid": 24,
        "needFid": false,
        "region": "cn",
        "service": "www.toutiao.com",
        "union": true,
    })
    .to_string();
    crate::screen::screen(&format!("[{NAME}] Fetching ttwid"));
    let response = super::direct::post(client, &url, &[("Content-Type", "application/json")], body)
        .map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] Unable to fetch ttwid: {}",
                crate::upstream_error_text(&error)
            ))
        })?;
    if let Some(ttwid) = crate::cookies::get_cookie(response.url().as_str(), "ttwid") {
        crate::cookies::set_cookie(".toutiao.com", "ttwid", &ttwid);
        return Ok(());
    }
    Err(Error::Reported(format!(
        "[{NAME}] This video is only available for registered users. {}",
        super::login::login_hint(None, Some(super::login::Hint::Cookies))
    )))
}

impl Extractor for Toutiao {
    fn key(&self) -> &'static str {
        KEY
    }

    /// `_real_initialize`: the `ttwid` cookie, once per run.
    fn real_initialize(&self, client: &Client, _session: &super::login::Session) -> Result<()> {
        fetch_ttwid(client)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        self.initialize(client, NAME, &video_id)?;
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let video_data = initial_video(&webpage)?;
        let video = |key: &str| video_data.as_ref().and_then(|data| data.get(key));
        let play_info = video("videoPlayInfo");
        let user = video("userInfo");

        let mut formats = Vec::new();
        let video_list = play_info
            .and_then(|info| info.get("video_list"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for entry in video_list {
            let Some(main_url) = entry
                .get("main_url")
                .and_then(Value::as_str)
                .filter(|main_url| !main_url.is_empty())
            else {
                continue;
            };
            let meta = entry.get("video_meta");
            let meta_value = |key: &str| meta.and_then(|meta| meta.get(key));
            let str_field = |key: &str| meta_value(key).and_then(Value::as_str).map(Value::from);
            let int_field = |key: &str| present(super::streamable::int_or_none(meta_value(key)));
            let channels = super::clyp::float_or_none(meta_value("audio_channels"))
                .as_f64()
                .map(|channels| Value::from(channels.trunc() as i64));
            let tbr = super::clyp::float_or_none(meta_value("real_bitrate"))
                .as_f64()
                .map(|bitrate| Value::from(bitrate / 1000.0));
            let fields = vec![
                ("url", Value::from(main_url)),
                ("acodec", str_field("audio_profile").unwrap_or(Value::Null)),
                ("asr", int_field("audio_sample_rate").unwrap_or(Value::Null)),
                ("audio_channels", channels.unwrap_or(Value::Null)),
                ("ext", str_field("vtype").unwrap_or(Value::Null)),
                ("filesize", int_field("size").unwrap_or(Value::Null)),
                ("format_id", str_field("definition").unwrap_or(Value::Null)),
                ("fps", int_field("fps").unwrap_or(Value::Null)),
                ("height", int_field("vheight").unwrap_or(Value::Null)),
                ("tbr", tbr.unwrap_or(Value::Null)),
                ("vcodec", str_field("codec_type").unwrap_or(Value::Null)),
                ("width", int_field("vwidth").unwrap_or(Value::Null)),
            ];
            // `traverse_obj` leaves out the keys whose values are missing.
            let fields = fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .collect();
            formats.push(dict_format(fields, NAME, &video_id)?);
        }

        // `traverse_obj` drops missing keys; `title` keeps its place as null
        // so processing supplies the generic one.
        let thumbnail = url_or_none(video("poster")).or_else(|| url_or_none(video("coverUrl")));
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(Value::from);
        let int = |value: Option<&Value>| present(super::streamable::int_or_none(value));
        let uploader_id = user
            .and_then(|user| user.get("userId"))
            .and_then(|id| match id {
                Value::String(text) => Some(Value::from(text.as_str())),
                Value::Null => None,
                other => Some(Value::from(other.to_string())),
            });
        let duration = present(super::clyp::float_or_none(
            play_info.and_then(|info| info.get("video_duration")),
        ));
        let optional: [(&str, Option<Value>); 10] = [
            ("comment_count", int(video("commentCount"))),
            ("duration", duration),
            ("like_count", int(video("repinCount"))),
            ("release_timestamp", int(video("publishTime"))),
            ("thumbnail", thumbnail),
            ("title", Some(text(video("title")).unwrap_or(Value::Null))),
            ("uploader", text(user.and_then(|user| user.get("name")))),
            ("uploader_id", uploader_id),
            ("view_count", int(video("playCount"))),
            ("webpage_url", url_or_none(video("detailUrl"))),
        ];
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", Value::from(video_id.clone())),
            ("formats", Value::Null),
        ];
        fields.extend(
            optional
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        );
        Ok(Some(dict_info(url, (NAME, KEY), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.toutiao.com/video/7505382061495176511/"
        ));
        assert!(suitable(
            "https://www.toutiao.com/video/7479446610359878153"
        ));
        assert!(suitable(
            "https://www.toutiao.com/video/7479446610359878153?x=1"
        ));
        assert!(!suitable("https://www.toutiao.com/a7479446610359878153/"));
    }
}
