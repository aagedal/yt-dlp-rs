//! Upstream `VidlyIE` (`yt_dlp/extractor/vidly.py`): the embed script's
//! `initCallback` player config, either a JW Player playlist or Vidly's own
//! sources plus the CloudFront HLS master, which is requested best-effort.
use super::{Extractor, dict_format, dict_info, download_webpage_note_headers, html5, jwplayer};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Vidly;

const NAME: &str = "Vidly";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:vid\.ly/|(?:s\.)?vid\.ly/embeded\.html\?(?:[^#]+&)?link=)(?P<id>\w+)",
    )
    .unwrap()
});
static INIT_CALLBACK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"initCallback\(").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `value[key]`'s `KeyError`.
fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

impl Extractor for Vidly {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let embed = Url::parse(&format!("https://vid.ly/{video_id}/embed"))?;
        let script = download_webpage_note_headers(
            client,
            &embed,
            NAME,
            &video_id,
            "Downloading webpage",
            &[("Referer", "https://vid.ly/")],
        )?;
        // `re.search` over the whole script: a match not followed by JSON
        // (such as the script's own `initCallback(json)`) is skipped.
        let player = INIT_CALLBACK.find_iter(&script).find_map(|found| {
            match super::search_json(&INIT_CALLBACK, &script[found.start()..], false) {
                Some(Some(player)) => Some(player),
                _ => None,
            }
        });
        let Some(player) = player else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract player; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let player_type = player
            .get("player")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if player_type.starts_with("jwplayer") {
            let config = player
                .get("config")
                .ok_or_else(|| key_error(&video_id, "config"))?;
            let no_formats = || {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: No video formats found!; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            };
            let Some(entry) = jwplayer::parse_keeping_nulls(config, &video_id, &embed)? else {
                return Err(no_formats());
            };
            let Some((mut info, keys)) =
                html5::jwplayer_entry_info(client, entry, url, &video_id, NAME)?
            else {
                return Err(no_formats());
            };
            info.webpage_url = url.clone();
            info.extractor = NAME.into();
            info.extractor_key = NAME.into();
            let shape = format!("jwplayer;{}", keys.join(","));
            return Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
                info, &shape,
            ))));
        }
        if !player_type.starts_with("vidly") {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unknown player type '{player_type}'; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let config = player.get("config");
        let ext = super::vidyard::mimetype2ext(config.and_then(|config| config.get("type")));
        let ext = ext.as_str().unwrap_or_default().to_owned();
        let mut formats = Vec::new();
        for (source, format_id) in [("source", "http-sd"), ("source_hd", "http-hd")] {
            let Some(source_url) = config
                .and_then(|config| config.get(source))
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            formats.push(dict_format(
                vec![
                    ("url", source_url.into()),
                    ("format_id", format_id.into()),
                    ("ext", ext.clone().into()),
                ],
                NAME,
                &video_id,
            )?);
        }
        // Higher quality HLS variants (`fatal=False`, master playlist only).
        let hls = Url::parse(&format!(
            "https://d3fenhwk93s16g.cloudfront.net/{video_id}/hls.m3u8"
        ))?;
        if let Some(mut stream) =
            super::extract_m3u8_master_optional(client, &hls, NAME, &video_id, None, false, false)
        {
            // `_extract_m3u8_formats` without `ext`.
            super::hls::without_ext(&mut stream);
            formats.extend(stream.formats);
        }
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", video_id.clone().into()),
                ("formats", Value::Null),
            ],
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://vid.ly/2i3o9j/embed"));
        assert!(suitable("https://vid.ly/7x0e6l"));
        assert!(suitable("http://s.vid.ly/embeded.html?link=jw_test&new=1"));
        assert!(!suitable("https://example.com/vid.ly/2i3o9j"));
    }
}
