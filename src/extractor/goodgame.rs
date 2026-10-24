//! Upstream `GoodGameIE` (`yt_dlp/extractor/goodgame.py`): a channel's
//! stream API and, while it is live, the master HLS playlist.
use super::{Extractor, download_json, extract_m3u8_fatal_live};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct GoodGame;

const NAME: &str = "goodgame:stream";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://goodgame\.ru/(?P<id>[\w.*-]+)").unwrap());

/// `_match_id` (`(?!channel/)` checked separately).
pub fn suitable(url: &str) -> bool {
    VALID_URL
        .captures(url)
        .is_some_and(|found| !url[found.get(1).unwrap().start()..].starts_with("channel/"))
}

/// `str_or_none`.
fn str_or_none(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::String(text) => Some(text.clone().into()),
        Value::Number(number) => Some(number.to_string().into()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.into()),
        _ => None,
    }
}

impl Extractor for GoodGame {
    fn key(&self) -> &'static str {
        "GoodGame"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let channel_name = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://goodgame.ru/api/4/users/{channel_name}/stream"
        ))?;
        let response = download_json(client, &api, NAME, &channel_name)?;
        let player_id = match response.get("streamkey") {
            Some(Value::String(key)) => key.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "GoodGame stream keys that are not strings are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "{channel_name}: An extractor error has occurred. (caused by KeyError('streamkey')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let live = match response.get("status") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => false,
            Some(Value::Number(number)) => number.as_f64() != Some(0.0),
            Some(Value::String(text)) => !text.is_empty(),
            Some(Value::Array(items)) => !items.is_empty(),
            Some(Value::Object(object)) => !object.is_empty(),
            Some(Value::Bool(true)) => true,
        };
        let mut info = if live {
            let manifest = Url::parse(&format!(
                "https://hls.goodgame.ru/manifest/{player_id}_master.m3u8"
            ))?;
            extract_m3u8_fatal_live(client, &manifest, NAME, &channel_name, None, true)?
        } else {
            // `raise_no_formats('User is offline', expected=True)`, then
            // the dict without formats.
            crate::no_formats::raise_expected(NAME, Some(&channel_name), None, "User is offline")?;
            let mut info = super::dict_info(url, (NAME, "GoodGame"), Vec::new(), Vec::new());
            info.extra.shift_remove(crate::info::TITLE_MISSING);
            info
        };
        let is_live = !info.formats.is_empty();
        info.id = player_id;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = "GoodGame".into();
        crate::metadata_parser::set_field(&mut info, "is_live", Value::Bool(is_live));
        let mut keys = vec!["id", "formats", "subtitles", "is_live"];
        let at = |path: &[&str]| {
            path.iter()
                .try_fold(&response, |value, key| value.get(*key))
                .filter(|value| !value.is_null())
        };
        let string = |path: &[&str]| at(path).filter(|value| value.is_string()).cloned();
        let url_field = |path: &[&str]| {
            at(path)
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
        };
        let title = string(&["title"]);
        let fields = [
            ("channel", string(&["channelkey"])),
            ("channel_id", str_or_none(at(&["id"]))),
            ("channel_url", url_field(&["link"]).map(Value::from)),
            ("uploader", string(&["streamer", "username"])),
            ("uploader_id", str_or_none(at(&["streamer", "id"]))),
            (
                "thumbnail",
                url_field(&["preview"]).map(|thumbnail| super::proto_relative(&thumbnail).into()),
            ),
            (
                "concurrent_view_count",
                Some(super::streamable::int_or_none(at(&["viewers"]))).filter(|v| !v.is_null()),
            ),
            (
                "channel_follower_count",
                Some(super::streamable::int_or_none(at(&["followers"]))).filter(|v| !v.is_null()),
            ),
            (
                "age_limit",
                (at(&["adult"]) == Some(&Value::Bool(true))).then(|| Value::from(18)),
            ),
        ];
        match title {
            Some(Value::String(title)) => {
                keys.push("title");
                info.title = title;
            }
            _ => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        for (key, value) in fields {
            if let Some(value) = value {
                keys.push(key);
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("dict;{}", keys.join(",")),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://goodgame.ru/TGW#autoplay"));
        assert!(suitable("https://goodgame.ru/Mr.Gray"));
        assert!(suitable("https://goodgame.ru/HeDoPa3yMeHue*"));
        assert!(!suitable("https://goodgame.ru/channel/Pomi/"));
    }
}
