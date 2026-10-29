//! Upstream `EggsIE` and `EggsArtistIE` (`yt_dlp/extractor/eggs.py`): a
//! song's API record as a direct audio result (or a YouTube-linked song as
//! a followed result), and an artist's songs as a playlist.
use super::{Extractor, download_json_headers};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::sync::LazyLock;
use url::Url;

pub struct Eggs;
pub struct EggsArtist;

const NAME: &str = "eggs:single";
const KEY: &str = "Eggs";
const ARTIST_NAME: &str = "eggs:artist";
const ARTIST_KEY: &str = "EggsArtist";

static SONG_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://eggs\.mu/artist/[^/?#]+/song/(?P<id>[\da-f-]+)").unwrap()
});
static ARTIST_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://eggs\.mu/artist/(?P<id>\w+)/?(?:[?#&]|$)").unwrap());

/// `EggsIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    SONG_URL.is_match(url)
}

/// `EggsArtistIE._match_valid_url`.
pub fn artist_suitable(url: &str) -> bool {
    ARTIST_URL.is_match(url)
}

/// `secrets.token_hex(8)`, sent as the API's `deviceId`.
fn device_id() -> String {
    format!("{:016x}", RandomState::new().build_hasher().finish())
}

/// `EggsBaseIE._call_api`.
fn call_api(
    client: &Client,
    (name, device): (&'static str, &str),
    endpoint: &str,
    id: &str,
) -> Result<Value> {
    let url = Url::parse(&format!("https://app-front-api.eggs.mu/v1/{endpoint}"))?;
    download_json_headers(
        client,
        &url,
        name,
        id,
        &[
            ("Accept", "*/*"),
            ("apVersion", "8.2.00"),
            ("deviceName", "Android"),
            ("deviceId", device),
        ],
    )
}

/// `traverse_obj(data, (key, {str}))`.
fn text(value: Option<&Value>) -> Option<Value> {
    value.and_then(Value::as_str).map(Value::from)
}

/// `{str_or_none}`: a non-null value's text.
fn str_or_none(value: Option<&Value>) -> Option<Value> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone().into()),
        Some(Value::Number(number)) => Some(number.to_string().into()),
        Some(other) => Some(other.to_string().into()),
    }
}

/// `{int_or_none}`.
fn int_or_none(value: Option<&Value>) -> Option<Value> {
    match value {
        Some(Value::Number(number)) => number.as_i64().map(Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok().map(Value::from),
        _ => None,
    }
}

/// `{url_or_none}`.
fn url_text(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map(Value::from)
}

/// `url_result(target, ie=ie_key)` as a playlist entry.
fn url_entry(target: &str, ie_key: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ie_key.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": ie_key,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// The YouTube-linked song's `url_result`, else the song as a direct audio
/// result (`_extract_music_info`).
enum Music {
    Youtube(String),
    Direct(Box<VideoInfo>),
}

fn extract_music_info(data: &Value) -> Result<Music> {
    if let Some(youtube) =
        url_text(data.get("youtubeUrl")).and_then(|value| value.as_str().map(str::to_owned))
    {
        return Ok(Music::Youtube(youtube));
    }
    let artist = data.get("artist");
    let artist_field = |key: &str| artist.and_then(|artist| artist.get(key));
    let artist_name = str_or_none(artist_field("artistName"));
    let music_id = str_or_none(data.get("musicId"));
    let webpage_url = match (&artist_name, &music_id) {
        (Some(Value::String(name)), Some(Value::String(id))) => {
            Value::from(format!("https://eggs.mu/artist/{name}/song/{id}"))
        }
        _ => Value::Null,
    };
    let music_url = url_text(data.get("musicDataPath"));
    let mut fields: Vec<(&str, Value)> = vec![
        ("id", music_id.clone().unwrap_or(Value::Null)),
        ("vcodec", "none".into()),
        ("webpage_url", webpage_url.clone()),
        ("extractor_key", KEY.into()),
        ("extractor", NAME.into()),
    ];
    // `traverse_obj` drops the missing and `None` keys.
    let optional = [
        ("title", text(data.get("musicTitle"))),
        ("url", music_url.clone()),
        ("uploader", text(artist_field("displayName"))),
        ("uploader_id", str_or_none(artist_field("artistId"))),
        ("thumbnail", url_text(data.get("imageDataPath"))),
        ("view_count", int_or_none(data.get("numberOfMusicPlays"))),
        ("like_count", int_or_none(data.get("numberOfLikes"))),
        ("comment_count", int_or_none(data.get("numberOfComments"))),
        (
            "composers",
            text(data.get("composer")).map(|composer| json!([composer])),
        ),
        (
            "tags",
            data.get("tags")
                .and_then(Value::as_array)
                .map(|tags| Value::Array(tags.iter().filter_map(|tag| text(Some(tag))).collect())),
        ),
        (
            "timestamp",
            text(data.get("releaseDate"))
                .and_then(|date| crate::dates::parse_iso8601(date.as_str()?))
                .map(Value::from),
        ),
        ("artist", text(artist_field("displayName"))),
    ];
    fields.extend(
        optional
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value))),
    );
    let formats = match (&music_url, music_id.as_ref().and_then(Value::as_str)) {
        (Some(music_url), Some(id)) => vec![super::dict_format(
            vec![
                ("url", music_url.clone()),
                ("format_id", "0".into()),
                ("vcodec", "none".into()),
            ],
            NAME,
            id,
        )?],
        _ => Vec::new(),
    };
    let page = Url::parse(webpage_url.as_str().unwrap_or("https://eggs.mu/"))?;
    // The dict is its own only format (`untyped-dict`): its keys in order.
    let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
    let shape = format!("untyped-dict;{}", keys.join(","));
    let info = super::dict_info(&page, (NAME, KEY), fields.clone(), formats);
    Ok(Music::Direct(Box::new(crate::info_order::mark_info(
        info, &shape,
    ))))
}

impl Extractor for Eggs {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SONG_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let song_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let device = device_id();
        let json_data = call_api(
            client,
            (NAME, &device),
            &format!("musics/{song_id}"),
            &song_id,
        )?;
        Ok(Some(match extract_music_info(&json_data)? {
            Music::Youtube(target) => super::follow_url(url, target),
            Music::Direct(info) => ExtractResult::Video(*info),
        }))
    }
}

impl Extractor for EggsArtist {
    fn key(&self) -> &'static str {
        ARTIST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ARTIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let artist_id = found["id"].to_owned();
        crate::screen::set_extractor(ARTIST_NAME);
        crate::screen::screen(&format!(
            "[{ARTIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let device = device_id();
        let artist_data = call_api(
            client,
            (ARTIST_NAME, &device),
            &format!("artists/{artist_id}"),
            &artist_id,
        )?;
        let song_data = call_api(
            client,
            (ARTIST_NAME, &device),
            &format!("artists/{artist_id}/musics"),
            &artist_id,
        )?;
        let mut entries = Vec::new();
        for song in song_data
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|song| song.is_object())
        {
            entries.push(match extract_music_info(song)? {
                Music::Youtube(target) => url_entry(&target, "Youtube")?,
                Music::Direct(info) => *info,
            });
        }

        let title = text(artist_data.get("displayName"))
            .and_then(|value| value.as_str().map(str::to_owned));
        let description = text(artist_data.get("profile"));
        let thumbnail = url_text(artist_data.get("imageDataPath"));
        let mut extra = Map::new();
        if let Some(description) = description {
            extra.insert("description".into(), description);
        }
        if let Some(thumbnail) = thumbnail {
            extra.insert("thumbnail".into(), thumbnail);
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;title,description,thumbnail,id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: artist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: ARTIST_NAME.into(),
            extractor_key: ARTIST_KEY.into(),
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
        assert!(suitable(
            "https://eggs.mu/artist/32_sunny_girl/song/0e95fd1d-4d61-4d5b-8b18-6092c551da90"
        ));
        assert!(artist_suitable("https://eggs.mu/artist/32_sunny_girl"));
        assert!(!artist_suitable(
            "https://eggs.mu/artist/32_sunny_girl/song/0e95fd1d-4d61-4d5b-8b18-6092c551da90"
        ));
    }
}
