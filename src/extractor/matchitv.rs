//! Upstream `MatchiTVIE` (`yt_dlp/extractor/matchitv.py`): the watch page's
//! non-fatal `__NEXT_DATA__` `loadedMedia` court and start time, and the
//! fixed padelgo.tv HLS playlist and thumbnail.
use super::{Extractor, download_webpage, extract_m3u8_fatal};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MatchiTv;

const NAME: &str = "MatchiTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?matchi\.tv/watch/?\?(?:[^#]+&)?s=(?P<id>[0-9a-zA-Z]+)")
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MATCHi TV {what} are not supported natively"))
}

impl Extractor for MatchiTv {
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
        // `_search_nextjs_data(webpage, video_id, fatal=False)`.
        let nextjs = match super::search_nextjs_data(&webpage, NAME, &video_id) {
            Ok(data) => data,
            Err(Error::Reported(message)) => {
                let message = message.strip_prefix("[MatchiTV] ").unwrap_or(&message);
                let message = message
                    .strip_prefix(&format!("{video_id}: "))
                    .unwrap_or(message)
                    .replacen("Unable to", "unable to", 1);
                crate::screen::warning(&format!("[{NAME}] {message}"));
                Value::Null
            }
            Err(error) => return Err(error),
        };
        // `('props', 'pageProps', 'loadedMedia', {dict})) or {}`.
        let loaded_media = nextjs
            .get("props")
            .and_then(|props| props.get("pageProps"))
            .and_then(|props| props.get("loadedMedia"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let start = loaded_media
            .get("startDateTime")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let court = match loaded_media.get("courtDescription") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => return Err(unsupported("court descriptions that are not strings")),
        };
        // `join_nonempty(court, start, delim=' ')`.
        let title = [court, start.clone()]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let manifest = Url::parse(&format!(
            "https://streams.padelgo.tv/v2/streams/m3u8/{video_id}/anonymous/playlist.m3u8"
        ))?;
        let mut info = extract_m3u8_fatal(client, &manifest, NAME, &video_id, Some("hls"), false)?;
        // `ext='mp4'`.
        for format in &mut info.formats {
            format.ext = "mp4".into();
        }
        info.ext = "mp4".into();
        info.id = video_id.clone();
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.title = title;
        info.metadata.thumbnail = Some(format!("https://thumbnails.padelgo.tv/{video_id}.jpg"));
        match start.and_then(|start| crate::dates::unified_strdate(&start, true)) {
            Some(date) => crate::metadata_parser::set_field(&mut info, "upload_date", date.into()),
            None => {
                info.extra.insert("upload_date".into(), Value::Null);
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,thumbnail,upload_date,formats",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://matchi.tv/watch?s=0euhjzrxsjm"));
        assert!(suitable("https://www.matchi.tv/watch/?a=1&s=FkKDJ9SvAx1"));
        assert!(!suitable("https://matchi.tv/watch?id=x"));
    }
}
