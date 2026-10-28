//! Upstream `MBNIE` (`yt_dlp/extractor/mbn.py`): MBN (매일방송) VOD
//! previews from the player JSON: each movie's stream URL authenticated
//! through `mbnStreamAuth_new_vod.mbn` and read as a non-fatal HLS
//! playlist, with the title, play time, broadcast date, image, programme
//! and episode number.
use super::{Extractor, dict_info, download_json_note, download_webpage};
use crate::{Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Mbn;

const NAME: &str = "MBN";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?mbn\.co\.kr/vod/programContents/preview(?:list)?/\d+/\d+/(?P<id>\d+)",
    )
    .unwrap()
});
static CONTENT_CLS_CD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""\?content_cls_cd=(\d+)&"#).unwrap());
static PLAYLIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/(?:chunk|play)list(?:_pd\d+)?\.m3u8").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `traverse_obj(media_info, ('movie_list', ..., 'url', {url_or_none}))`.
fn stream_urls(media_info: &Value) -> Vec<String> {
    let movies: Vec<&Value> = match media_info.get("movie_list") {
        Some(Value::Array(movies)) => movies.iter().collect(),
        Some(Value::Object(movies)) => movies.values().collect(),
        _ => Vec::new(),
    };
    movies
        .into_iter()
        .filter_map(|movie| movie.get("url"))
        .filter_map(Value::as_str)
        .filter_map(super::json_ld::url_or_none)
        .collect()
}

impl Extractor for Mbn {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let content_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &content_id)?;
        let content_cls_cd = match CONTENT_CLS_CD.captures(&webpage) {
            Some(found) => found[1].to_owned(),
            None => {
                super::search_warning(NAME, "content cls cd");
                "20".to_owned()
            }
        };
        let player = super::update_url_query(
            &Url::parse("https://www.mbn.co.kr/player/mbnVodPlayer_2020.mbn")?,
            &[
                ("content_cls_cd", content_cls_cd.as_str()),
                ("content_id", content_id.as_str()),
                ("relay_type", "1"),
            ],
        );
        let media_info =
            download_json_note(client, &player, NAME, &content_id, "Fetching playback data")?;

        let mut formats = Vec::new();
        for stream_url in stream_urls(&media_info) {
            let stream_url = PLAYLIST.replace_all(&stream_url, "/manifest.m3u8");
            let auth = Url::parse(&format!(
                "https://www.mbn.co.kr/player/mbnStreamAuth_new_vod.mbn?vod_url={stream_url}"
            ))?;
            let final_url = super::download_webpage_note(
                client,
                &auth,
                NAME,
                &content_id,
                "Fetching authenticated m3u8 url",
            )?;
            let Some(final_url) = super::json_ld::url_or_none(&final_url) else {
                // `_extract_m3u8_formats(None, fatal=False)`.
                crate::screen::warning(&format!(
                    "[{NAME}] Failed to obtain m3u8 URL; {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
                continue;
            };
            let Ok(manifest) = Url::parse(&final_url) else {
                return Err(crate::Error::UnsupportedHtml5(
                    "MBN authenticated m3u8 URLs that are not URLs are not supported natively"
                        .into(),
                ));
            };
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &content_id,
                None,
                false,
                false,
            ) {
                formats.extend(stream.formats);
            }
        }

        let string = |key: &str| match media_info.get(key) {
            Some(Value::String(text)) => Value::from(text.as_str()),
            _ => Value::Null,
        };
        // `{lambda x: x.replace('.', '')}, {unified_strdate}`: anything but a
        // string fails the call and is dropped.
        let release_date = match media_info.get("bcast_date") {
            Some(Value::String(date)) => {
                crate::dates::unified_strdate(&date.replace('.', ""), false)
                    .map_or(Value::Null, Value::from)
            }
            _ => Value::Null,
        };
        let thumbnail = media_info
            .get("movie_start_Img")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map_or(Value::Null, Value::from);
        let mut fields = vec![("id", Value::from(content_id))];
        for (key, value) in [
            ("title", string("movie_title")),
            (
                "duration",
                super::streamable::int_or_none(media_info.get("play_sec")),
            ),
            ("release_date", release_date),
            ("thumbnail", thumbnail),
            ("series", string("prog_nm")),
            (
                "episode_number",
                super::streamable::int_or_none(media_info.get("ad_contentnumber")),
            ),
        ] {
            if !value.is_null() {
                fields.push((key, value));
            }
        }
        fields.push(("formats", Value::Null));
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://mbn.co.kr/vod/programContents/previewlist/861/5433/1276155"
        ));
        assert!(suitable(
            "https://www.mbn.co.kr/vod/programContents/preview/952/6088/1054797?next=1"
        ));
        assert!(!suitable("https://www.mbn.co.kr/vod/programContents/861"));
        assert_eq!(
            PLAYLIST.replace("https://x/a/chunklist_pd12.m3u8?t=1", "/manifest.m3u8"),
            "https://x/a/manifest.m3u8?t=1"
        );
    }
}
