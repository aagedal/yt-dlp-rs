//! Upstream `GlobalPlayer*IE` classes (`yt_dlp/extractor/globalplayer.py`):
//! live stations and playlists, podcast and radio catch-up shows (as
//! playlists of their episodes), podcast episodes and videos. Each page's
//! `__NEXT_DATA__` props name the stream or media, which the BFF playables
//! API resolves to a playable URL.
use super::{Extractor, download_json, download_webpage, search_nextjs_data};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct GlobalPlayerLive;
pub struct GlobalPlayerLivePlaylist;
pub struct GlobalPlayerAudio;
pub struct GlobalPlayerAudioEpisode;
pub struct GlobalPlayerVideo;

const NAME: &str = "GlobalPlayer";
const PLAYABLES: &str = "https://bff-web-guacamole.musicradio.com/playables/";
const EPISODE_NAME: &str = "GlobalPlayerAudioEpisode";
const EPISODE_KEY: &str = "GlobalPlayerAudioEpisode";

static LIVE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://www\.globalplayer\.com/live/(?P<id>\w+)/\w+").unwrap());
static LIVE_PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://www\.globalplayer\.com/playlists/(?P<id>\w+)").unwrap()
});
static AUDIO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://www\.globalplayer\.com/(?P<path>(?P<podcast>podcasts)/|catchup/\w+/\w+/)(?P<id>\w+)/?(?:$|[?#])",
    )
    .unwrap()
});
static EPISODE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://www\.globalplayer\.com/(?:(?P<podcast>podcasts)|catchup/\w+/\w+)/episodes/(?P<id>\w+)/?(?:$|[?#])",
    )
    .unwrap()
});
static VIDEO_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://www\.globalplayer\.com/videos/(?P<id>\w+)").unwrap());

/// `_match_valid_url` of the classes above, in upstream's order.
pub fn live_suitable(url: &str) -> bool {
    LIVE_URL.is_match(url)
}

pub fn live_playlist_suitable(url: &str) -> bool {
    LIVE_PLAYLIST_URL.is_match(url)
}

pub fn audio_suitable(url: &str) -> bool {
    AUDIO_URL.is_match(url)
}

pub fn episode_suitable(url: &str) -> bool {
    EPISODE_URL.is_match(url)
}

pub fn video_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `_get_page_props(url, video_id)`.
fn page_props(client: &Client, url: &Url, id: &str, name: &'static str) -> Result<Value> {
    let webpage = download_webpage(client, url, name, id)?;
    let data = search_nextjs_data(&webpage, name, id)?;
    data.get("props")
        .and_then(|props| props.get("pageProps"))
        .cloned()
        .ok_or_else(|| key_error(id, "props"))
}

/// `_get_playback_url(data)`: the first `canUse == 'true'` playback URL.
fn playback_url(data: &Value, id: &str, name: &str) -> Result<Url> {
    data.get("playback")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("canUse").and_then(Value::as_str) == Some("true"))
        .filter_map(|item| item.get("url").and_then(Value::as_str))
        .find_map(|url| super::json_ld::url_or_none(url).and_then(|url| Url::parse(&url).ok()))
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{name}] {id}: Unable to extract playback URL; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

/// `_download_json` of a BFF playable.
fn playable(client: &Client, id: &str, name: &'static str) -> Result<Value> {
    download_json(client, &Url::parse(&format!("{PLAYABLES}{id}"))?, name, id)
}

/// One `traverse_obj` dict template entry: the output key, the path to its
/// value and whether the value is a URL (`url_or_none`) or text (`str`).
type Spec = (&'static str, &'static [&'static str], bool);

/// The `thumbnail`, `description` and `title` templates most pages share.
const STANDARD: [Spec; 3] = [
    ("thumbnail", &["image", "url"], true),
    ("description", &["description"], false),
    ("title", &["title"], false),
];

/// `traverse_obj(meta, {key: path})`: the keys whose value is present (and a
/// valid URL where one is required) in template order.
fn traverse_fields(meta: &Value, specs: &[Spec]) -> Vec<(&'static str, Value)> {
    specs
        .iter()
        .filter_map(|(key, path, is_url)| {
            let value = path
                .iter()
                .try_fold(meta, |value, step| value.get(*step))
                .and_then(Value::as_str)?;
            let value = if *is_url {
                super::json_ld::url_or_none(value)?
            } else {
                value.to_owned()
            };
            Some((*key, Value::String(value)))
        })
        .collect()
}

/// Audio-only extensions: their format has `vcodec: none`.
const AUDIO_EXTS: [&str; 6] = ["aac", "mp3", "m4a", "ogg", "opus", "flac"];

/// `determine_ext(url)` (the dict has no `ext` of its own).
fn ext_of(media: &Url) -> String {
    super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into())
}

/// A direct-media info (`url`, `ext`, `vcodec` as its format), with the
/// dict's `fields` in upstream order (`id` among them).
fn direct_info(
    page: &Url,
    media: &Url,
    ext: &str,
    (extractor, key): (&str, &str),
    fields: Vec<(&str, Value)>,
) -> VideoInfo {
    let mut info = super::direct::with_format(
        super::direct::build_at(media.clone(), media, ext.to_owned()),
        "",
    );
    for format in &mut info.formats {
        format.ext = ext.to_owned();
        format.format_id = String::new();
        if AUDIO_EXTS.contains(&ext) {
            format.vcodec = Some("none".into());
        }
    }
    info.format_id = None;
    info.webpage_url = page.clone();
    info.direct = None;
    info.extractor = extractor.to_owned();
    info.extractor_key = key.to_owned();
    let mut keys = Vec::new();
    for (name, value) in fields {
        keys.push(name.to_owned());
        match name {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "thumbnail" => info.metadata.thumbnail = value.as_str().map(str::to_owned),
            "description" => info.metadata.description = value.as_str().map(str::to_owned),
            // Already set from the arguments (only their key order matters here).
            "extractor" | "extractor_key" | "webpage_url" => {}
            _ => crate::metadata_parser::set_field(&mut info, name, value),
        }
    }
    crate::info_order::mark_info(info, &format!("untyped-dict;{}", keys.join(",")))
}

/// `_match_id`-style progress lines for an extractor's page.
fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// A `_type: playlist` result: `{'_type', 'id', 'entries', **metadata}` (the
/// `thumbnail` and `description` are the playlist dict's extra keys).
fn playlist_result(
    (id, name): (&str, &str),
    url: &Url,
    entries: Vec<VideoInfo>,
    fields: Vec<(&'static str, Value)>,
) -> ExtractResult {
    let mut title = String::new();
    let mut extra = Map::new();
    let mut keys = vec!["_type".to_owned(), "id".to_owned(), "entries".to_owned()];
    for (key, value) in fields {
        keys.push(key.to_owned());
        match key {
            "title" => title = value.as_str().unwrap_or_default().to_owned(),
            _ => {
                extra.insert(key.to_owned(), value);
            }
        }
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("youtube-tab-plain;{}", keys.join(",")).into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id: id.to_owned(),
        title,
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: name.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

/// The upstream `_entries` of a podcast or catch-up show: one direct MP3
/// (or M4A) per block, `webpage_url` its episode page.
fn audio_entry(client: &Client, path: &str, block: &Value, video_id: &str) -> Result<VideoInfo> {
    let entry_id = block
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(video_id, "id"))?
        .to_owned();
    let data = playable(client, &entry_id, "GlobalPlayerAudio")?;
    let media = playback_url(&data, &entry_id, "GlobalPlayerAudio")?;
    let page = Url::parse(&format!(
        "https://www.globalplayer.com/{path}episodes/{entry_id}"
    ))?;
    let mut fields = vec![
        ("id", entry_id.clone().into()),
        ("url", media.as_str().into()),
        ("vcodec", "none".into()),
        ("extractor", EPISODE_NAME.into()),
        ("extractor_key", EPISODE_KEY.into()),
        ("webpage_url", Value::Null),
    ];
    fields.extend(block_fields(block));
    Ok(direct_info(
        &page,
        &media,
        &ext_of(&media),
        (EPISODE_NAME, EPISODE_KEY),
        fields,
    ))
}

/// A block's `image.url`, `description` and `title` (the entry dict's keys).
fn block_fields(block: &Value) -> Vec<(&'static str, Value)> {
    traverse_fields(block, &STANDARD)
}

impl Extractor for GlobalPlayerLive {
    fn key(&self) -> &'static str {
        "GlobalPlayerLive"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = LIVE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce("GlobalPlayerLive", url);
        let station = page_props(client, url, &video_id, "GlobalPlayerLive")?
            .get("station")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "station"))?;
        let station_id = station
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&video_id, "id"))?
            .to_owned();
        let data = playable(client, &station_id, "GlobalPlayerLive")?;
        let media = playback_url(&data, &station_id, "GlobalPlayerLive")?;
        let mut fields = vec![
            ("id", station_id.clone().into()),
            ("url", media.as_str().into()),
            ("ext", "aac".into()),
            ("vcodec", "none".into()),
            ("is_live", Value::Bool(true)),
        ];
        fields.extend(traverse_fields(
            &station,
            &[
                ("thumbnail", &["brandLogo"], true),
                ("description", &["tagline"], false),
                ("title", &["name"], false),
            ],
        ));
        let info = direct_info(
            url,
            &media,
            "aac",
            ("GlobalPlayerLive", "GlobalPlayerLive"),
            fields,
        );
        Ok(Some(ExtractResult::Video(info)))
    }
}

impl Extractor for GlobalPlayerLivePlaylist {
    fn key(&self) -> &'static str {
        "GlobalPlayerLivePlaylist"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = LIVE_PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce("GlobalPlayerLivePlaylist", url);
        let meta = page_props(client, url, &video_id, "GlobalPlayerLivePlaylist")?
            .get("playlistData")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "playlistData"))?;
        let stream = meta
            .get("streamUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&video_id, "streamUrl"))?
            .to_owned();
        let media = Url::parse(&stream)?;
        let mut fields = vec![
            ("url", stream.clone().into()),
            ("ext", "aac".into()),
            ("vcodec", "none".into()),
            ("id", video_id.clone().into()),
            ("is_live", Value::Bool(true)),
        ];
        fields.extend(traverse_fields(
            &meta,
            &[
                ("thumbnail", &["image"], true),
                ("description", &["description"], false),
                ("title", &["title"], false),
            ],
        ));
        let info = direct_info(
            url,
            &media,
            "aac",
            ("GlobalPlayerLivePlaylist", "GlobalPlayerLivePlaylist"),
            fields,
        );
        Ok(Some(ExtractResult::Video(info)))
    }
}

impl Extractor for GlobalPlayerAudio {
    fn key(&self) -> &'static str {
        "GlobalPlayerAudio"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = AUDIO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let path = found["path"].to_owned();
        announce("GlobalPlayerAudio", url);
        let props = page_props(client, url, &video_id, "GlobalPlayerAudio")?;
        let show = if found.name("podcast").is_some() {
            props
                .get("podcastInfo")
                .ok_or_else(|| key_error(&video_id, "podcastInfo"))?
        } else {
            props
                .get("catchupShow")
                .or_else(|| props.get("catchupInfo"))
                .ok_or_else(|| key_error(&video_id, "catchupInfo"))?
        };
        let meta = show
            .get("metadata")
            .ok_or_else(|| key_error(&video_id, "metadata"))?;
        let blocks = show
            .get("blocks")
            .and_then(|blocks| blocks.get(1))
            .and_then(|block| block.get("items"))
            .and_then(Value::as_array)
            .ok_or_else(|| key_error(&video_id, "blocks"))?;
        let mut entries = Vec::new();
        for block in blocks {
            entries.push(audio_entry(client, &path, block, &video_id)?);
        }
        Ok(Some(playlist_result(
            (&video_id, "GlobalPlayerAudio"),
            url,
            entries,
            traverse_fields(meta, &STANDARD),
        )))
    }
}

impl Extractor for GlobalPlayerAudioEpisode {
    fn key(&self) -> &'static str {
        EPISODE_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = EPISODE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(EPISODE_NAME, url);
        let props = page_props(client, url, &video_id, "GlobalPlayerAudioEpisode")?;
        let key = if found.name("podcast").is_some() {
            "podcastEpisode"
        } else {
            "catchupEpisode"
        };
        let meta = props
            .get(key)
            .and_then(|episode| episode.get("metadata"))
            .ok_or_else(|| key_error(&video_id, key))?;
        let data = playable(client, &video_id, "GlobalPlayerAudioEpisode")?;
        let media = playback_url(&data, &video_id, "GlobalPlayerAudioEpisode")?;
        let mut fields = vec![
            ("id", video_id.clone().into()),
            ("url", media.as_str().into()),
            ("vcodec", "none".into()),
        ];
        fields.extend(traverse_fields(meta, &STANDARD));
        Ok(Some(ExtractResult::Video(direct_info(
            url,
            &media,
            &ext_of(&media),
            (EPISODE_NAME, EPISODE_KEY),
            fields,
        ))))
    }
}

impl Extractor for GlobalPlayerVideo {
    fn key(&self) -> &'static str {
        "GlobalPlayerVideo"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce("GlobalPlayerVideo", url);
        let meta = page_props(client, url, &video_id, "GlobalPlayerVideo")?
            .get("videoData")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "videoData"))?;
        let media = meta
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .and_then(|media| Url::parse(&media).ok())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: No video formats found!; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let mut fields = vec![
            ("id", video_id.clone().into()),
            ("url", media.as_str().into()),
        ];
        fields.extend(traverse_fields(&meta, &STANDARD));
        Ok(Some(ExtractResult::Video(direct_info(
            url,
            &media,
            &ext_of(&media),
            ("GlobalPlayerVideo", "GlobalPlayerVideo"),
            fields,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(live_suitable("https://www.globalplayer.com/live/heart/uk/"));
        assert!(live_playlist_suitable(
            "https://www.globalplayer.com/playlists/8bLk/"
        ));
        assert!(audio_suitable(
            "https://www.globalplayer.com/podcasts/42KuaM/"
        ));
        assert!(audio_suitable(
            "https://www.globalplayer.com/catchup/lbc/uk/46vyD7z/"
        ));
        assert!(episode_suitable(
            "https://www.globalplayer.com/podcasts/episodes/7DrorSc/"
        ));
        assert!(video_suitable(
            "https://www.globalplayer.com/videos/2JsSZ7Gm2uP/"
        ));
        assert!(!audio_suitable(
            "https://www.globalplayer.com/podcasts/episodes/7DrorSc/"
        ));
    }
}
