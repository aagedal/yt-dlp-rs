//! Upstream `YoukuIE` (`yt_dlp/extractor/youku.py`): a video's `ups` play
//! metadata (the `log.mmstat` ETag as the client's `utid`) and its HLS
//! streams. `YoukuShowIE` (the `list.youku.com` episode lists) is not ported.
use super::direct;
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Youku;

const NAME: &str = "youku";
const KEY: &str = "Youku";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?:https?://((?:v|play(?:er)?)\.(?:youku|tudou)\.com/(?:v_show/id_|player\.php/sid/)|video\.tudou\.com/v/)|youku:)(?P<id>[A-Za-z0-9]+)(?:\.html|/v\.swf|)",
    )
    .unwrap()
});

/// `YoukuIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `get_format_name`.
fn format_name(stream_type: Option<&str>) -> Option<&'static str> {
    Some(match stream_type? {
        "3gp" => "h6",
        "3gphd" => "h5",
        "flv" | "flvhd" => "h4",
        "mp4" => "h3",
        "mp4hd" => "h3",
        "mp4hd2" | "mp4hd3" => "h4",
        "hd2" => "h2",
        "hd3" => "h1",
        _ => return None,
    })
}

/// `int(value)` of a JSON number or numeric string.
fn int_value(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|v| v as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `clean_html` of an error note, for the server's error message.
fn note_text(note: &str) -> String {
    crate::html::clean_html(note)
}

impl Extractor for Youku {
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

        // `Retrieving cna info`: the ETag's quoted value is the `utid`.
        let mmstat = Url::parse("https://log.mmstat.com/eg.js")?;
        let response = direct::request_with_headers(client, &mmstat, None, &[])?;
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| key_error(&video_id, "etag"))?;
        let cna = etag
            .get(1..etag.len().saturating_sub(1))
            .unwrap_or_default()
            .to_owned();

        let client_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_secs_f64())
            / 1000.0;
        let ups = super::update_url_query(
            &Url::parse("https://ups.youku.com/ups/get.json")?,
            &[
                ("vid", &video_id),
                ("ccode", "0564"),
                ("client_ip", "192.168.1.1"),
                ("utid", &cna),
                ("client_ts", &client_ts.to_string()),
            ],
        );
        // `headers.update(self.geo_verification_headers())`.
        let response = download_json(
            &super::geo_verification_client(client),
            &ups,
            NAME,
            &video_id,
        )?;
        let data = response
            .get("data")
            .ok_or_else(|| key_error(&video_id, "data"))?;

        if let Some(error) = data.get("error") {
            let note = error.get("note").and_then(Value::as_str);
            if note.is_some_and(|note| note.contains("因版权原因无法观看此视频")) {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Youku said: Sorry, this video is available in China only"
                )));
            }
            if note.is_some_and(|note| note.contains("该视频被设为私密")) {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Youku said: Sorry, this video is private"
                )));
            }
            let code = int_value(error.get("code")).ok_or_else(|| key_error(&video_id, "code"))?;
            let mut message = format!("Youku server reported error {code}");
            if let Some(note) = note {
                message.push_str(": ");
                message.push_str(&note_text(note));
            }
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: {message}; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }

        let video_data = data
            .get("video")
            .ok_or_else(|| key_error(&video_id, "video"))?;
        let title = video_data
            .get("title")
            .ok_or_else(|| key_error(&video_id, "title"))?
            .clone();

        let mut formats = Vec::new();
        let streams = data
            .get("stream")
            .and_then(Value::as_array)
            .ok_or_else(|| key_error(&video_id, "stream"))?;
        for stream in streams {
            if stream.get("channel_type").and_then(Value::as_str) == Some("tail") {
                continue;
            }
            let size = int_value(stream.get("size")).ok_or_else(|| key_error(&video_id, "size"))?;
            let manifest = stream.get("m3u8_url").cloned().unwrap_or(Value::Null);
            formats.push(dict_format(
                vec![
                    ("url", manifest),
                    (
                        "format_id",
                        format_name(stream.get("stream_type").and_then(Value::as_str))
                            .map_or(Value::Null, Value::from),
                    ),
                    ("ext", json!("mp4")),
                    ("protocol", json!("m3u8_native")),
                    ("filesize", json!(size)),
                    ("width", json!(int_value(stream.get("width")))),
                    ("height", json!(int_value(stream.get("height")))),
                ],
                NAME,
                &video_id,
            )?);
        }

        let uploader_home = data
            .get("uploader")
            .and_then(|uploader| uploader.get("homepage"))
            .cloned()
            .unwrap_or(Value::Null);
        let userid = video_data.get("userid").map(|userid| match userid {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        });
        let fields = vec![
            ("id", json!(video_id)),
            ("title", title),
            ("formats", Value::Null),
            (
                "duration",
                video_data.get("seconds").cloned().unwrap_or(Value::Null),
            ),
            (
                "thumbnail",
                video_data.get("logo").cloned().unwrap_or(Value::Null),
            ),
            (
                "uploader",
                video_data.get("username").cloned().unwrap_or(Value::Null),
            ),
            ("uploader_id", userid.map_or(Value::Null, Value::from)),
            ("uploader_url", uploader_home),
            (
                "tags",
                video_data.get("tags").cloned().unwrap_or(Value::Null),
            ),
        ];
        Ok(Some(dict_info(url, (NAME, KEY), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://player.youku.com/player.php/sid/XNDgyMDQ2NTQw/v.swf"
        ));
        assert!(suitable("http://v.youku.com/v_show/id_XNjA1NzA2Njgw.html"));
        assert!(suitable(
            "https://play.tudou.com/v_show/id_XNjAxNjI2OTU3Ng==.html?"
        ));
        assert!(suitable("youku:XNjA1NzA2Njgw"));
        assert!(!suitable("https://v.youku.com/"));
    }
}
