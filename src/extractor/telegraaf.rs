//! Upstream `TelegraafIE` (`yt_dlp/extractor/telegraaf.py`): an article's
//! `app.telegraaf.nl/graphql` video ID and its `content.tmgvideo.nl`
//! playlist item (adaptive HLS/DASH manifests and progressive files).
use super::streamable::int_or_none;
use super::{
    Extractor, dict_format, dict_info, download_json, download_json_headers,
    extract_m3u8_master_optional, extract_mpd_optional_note,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Telegraaf;

const NAME: &str = "Telegraaf";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?telegraaf\.nl/video/(?P<id>\d+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!(
        "Telegraaf responses without {what} are not supported natively"
    ))
}

impl Extractor for Telegraaf {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let article_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let query = format!(
            "{{\n  article(uid: {article_id}) {{\n    videos {{\n      videoId\n    }}\n  }}\n}}"
        );
        let graphql = super::update_url_query(
            &Url::parse("https://app.telegraaf.nl/graphql")?,
            &[("query", &query)],
        );
        let article = download_json_headers(
            client,
            &graphql,
            NAME,
            &article_id,
            &[("User-Agent", "De Telegraaf/6.8.11 (Android 11; en_US)")],
        )?;
        let video_id = article
            .pointer("/data/article/videos/0/videoId")
            .and_then(Value::as_str)
            .ok_or_else(|| unsupported("a string data.article.videos[0].videoId"))?
            .to_owned();
        let playlist = Url::parse(&format!(
            "https://content.tmgvideo.nl/playlist/item={video_id}/playlist.json"
        ))?;
        let item = download_json(client, &playlist, NAME, &video_id)?
            .pointer("/items/0")
            .cloned()
            .ok_or_else(|| unsupported("items[0]"))?;
        let title = item
            .get("title")
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&video_id, "title"))?;
        if !title.is_string() {
            return Err(unsupported("a string title"));
        }

        let mut formats = Vec::new();
        let locations = item.get("locations").cloned().unwrap_or(Value::Null);
        let list = |name: &str| {
            locations
                .get(name)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        for location in list("adaptive") {
            let Some(manifest) = location
                .get("src")
                .and_then(Value::as_str)
                .filter(|src| !src.is_empty())
            else {
                continue;
            };
            let ext =
                super::direct::determine_ext(manifest).unwrap_or_else(|| "unknown_video".into());
            let manifest_url = Url::parse(manifest)?;
            match ext.as_str() {
                "m3u8" => {
                    if let Some(stream) = extract_m3u8_master_optional(
                        client,
                        &manifest_url,
                        NAME,
                        &video_id,
                        Some("hls"),
                        false,
                        false,
                    ) {
                        formats.extend(stream.formats);
                    }
                }
                "mpd" => {
                    if let Some(mut stream) = extract_mpd_optional_note(
                        client,
                        &manifest_url,
                        NAME,
                        &video_id,
                        Some("dash"),
                        "Downloading MPD manifest",
                    ) {
                        super::ignore_subtitles(&mut stream, NAME, "DASH");
                        formats.extend(stream.formats);
                    }
                }
                ext => crate::screen::warning(&format!("[{NAME}] Unknown adaptive format {ext}")),
            }
        }
        for location in list("progressive") {
            let Some(src) = location
                .pointer("/sources/0/src")
                .filter(|src| super::nuxt::truthy(src))
            else {
                continue;
            };
            let label = location
                .get("label")
                .filter(|label| super::nuxt::truthy(label))
                .and_then(Value::as_str);
            formats.push(dict_format(
                vec![
                    ("url", src.clone()),
                    ("width", int_or_none(location.get("width"))),
                    ("height", int_or_none(location.get("height"))),
                    (
                        "format_id",
                        label
                            .map_or_else(|| "http".into(), |label| format!("http-{label}"))
                            .into(),
                    ),
                ],
                NAME,
                &video_id,
            )?);
        }

        let timestamp = item
            .get("datecreated")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601_space)
            .map_or(Value::Null, Value::from);
        let duration = int_or_none(item.get("duration"));
        let mut fields = vec![
            ("id", video_id.clone().into()),
            ("title", title),
            (
                "description",
                item.get("description").cloned().unwrap_or(Value::Null),
            ),
            ("formats", Value::Null),
        ];
        // Processing drops a `None` duration.
        if !duration.is_null() {
            fields.push(("duration", duration));
        }
        fields.push((
            "thumbnail",
            item.get("poster").cloned().unwrap_or(Value::Null),
        ));
        fields.push(("timestamp", timestamp));
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.telegraaf.nl/video/734366489/historisch-scheepswrak-slaat-na-100-jaar-los"
        ));
        assert!(!suitable("https://www.telegraaf.nl/nieuws/734366489"));
    }
}
