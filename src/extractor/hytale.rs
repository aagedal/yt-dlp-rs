//! Upstream `HytaleIE` (`yt_dlp/extractor/hytale.py`): Hytale news posts as
//! playlists of their Cloudflare Stream embeds (transparent `url` results
//! titled from the media page's clip captions), with the OpenGraph title.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Hytale;

const NAME: &str = "Hytale";
const CLOUDFLARE: &str = "CloudflareStream";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?hytale\.com/news/\d+/\d+/(?P<id>[a-z0-9-]+)").unwrap()
});
static CLIPS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__INITIAL_COMPONENTS_STATE__\s*=\s*\[\s*").unwrap());
static STREAM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<stream\s+class\s*=\s*"ql-video\s+cf-stream"\s+src\s*=\s*"([a-f0-9]{32})""#)
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_real_initialize`: the media page's clip captions by stream ID.
fn clip_titles(client: &Client, playlist_id: &str) -> Result<Map<String, Value>> {
    crate::screen::screen(&format!("[{NAME}] Downloading list of media"));
    let page = Url::parse("https://hytale.com/media")?;
    // `fatal=False`: a failed download is a warning and an empty page.
    let media = match super::download_webpage_quiet(client, &page, NAME, "") {
        Ok(media) => media,
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] Unable to download webpage: {}",
                crate::upstream_error_text(&source)
            ));
            String::new()
        }
        Err(error) => return Err(error),
    };
    let state = super::search_json(&CLIPS, &media, false).ok_or_else(|| {
        Error::Reported(format!(
            "[{NAME}] {playlist_id}: Unable to extract clips json; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    let mut titles = Map::new();
    if let Some(Value::Array(clips)) = state
        .as_ref()
        .and_then(|state| state.pointer("/media/clips"))
    {
        for clip in clips {
            let src = match clip.get("src") {
                Some(Value::String(src)) => src.clone(),
                _ => continue,
            };
            titles.insert(src, clip.get("caption").cloned().unwrap_or(Value::Null));
        }
    }
    Ok(titles)
}

impl Extractor for Hytale {
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
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        // `initialize()` runs before the `Extracting URL` line.
        let titles = clip_titles(client, &playlist_id)?;
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &playlist_id)?;
        let mut entries = Vec::new();
        for found in STREAM.captures_iter(&webpage) {
            let hash = &found[1];
            let target = format!(
                "https://cloudflarestream.com/{hash}/manifest/video.mpd?parentOrigin=https%3A%2F%2Fhytale.com"
            );
            let title = titles.get(hash).cloned().unwrap_or(Value::Null);
            let mut entry = Map::new();
            entry.insert("title".into(), title.clone());
            entry.insert("ie_key".into(), CLOUDFLARE.into());
            entry.insert("_type".into(), "url_transparent".into());
            entry.insert("url".into(), target.clone().into());
            let mut fields = Map::new();
            if !title.is_null() {
                fields.insert("title".into(), title);
            }
            let parsed = Url::parse(&target)?;
            let mut info = super::direct::build_at(parsed.clone(), &parsed, String::new());
            info.formats.clear();
            info.direct = None;
            info.id = hash.to_owned();
            info.extra.insert(
                crate::TRANSPARENT.into(),
                json!({
                    "url": target,
                    "force_videoid": null,
                    "fields": Value::Object(fields),
                    "ie_key": CLOUDFLARE,
                    "extractor": CLOUDFLARE,
                    "entry": Value::Object(entry),
                }),
            );
            entries.push(info);
        }
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            super::search_warning(NAME, "OpenGraph title");
        }
        Ok(Some(super::nrk::playlist_result(
            url,
            NAME,
            playlist_id,
            title.filter(|title| !title.is_empty()),
            None,
            entries,
            false,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://hytale.com/news/2021/07/summer-2021-development-update"
        ));
        assert!(!suitable("https://hytale.com/media"));
    }
}
