//! Upstream `MySpaceIE` and `MySpaceAlbumIE` (`yt_dlp/extractor/myspace.py`):
//! a video's `context` JSON (HLS, RTMP and HTTP streams), a song's
//! `data-*` button attributes, and an album's `music:song` links as `url`
//! entries.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct MySpace;
pub struct MySpaceAlbum;

const NAME: &str = "MySpace";
const ALBUM_NAME: &str = "MySpace:album";
const ALBUM_KEY: &str = "MySpaceAlbum";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://myspace\.com/[^/]+/(?P<mediatype>video/[^/]+/(?P<video_id>\d+)|music/song/[^/?#&]+-(?P<song_id>\d+)-\d+(?:[/?#&]|$))",
    )
    .unwrap()
});
static ALBUM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://myspace\.com/([^/]+)/music/album/(?P<title>.*-)(?P<id>\d+)").unwrap()
});
static PLAYER_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"videoSwf":"([^"?]*)"#).unwrap());
static CONTEXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"context = (\{.*?\});").unwrap());
static TRACK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""music:song" content="(.*?)""#).unwrap());

/// `_match_valid_url` for the song and video IE.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MySpace {what} are not supported natively"))
}

/// `data-NAME=(['"])(?P<data>.*?)\1` within the song's button, `''` when absent.
fn song_attribute(song: &str, name: &str) -> String {
    let pattern = format!(r#"data-{}=(?:"([^"]*)"|'([^']*)')"#, regex::escape(name));
    let found = Regex::new(&pattern).expect("escaped attribute pattern");
    found
        .captures(song)
        .and_then(|found| found.get(1).or_else(|| found.get(2)))
        .map_or_else(String::new, |value| value.as_str().to_owned())
}

/// `formats_from_stream_urls`.
fn formats_from_stream_urls(
    id: &str,
    stream_url: &str,
    hls_stream_url: &str,
    http_stream_url: &str,
    player_url: Option<&str>,
    (width, height): (Option<i64>, Option<i64>),
    is_song: bool,
) -> Result<Vec<crate::VideoFormat>> {
    let vcodec = is_song.then_some("none");
    let mut formats = Vec::new();
    if !hls_stream_url.is_empty() {
        formats.push(dict_format(
            vec![
                ("format_id", "hls".into()),
                ("url", hls_stream_url.into()),
                ("protocol", "m3u8_native".into()),
                ("ext", if is_song { "m4a" } else { "mp4" }.into()),
                ("vcodec", vcodec.map_or(Value::Null, Value::from)),
            ],
            NAME,
            id,
        )?);
    }
    if !stream_url.is_empty()
        && let Some(player_url) = player_url
    {
        let Some((rtmp_url, play_path)) = stream_url.split_once(';') else {
            return Err(Error::UnsupportedHtml5(
                "MySpace RTMP streams without a play path are not supported natively".into(),
            ));
        };
        formats.push(dict_format(
            vec![
                ("format_id", "rtmp".into()),
                ("url", rtmp_url.into()),
                ("play_path", play_path.into()),
                ("player_url", player_url.into()),
                ("protocol", "rtmp".into()),
                ("ext", "flv".into()),
                ("width", width.map_or(Value::Null, Value::from)),
                ("height", height.map_or(Value::Null, Value::from)),
                ("vcodec", vcodec.map_or(Value::Null, Value::from)),
            ],
            NAME,
            id,
        )?);
    }
    if !http_stream_url.is_empty() {
        formats.push(dict_format(
            vec![
                ("format_id", "http".into()),
                ("url", http_stream_url.into()),
                ("width", width.map_or(Value::Null, Value::from)),
                ("height", height.map_or(Value::Null, Value::from)),
                ("vcodec", vcodec.map_or(Value::Null, Value::from)),
            ],
            NAME,
            id,
        )?);
    }
    Ok(formats)
}

/// `int_or_none` of a JSON value.
fn int_value(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn song_info(
    url: &Url,
    webpage: &str,
    video_id: &str,
    player_url: Option<&str>,
) -> Result<Option<VideoInfo>> {
    let pattern = format!(r#"<button.*data-song-id=(?:"{video_id}"|'{video_id}').*"#);
    let song = Regex::new(&pattern).expect("escaped song pattern");
    let Some(song) = song.find(webpage).map(|found| found.as_str().to_owned()) else {
        crate::screen::warning(&format!(
            "[{NAME}] {video_id}: No downloadable song on this page"
        ));
        return Ok(None);
    };
    let stream_url = song_attribute(&song, "stream-url");
    let hls_stream_url = song_attribute(&song, "hls-stream-url");
    let http_stream_url = song_attribute(&song, "http-stream-url");
    let formats = formats_from_stream_urls(
        video_id,
        &stream_url,
        &hls_stream_url,
        &http_stream_url,
        player_url,
        (None, None),
        true,
    )?;
    if formats.is_empty() {
        let vevo_id = song_attribute(&song, "vevo-id");
        let youtube_id = song_attribute(&song, "youtube-id");
        if !vevo_id.is_empty() || !youtube_id.is_empty() {
            return Err(unsupported("songs that hand off to Vevo or YouTube"));
        }
        return Err(Error::Reported(format!(
            "[{NAME}] {video_id}: Found song but don't know how to download it"
        )));
    }
    let title = crate::html::og_search_property(webpage, "title");
    if title.is_none() {
        super::search_warning(NAME, "OpenGraph title");
    }
    let thumbnail = crate::html::og_search_property(webpage, "image");
    if thumbnail.is_none() {
        super::search_warning(NAME, "OpenGraph image");
    }
    let duration = song_attribute(&song, "duration");
    Ok(Some(dict_info(
        url,
        (NAME, NAME),
        vec![
            ("id", video_id.into()),
            ("title", title.map_or(Value::Null, Value::from)),
            ("uploader", song_attribute(&song, "artist-name").into()),
            (
                "uploader_id",
                song_attribute(&song, "artist-username").into(),
            ),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            (
                "duration",
                duration
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .map_or(Value::Null, Value::from),
            ),
            ("formats", Value::Null),
        ],
        formats,
    )))
}

fn video_info(
    url: &Url,
    webpage: &str,
    video_id: &str,
    player_url: Option<&str>,
) -> Result<VideoInfo> {
    let context = CONTEXT
        .captures(webpage)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] Unable to extract context; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    let parsed: Value = serde_json::from_str(&context).map_err(|_| {
        Error::Reported(format!(
            "[{NAME}] {video_id}: Failed to parse JSON; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    let video = parsed
        .get("video")
        .ok_or_else(|| super::clyp::key_error(video_id, "video"))?;
    let title = video
        .get("title")
        .cloned()
        .ok_or_else(|| super::clyp::key_error(video_id, "title"))?;
    let text = |key: &str| video.get(key).and_then(Value::as_str).unwrap_or_default();
    let width = int_value(video.get("width"));
    let height = int_value(video.get("height"));
    let formats = formats_from_stream_urls(
        video_id,
        text("streamUrl"),
        text("hlsStreamUrl"),
        text("mp4StreamUrl"),
        player_url,
        (width, height),
        false,
    )?;
    let timestamp = video
        .get("dateAdded")
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map_or(Value::Null, Value::from);
    Ok(dict_info(
        url,
        (NAME, NAME),
        vec![
            ("id", video_id.into()),
            ("title", title),
            (
                "description",
                video.get("description").cloned().unwrap_or(Value::Null),
            ),
            (
                "thumbnail",
                video.get("imageUrl").cloned().unwrap_or(Value::Null),
            ),
            (
                "uploader",
                video.get("artistName").cloned().unwrap_or(Value::Null),
            ),
            (
                "uploader_id",
                video.get("artistUsername").cloned().unwrap_or(Value::Null),
            ),
            (
                "duration",
                int_value(video.get("duration")).map_or(Value::Null, Value::from),
            ),
            ("timestamp", timestamp),
            ("formats", Value::Null),
        ],
        formats,
    ))
}

impl Extractor for MySpace {
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
        let is_song = found["mediatype"].starts_with("music/song");
        let video_id = found
            .name("video_id")
            .or_else(|| found.name("song_id"))
            .map_or(String::new(), |id| id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `_search_regex(r'videoSwf":"([^"?]*)', ..., fatal=False)`.
        let player_url = PLAYER_URL
            .captures(&webpage)
            .map(|found| found[1].to_owned());
        if player_url.is_none() {
            super::search_warning(NAME, "player URL");
        }
        let info = if is_song {
            song_info(url, &webpage, &video_id, player_url.as_deref())?
        } else {
            Some(video_info(url, &webpage, &video_id, player_url.as_deref())?)
        };
        Ok(info.map(ExtractResult::Video))
    }
}

/// `url_result(target, ie=MySpaceIE.ie_key())` as an album entry.
fn track_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), NAME.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for MySpaceAlbum {
    fn key(&self) -> &'static str {
        ALBUM_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ALBUM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        let display_id = format!("{}{playlist_id}", &found["title"]);
        crate::screen::set_extractor(ALBUM_NAME);
        crate::screen::screen(&format!(
            "[{ALBUM_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, ALBUM_NAME, &display_id)?;
        let tracks: Vec<String> = TRACK
            .captures_iter(&webpage)
            .map(|found| found[1].to_owned())
            .collect();
        if tracks.is_empty() {
            return Err(Error::Reported(format!(
                "{display_id}: No songs found, try using proxy"
            )));
        }
        let entries = tracks
            .iter()
            .map(|track| track_entry(track))
            .collect::<Result<Vec<_>>>()?;
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            super::search_warning(ALBUM_NAME, "OpenGraph title");
        }
        let mut extra = Map::new();
        extra.insert("display_id".into(), display_id.into());
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "typed-playlist-display".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: ALBUM_NAME.into(),
            extractor_key: ALBUM_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// URL matching for `--list-extractors`.
pub fn album_suitable(url: &str) -> bool {
    ALBUM_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://myspace.com/fiveminutestothestage/video/little-big-town/109594919"
        ));
        assert!(suitable(
            "https://myspace.com/killsorrow/music/song/of-weakened-soul...-93388656-103880681"
        ));
        assert!(!suitable("https://myspace.com/killsorrow"));
        assert!(ALBUM_URL.is_match("https://myspace.com/killsorrow/music/album/the-demo-18596029"));
    }

    #[test]
    fn song_attributes_are_quoted_values() {
        let song = r#"<button data-stream-url="" data-artist-name='Killsorrow'>"#;
        assert_eq!(song_attribute(song, "artist-name"), "Killsorrow");
        assert_eq!(song_attribute(song, "stream-url"), "");
        assert_eq!(song_attribute(song, "missing"), "");
    }
}
