//! Upstream `SoundgasmIE` and `SoundgasmProfileIE`
//! (`yt_dlp/extractor/soundgasm.py`): a sound page's jPlayer `m4a` URL with
//! its title and description, and a profile's sound links as a playlist.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Soundgasm;
pub struct SoundgasmProfile;

const NAME: &str = "Soundgasm";
const IE_NAME: &str = "soundgasm";
const PROFILE_NAME: &str = "SoundgasmProfile";
const PROFILE_IE_NAME: &str = "soundgasm:profile";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?soundgasm\.net/u/(?P<user>[0-9a-zA-Z_-]+)/(?P<display_id>[0-9a-zA-Z_-]+)",
    )
    .unwrap()
});
static PROFILE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?soundgasm\.net/u/(?P<id>[^/]+)/?(?:\#.*)?$").unwrap()
});
/// `(?s)m4a\s*:\s*(["\'])(?P<url>(?:(?!\1).)+)\1` as one alternative per
/// quote.
static AUDIO_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)m4a\s*:\s*(?:"(?P<a>[^"]+)"|'(?P<b>[^']+)')"#).unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div[^>]+\bclass=["']jp-title[^>]+>([^<]+)"#).unwrap());
static DESCRIPTIONS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"(?s)<div[^>]+\bclass=["']jp-description[^>]+>(.+?)</div>"#).unwrap(),
        Regex::new(r"(?s)<li>Description:\s(.*?)</li>").unwrap(),
    ]
});
static AUDIO_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/([^/]+)\.m4a").unwrap());

/// `SoundgasmIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `SoundgasmProfileIE._match_id`.
pub fn profile_suitable(url: &str) -> bool {
    PROFILE_URL.is_match(url)
}

fn clean(text: &str) -> String {
    crate::html::clean_html(text).trim().to_owned()
}

impl Extractor for Soundgasm {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let user = found["user"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(IE_NAME);
        crate::screen::screen(&format!(
            "[{IE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, IE_NAME, &display_id)?;
        // `_html_search_regex(..., 'audio URL', group='url')` (fatal; the
        // pattern has no `id` group, so the error names no video).
        let audio_url = AUDIO_URL
            .captures(&webpage)
            .and_then(|found| found.name("a").or_else(|| found.name("b")))
            .map(|found| clean(found.as_str()))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{IE_NAME}] Unable to extract audio URL; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let title = TITLE
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .unwrap_or_else(|| display_id.clone());
        let description = DESCRIPTIONS
            .iter()
            .find_map(|pattern| pattern.captures(&webpage))
            .map(|found| clean(&found[1]));
        if description.is_none() {
            search_warning(IE_NAME, "description");
        }
        let audio_id = AUDIO_ID
            .captures(&audio_url)
            .map(|found| found[1].to_owned())
            .unwrap_or_else(|| display_id.clone());
        let media = Url::parse(&audio_url).map_err(|_| {
            Error::UnsupportedHtml5(format!(
                "Soundgasm audio URLs that are not absolute are not supported natively: {audio_url}"
            ))
        })?;
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.vcodec = Some("none".into());
            format.format_id = String::new();
        }
        info.vcodec = Some("none".into());
        info.format_id = None;
        info.id = audio_id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = IE_NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(&mut info, "display_id", Value::String(display_id));
        crate::metadata_parser::set_field(
            &mut info,
            "description",
            description.map_or(Value::Null, Value::String),
        );
        crate::metadata_parser::set_field(&mut info, "uploader", Value::String(user));
        Ok(Some(crate::info_order::mark_info(
            info,
            "untyped-dict;id,display_id,url,vcodec,title,description,uploader",
        )))
    }
}

/// `url_result(audio_url, 'Soundgasm')`.
fn sound_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target).map_err(|_| {
        Error::UnsupportedHtml5(format!(
            "Soundgasm profile links that are not absolute URLs are not supported natively: {target}"
        ))
    })?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": IE_NAME,
            "entry": {"ie_key": NAME, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for SoundgasmProfile {
    fn key(&self) -> &'static str {
        PROFILE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PROFILE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(PROFILE_IE_NAME);
        crate::screen::screen(&format!(
            "[{PROFILE_IE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, PROFILE_IE_NAME, &id)?;
        // `re.findall(rf'href="([^"]+/u/{profile_id}/[^"]+)', webpage)`: the
        // id is interpolated into the pattern unescaped.
        let links = Regex::new(&format!(r#"href="([^"]+/u/{id}/[^"]+)"#)).map_err(|_| {
            Error::UnsupportedHtml5(format!(
                "Soundgasm profile ids that are not valid patterns are not supported natively: {id}"
            ))
        })?;
        let entries = links
            .captures_iter(&webpage)
            .map(|found| sound_entry(&found[1]))
            .collect::<Result<Vec<_>>>()?;
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: PROFILE_IE_NAME.into(),
            extractor_key: PROFILE_NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://soundgasm.net/u/ytdl/Piano-sample"));
        assert!(!suitable("http://soundgasm.net/u/ytdl"));
        assert!(profile_suitable("http://soundgasm.net/u/ytdl"));
        assert!(profile_suitable("https://www.soundgasm.net/u/ytdl/#x"));
        assert!(!profile_suitable(
            "http://soundgasm.net/u/ytdl/Piano-sample"
        ));
    }
}
