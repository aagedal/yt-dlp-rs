//! Upstream `AudiomackIE` and `AudiomackAlbumIE`
//! (`yt_dlp/extractor/audiomack.py`): a song's extended
//! `api/music/url/song/` JSON as its own media URL (a wrapped SoundCloud
//! track as a `url` result of the SoundCloud extractor), and an album's
//! songs queried one track number at a time until an empty URL.
use super::{Extractor, download_json, download_json_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Audiomack;
pub struct AudiomackAlbum;

const NAME: &str = "Audiomack";
const IE_NAME: &str = "audiomack";
const ALBUM_NAME: &str = "AudiomackAlbum";
const ALBUM_IE_NAME: &str = "audiomack:album";

/// `(?:song/|(?=.+/song/))(?P<id>[\w/-]+)`: the lookahead as a check that
/// the rest contains `/song/`.
static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?audiomack\.com/(?P<song>song/)?(?P<id>[\w/-]+)",
    )
    .unwrap()
});
static ALBUM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?audiomack\.com/(?P<album>album/)?(?P<id>[\w/-]+)").unwrap()
});

/// `_match_id` for `audiomack.com/(?:KIND/|(?=.+/KIND/))(?P<id>[\w/-]+)`.
fn match_id(pattern: &Regex, kind: &str, url: &str) -> Option<String> {
    let found = pattern.captures(url)?;
    if found.name(kind).is_some() {
        return Some(found["id"].to_owned());
    }
    // The lookahead: `.+/KIND/` somewhere after the host's slash (`.` stops
    // at a newline), with `[\w/-]+` then matching from the same position.
    let rest = &url[found.name("id")?.start()..];
    let line = rest.split('\n').next().unwrap_or_default();
    let needle = format!("/{kind}/");
    line.match_indices(&needle)
        .any(|(index, _)| index > 0)
        .then(|| found["id"].to_owned())
}

/// `AudiomackIE._match_id`.
pub fn suitable(url: &str) -> bool {
    match_id(&VALID_URL, "song", url).is_some()
}

/// `AudiomackAlbumIE._match_id`.
pub fn album_suitable(url: &str) -> bool {
    match_id(&ALBUM_URL, "album", url).is_some()
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Audiomack {what} are not supported natively"))
}

/// An `ExtractorError(message)` (unexpected, so with the bug report note)
/// under the URL's temporary ID.
fn extractor_error(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{name}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `str(value)`.
fn py_str(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(if *flag { "True" } else { "False" }.into()),
        Value::Null => Ok("None".into()),
        _ => Err(unsupported("IDs that are not scalars")),
    }
}

/// `int(time.time())`.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs())
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// An API response's `url` as an absolute URL.
fn media_url(api_response: &Map<String, Value>) -> Result<(String, Url)> {
    let Some(Value::String(media)) = api_response.get("url") else {
        return Err(unsupported("media URLs that are not strings"));
    };
    let parsed = Url::parse(media).map_err(|_| unsupported("media URLs that are not absolute"))?;
    Ok((media.clone(), parsed))
}

/// The `{'id', 'uploader', 'title', 'url'}` dict: its own (only) format.
fn song_info(
    page: &Url,
    (extractor, key): (&str, &str),
    id: String,
    uploader: Value,
    title: &Value,
    media: &Url,
) -> Result<VideoInfo> {
    let ext =
        super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
    let mut info = super::direct::with_format(
        super::direct::build_at(media.clone(), media, ext.clone()),
        "",
    );
    for format in &mut info.formats {
        format.ext = ext.clone();
        format.format_id = String::new();
    }
    info.format_id = None;
    info.id = id;
    match title {
        Value::String(title) => info.title = title.clone(),
        Value::Null => {
            info.title = String::new();
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        _ => return Err(unsupported("titles that are not strings")),
    }
    info.webpage_url = page.clone();
    info.direct = None;
    info.extractor = extractor.into();
    info.extractor_key = key.into();
    crate::metadata_parser::set_field(&mut info, "uploader", uploader);
    Ok(crate::info_order::mark_info(
        info,
        "untyped-dict;id,uploader,title,url",
    ))
}

impl Extractor for Audiomack {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(temp_id) = match_id(&VALID_URL, "song", url.as_str()) else {
            return Ok(None);
        };
        announce(IE_NAME, url);
        let album_url_tag = temp_id.replace("/song/", "/");
        let api = Url::parse(&format!(
            "http://www.audiomack.com/api/music/url/song/{album_url_tag}?extended=1&_={}",
            now()
        ))?;
        let api_response = download_json(client, &api, IE_NAME, &album_url_tag)?;
        let Value::Object(api_response) = api_response else {
            return Err(unsupported("API responses that are not objects"));
        };
        // API is inconsistent with errors.
        if !api_response.get("url").is_some_and(super::nuxt::truthy)
            || api_response.contains_key("error")
        {
            return Err(extractor_error(
                IE_NAME,
                &temp_id,
                &format!("Invalid url {}", url.as_str()),
            ));
        }
        let (media, parsed) = media_url(&api_response)?;
        // A wrapped SoundCloud track is SoundCloud's to extract.
        if super::soundcloud_suitable(&media) {
            let mut follow = super::direct::build_at(url.clone(), url, String::new());
            follow.formats.clear();
            follow.direct = None;
            follow.extra.insert(crate::FOLLOW_URL.into(), media.into());
            return Ok(Some(ExtractResult::Video(follow)));
        }
        let id = match api_response.get("id") {
            Some(id) => py_str(id)?,
            None => album_url_tag,
        };
        let info = song_info(
            url,
            (IE_NAME, NAME),
            id,
            api_response.get("artist").cloned().unwrap_or(Value::Null),
            api_response.get("title").unwrap_or(&Value::Null),
            &parsed,
        )?;
        Ok(Some(ExtractResult::Video(info)))
    }
}

impl Extractor for AudiomackAlbum {
    fn key(&self) -> &'static str {
        ALBUM_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(temp_id) = match_id(&ALBUM_URL, "album", url.as_str()) else {
            return Ok(None);
        };
        announce(ALBUM_IE_NAME, url);
        let album_url_tag = temp_id.replace("/album/", "/");
        // The album's metadata is repeated in each song's, so songs are
        // queried until one has an empty URL.
        let mut album: Vec<(&str, String)> = Vec::new();
        let mut entries = Vec::new();
        for track_no in 0usize.. {
            let api = Url::parse(&format!(
                "http://www.audiomack.com/api/music/url/album/{album_url_tag}/{track_no}?extended=1&_={}",
                now()
            ))?;
            let api_response = download_json_note(
                client,
                &api,
                ALBUM_IE_NAME,
                &album_url_tag,
                &format!("Querying song information ({})", track_no + 1),
            )?;
            let Value::Object(api_response) = api_response else {
                return Err(unsupported("API responses that are not objects"));
            };
            if !api_response.contains_key("url") || api_response.contains_key("error") {
                return Err(extractor_error(
                    ALBUM_IE_NAME,
                    &temp_id,
                    &format!(
                        "Invalid url for track {track_no} of album url {}",
                        url.as_str()
                    ),
                ));
            }
            if !super::nuxt::truthy(&api_response["url"]) {
                break;
            }
            for (result_key, api_key) in [("id", "album_id"), ("title", "album_title")] {
                if let Some(value) = api_response.get(api_key)
                    && !album.iter().any(|(key, _)| *key == result_key)
                {
                    album.push((result_key, py_str(value)?));
                }
            }
            let (_, parsed) = media_url(&api_response)?;
            let basename = crate::info::url_basename(&parsed);
            let song_id = basename
                .rsplit_once('.')
                .map_or("", |(stem, _)| stem)
                .to_owned();
            let id = match api_response.get("id") {
                Some(id) => py_str(id)?,
                None => song_id.clone(),
            };
            let song_title = Value::String(song_id);
            entries.push(song_info(
                url,
                (ALBUM_IE_NAME, ALBUM_NAME),
                id,
                api_response.get("artist").cloned().unwrap_or(Value::Null),
                api_response.get("title").unwrap_or(&song_title),
                &parsed,
            )?);
        }
        let mut extra = Map::new();
        let mut id = None;
        let mut title = String::new();
        for (key, value) in &album {
            match *key {
                "id" => id = Some(value.clone()),
                _ => title = value.clone(),
            }
        }
        if id.is_none() {
            // `{'_type': 'playlist', 'entries': [...]}` has no `id`.
            extra.insert("id".into(), Value::Null);
        }
        let keys: Vec<&str> = album.iter().map(|(key, _)| *key).collect();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;;after={}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: id.unwrap_or_default(),
            title,
            webpage_url: url.clone(),
            extractor: ALBUM_IE_NAME.into(),
            extractor_key: ALBUM_NAME.into(),
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
        let song = |url| match_id(&VALID_URL, "song", url);
        assert_eq!(
            song("http://www.audiomack.com/song/roosh-williams/extraordinary").as_deref(),
            Some("roosh-williams/extraordinary")
        );
        assert_eq!(
            song("https://audiomack.com/roosh-williams/song/extraordinary").as_deref(),
            Some("roosh-williams/song/extraordinary")
        );
        assert_eq!(
            song("https://audiomack.com/roosh-williams/extraordinary"),
            None
        );
        assert_eq!(song("https://audiomack.com/song"), None);
        let album = |url| match_id(&ALBUM_URL, "album", url);
        assert_eq!(
            album("http://www.audiomack.com/album/flytunezcom/tha-tour-part-2-mixtape").as_deref(),
            Some("flytunezcom/tha-tour-part-2-mixtape")
        );
        assert_eq!(
            album("https://audiomack.com/flytunezcom/album/tha-tour").as_deref(),
            Some("flytunezcom/album/tha-tour")
        );
        assert!(!album_suitable(
            "http://www.audiomack.com/song/roosh-williams/extraordinary"
        ));
        assert!(!suitable(
            "http://www.audiomack.com/album/flytunezcom/tha-tour"
        ));
    }
}
