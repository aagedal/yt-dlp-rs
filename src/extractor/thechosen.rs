//! Upstream `TheChosenIE` (`yt_dlp/extractor/thechosen.py`): the video API's
//! HLS and DASH manifests (non-fatal), its thumbnails and metadata. The
//! `TheChosenGroupIE` playlist (a `FrontroGroupBaseIE`, `_WORKING = False`)
//! is not ported.
use super::{
    Extractor, dict_info, direct, download_json, extract_m3u8_master_optional, extract_mpd_optional,
};
use crate::{Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TheChosen;

const NAME: &str = "TheChosen";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?watch\.thechosen\.tv/(?:video|watch)/(?P<id>[0-9]+)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for TheChosen {
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
        let api = Url::parse(&format!(
            "https://api.watch.thechosen.tv/v1/videos/{video_id}"
        ))?;
        let metadata = download_json(client, &api, NAME, &video_id)?;

        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        // `('details', 'video', ..., 'url', {url_or_none})`.
        let manifests = metadata
            .pointer("/details/video")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("url")?.as_str())
            .filter_map(super::json_ld::url_or_none);
        for manifest_url in manifests {
            let ext =
                direct::determine_ext(&manifest_url).unwrap_or_else(|| "unknown_video".into());
            let manifest = Url::parse(&manifest_url)?;
            let stream = match ext.as_str() {
                "m3u8" => extract_m3u8_master_optional(
                    client, &manifest, NAME, &video_id, None, true, false,
                ),
                "mpd" => extract_mpd_optional(client, &manifest, NAME, &video_id, true),
                _ => {
                    crate::screen::warning(&format!(
                        "[{NAME}] {video_id}: Skipping unsupported format extension \"{ext}\""
                    ));
                    continue;
                }
            };
            if let Some(stream) = stream {
                formats.extend(stream.formats);
                super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        }

        // `(('thumbs', 'thumbnails'), {dict.items}, lambda _, v: url_or_none(v[1]))`.
        let mut thumbnails = Vec::new();
        for key in ["thumbs", "thumbnails"] {
            let Some(Value::Object(items)) = metadata.get(key) else {
                continue;
            };
            for (thumb_id, thumb_url) in items {
                if let Some(thumb_url) = thumb_url.as_str().and_then(super::json_ld::url_or_none) {
                    thumbnails.push(json!({"id": thumb_id, "url": thumb_url}));
                }
            }
        }

        let mut fields = vec![("id", Value::from(video_id.clone()))];
        for key in ["title", "description"] {
            if let Some(text) = metadata.get(key).and_then(Value::as_str) {
                fields.push((key, text.into()));
            }
        }
        let duration = super::streamable::int_or_none(metadata.get("duration"));
        if !duration.is_null() {
            fields.push(("duration", duration));
        }
        fields.push(("thumbnails", Value::Array(thumbnails)));
        fields.push(("formats", Value::Null));
        fields.push(("subtitles", Value::Null));
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(with_thumbnail_keys(info, "id,url")))
    }
}

/// The info's `dict;KEYS` shape with the thumbnails' key order (upstream's
/// dict literal order, `_sanitize_thumbnails` adding the rest).
fn with_thumbnail_keys(info: VideoInfo, keys: &str) -> VideoInfo {
    let shape = info
        .extra
        .get(crate::info_order::INFO_SHAPE)
        .and_then(Value::as_str)
        .unwrap_or("dict")
        .to_owned();
    crate::info_order::mark_info(info, &format!("{shape};thumbnail={keys}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://watch.thechosen.tv/video/184683594325"));
        assert!(suitable("https://watch.thechosen.tv/watch/184683596189"));
        assert!(!suitable("https://thechosen.tv/video/1"));
        assert!(!suitable("https://watch.thechosen.tv/group/309237658592"));
    }
}
