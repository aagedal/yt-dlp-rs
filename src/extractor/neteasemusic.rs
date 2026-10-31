//! Upstream `NetEaseMusicMvIE`, `NetEaseMusicProgramIE` and
//! `NetEaseMusicDjRadioIE` (`yt_dlp/extractor/neteasemusic.py`): an MV's
//! detail API, a program's main song from the encrypted `eapi` player URL
//! (or its songs as url results), and a radio's programs as url results.
//! `netease:song` (geo-restricted here), the album, singer and playlist
//! classes and logged-in requests are not ported.
use super::{Extractor, dict_format, dict_info, nrk, post_json_note_headers};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoFormat, VideoInfo};
use aes::cipher::{BlockCipherEncrypt, KeyInit};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NetEaseMv;
pub struct NetEaseProgram;
pub struct NetEaseDjRadio;

const MV_NAME: &str = "netease:mv";
const MV_KEY: &str = "NetEaseMusicMv";
const PROGRAM_NAME: &str = "netease:program";
const PROGRAM_KEY: &str = "NetEaseMusicProgram";
const DJRADIO_NAME: &str = "netease:djradio";
const DJRADIO_KEY: &str = "NetEaseMusicDjRadio";
const API_BASE: &str = "http://music.163.com/api/";
const EAPI_BASE: &str = "https://interface3.music.163.com/";
const EAPI_KEY: &[u8; 16] = b"e82ckenh8dichen8";
const PAGE_SIZE: i64 = 1000;
/// `NetEaseMusicBaseIE._LEVELS`, in the order the levels are requested.
const LEVELS: [&str; 8] = [
    "standard", "higher", "exhigh", "lossless", "hires", "jyeffect", "jymaster", "sky",
];

static MV_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://music\.163\.com/(?:#/)?mv\?id=(?P<id>[0-9]+)").unwrap()
});
static PROGRAM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://music\.163\.com/(?:#/)?(?:dj|program)\?id=(?P<id>[0-9]+)").unwrap()
});
static DJRADIO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://music\.163\.com/(?:#/)?djradio\?id=(?P<id>[0-9]+)").unwrap()
});

/// `NetEaseMusicMvIE._match_valid_url`.
pub fn mv_suitable(url: &str) -> bool {
    MV_URL.is_match(url)
}

/// `NetEaseMusicProgramIE._match_valid_url`.
pub fn program_suitable(url: &str) -> bool {
    PROGRAM_URL.is_match(url)
}

/// `NetEaseMusicDjRadioIE._match_valid_url`.
pub fn djradio_suitable(url: &str) -> bool {
    DJRADIO_URL.is_match(url)
}

/// `int_or_none(value, scale=...)`.
fn int_or_none(value: Option<&Value>, scale: i64) -> Option<i64> {
    let number = match value? {
        Value::Number(number) => number.as_i64()?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    Some(number / scale)
}

/// `str` of a field.
fn str_field(value: Option<&Value>) -> Option<String> {
    value?.as_str().map(str::to_owned)
}

/// `str_or_none` of an ID.
fn id_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `url_or_none` of a field.
fn url_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    (text.starts_with("https://") || text.starts_with("http://")).then(|| text.to_owned())
}

/// `clean_html`-free `filter`: a non-empty string field.
fn filtered(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `_query_api`: the JSON with its `code` checked.
fn query_api(
    client: &Client,
    name: &'static str,
    endpoint: &str,
    id: &str,
    note: &str,
) -> Result<Value> {
    let url = Url::parse(&format!("{API_BASE}{endpoint}"))?;
    let result =
        super::download_json_note_headers(client, &url, name, id, note, &[("Referer", API_BASE)])?;
    let code = result.get("code").and_then(Value::as_i64);
    let message = str_field(result.get("message")).unwrap_or_default();
    if code == Some(-462) {
        return Err(Error::Reported(format!(
            "[{name}] {id}: Login required to download: {message}"
        )));
    }
    if code != Some(200) {
        let code = code.map_or_else(|| "None".to_owned(), |code| code.to_string());
        return Err(Error::Reported(format!(
            "[{name}] {id}: Failed to get meta info: {code} {message}; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    Ok(result)
}

/// `_create_eapi_cipher`'s `params=` body.
fn eapi_body(api_path: &str, query_body: &Value, cookies: &Map<String, Value>) -> String {
    let mut body = query_body.as_object().cloned().unwrap_or_default();
    body.insert("header".into(), Value::Object(cookies.clone()));
    let request_text = Value::Object(body).to_string();
    let message = format!("nobody{api_path}use{request_text}md5forencrypt");
    let digest = crate::md5::hex(message.as_bytes());
    let mut data =
        format!("{api_path}-36cd479b6b5-{request_text}-36cd479b6b5-{digest}").into_bytes();
    // PKCS#7: always 1..=16 bytes of padding.
    let padding = 16 - data.len() % 16;
    data.extend(std::iter::repeat_n(padding as u8, padding));
    let cipher = aes::Aes128::new(EAPI_KEY.into());
    for chunk in data.as_chunks_mut::<16>().0 {
        let mut block = aes::cipher::Block::<aes::Aes128>::default();
        block.copy_from_slice(chunk);
        cipher.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
    }
    let hex: String = data.iter().map(|byte| format!("{byte:02X}")).collect();
    format!("params={hex}")
}

/// `_download_eapi_json`: an encrypted POST to the `eapi` endpoint.
fn download_eapi_json(client: &Client, path: &str, id: &str, query_body: &Value) -> Result<Value> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let mut cookies = Map::new();
    for (key, value) in [
        ("osver", "undefined"),
        ("deviceId", "undefined"),
        ("appver", "8.0.0"),
        ("versioncode", "140"),
        ("mobilename", "undefined"),
        ("buildver", "1623435496"),
        ("resolution", "1920x1080"),
        ("__csrf", ""),
        ("os", "pc"),
        ("channel", "undefined"),
    ] {
        cookies.insert(key.into(), value.into());
    }
    cookies.insert(
        "requestId".into(),
        format!("{millis}_{:04}", millis % 1000).into(),
    );
    let cookie_header = cookies
        .iter()
        .map(|(key, value)| format!("{key}={}", value.as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("; ");
    let body = eapi_body(&format!("/api{path}"), query_body, &cookies);
    let url = Url::parse(&format!("{EAPI_BASE}eapi{path}"))?;
    post_json_note_headers(
        client,
        &url,
        PROGRAM_NAME,
        id,
        "Downloading song URL info",
        &[
            ("Referer", "https://music.163.com"),
            ("Cookie", cookie_header.as_str()),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        body,
    )
}

/// `_extract_formats(info)`: one format per level the API answers for.
fn song_formats(client: &Client, name: &'static str, song_id: i64) -> Result<Vec<VideoFormat>> {
    let id = song_id.to_string();
    let mut formats = Vec::new();
    for level in LEVELS {
        let response = download_eapi_json(
            client,
            "/song/enhance/player/url/v1",
            &id,
            &json!({"ids": format!("[{song_id}]"), "level": level, "encodeType": "flac"}),
        )?;
        let song = response
            .get("data")
            .and_then(Value::as_array)
            .and_then(|songs| {
                songs
                    .iter()
                    .find(|song| url_text(song.get("url")).is_some())
            });
        let Some(song) = song else {
            break;
        };
        let actual_level = str_field(song.get("level")).filter(|level| !level.is_empty());
        if let Some(actual_level) = &actual_level
            && actual_level != level
        {
            if level == "lossless" || level == "jymaster" {
                break;
            }
            continue;
        }
        let mut fields = vec![
            ("url", song["url"].clone()),
            ("format_id", level.into()),
            ("vcodec", "none".into()),
        ];
        if let Some(ext) = str_field(song.get("type")) {
            fields.push(("ext", ext.into()));
        }
        if let Some(abr) = int_or_none(song.get("br"), 1000) {
            fields.push(("abr", abr.into()));
        }
        if let Some(filesize) = int_or_none(song.get("size"), 1) {
            fields.push(("filesize", filesize.into()));
        }
        let mut format = dict_format(fields, name, &id)?;
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
        if actual_level.is_none() {
            // Only one level is available when the API does not name one
            // (`netease:program`).
            break;
        }
    }
    if formats.is_empty() {
        return Err(Error::Reported(format!(
            "[{name}] {id}: No media links found; possibly due to geo restriction\nThis video is available in China.\nYou might want to use a VPN or a proxy server (with --proxy) to workaround."
        )));
    }
    Ok(formats)
}

/// `_yes_playlist`'s answer for a playlist with songs.
fn yes_playlist(name: &str, id: &str, label: &str) -> bool {
    if super::no_playlist() {
        crate::screen::screen(&format!(
            "[{name}] Downloading just the song {id} because of --no-playlist"
        ));
        false
    } else {
        crate::screen::screen(&format!(
            "[{name}] Downloading {label} {id} - add --no-playlist to download just the song"
        ));
        true
    }
}

/// A `playlist_result` with the given keyword fields first.
fn playlist(
    id: String,
    url: &Url,
    (name, key): (&str, &str),
    fields: Vec<(&str, Value)>,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut extra = Map::new();
    let mut shape = Vec::new();
    let mut title = String::new();
    for (field, value) in fields {
        shape.push(field.to_owned());
        if field == "title" {
            // The playlist's own title; only its key keeps its position.
            title = value.as_str().unwrap_or_default().to_owned();
        } else {
            extra.insert(field.into(), value);
        }
    }
    shape.push("id".into());
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!(
            "playlist-dict;{}{}",
            shape.join(","),
            crate::info_order::GENERATOR
        )
        .into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title,
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

impl Extractor for NetEaseMv {
    fn key(&self) -> &'static str {
        MV_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = MV_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let mv_id = found["id"].to_owned();
        crate::screen::set_extractor(MV_NAME);
        crate::screen::screen(&format!(
            "[{MV_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let response = query_api(
            client,
            MV_NAME,
            &format!("mv/detail?id={mv_id}&type=mp4"),
            &mv_id,
            "Downloading mv info",
        )?;
        let info = response
            .get("data")
            .ok_or_else(|| key_error(&mv_id, "data"))?;

        let mut formats = Vec::new();
        for (brs, mv_url) in info
            .get("brs")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let height = brs.parse::<i64>().ok();
            formats.push(dict_format(
                vec![
                    ("url", mv_url.clone()),
                    ("ext", "mp4".into()),
                    ("format_id", format!("{brs}p").into()),
                    ("height", height.map_or(Value::Null, Value::from)),
                ],
                MV_NAME,
                &mv_id,
            )?);
        }

        let mut creators: Vec<Value> = info
            .get("artists")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|artist| artist.get("name").and_then(Value::as_str))
            .map(Value::from)
            .collect();
        if creators.is_empty() {
            creators.push(info.get("artistName").cloned().unwrap_or(Value::Null));
        }

        // `traverse_obj(info, {...}, get_all=False)` in the upstream key order.
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", mv_id.clone().into()),
            ("formats", Value::Null),
            ("creators", creators.into()),
        ];
        if let Some(title) = str_field(info.get("name")) {
            fields.push(("title", title.into()));
        }
        let description = filtered(str_field(info.get("desc")))
            .or_else(|| filtered(str_field(info.get("briefDesc"))));
        if let Some(description) = description {
            fields.push(("description", description.into()));
        }
        if let Some(upload_date) = str_field(info.get("publishTime"))
            .and_then(|date| crate::dates::unified_strdate(&date, false))
        {
            fields.push(("upload_date", upload_date.into()));
        }
        if let Some(thumbnail) = url_text(info.get("cover")) {
            fields.push(("thumbnail", thumbnail.into()));
        }
        if let Some(duration) = int_or_none(info.get("duration"), 1000) {
            fields.push(("duration", duration.into()));
        }
        for (field, key) in [
            ("view_count", "playCount"),
            ("like_count", "likeCount"),
            ("comment_count", "commentCount"),
        ] {
            if let Some(count) = int_or_none(info.get(key), 1) {
                fields.push((field, count.into()));
            }
        }
        Ok(Some(dict_info(url, (MV_NAME, MV_KEY), fields, formats)))
    }
}

impl Extractor for NetEaseProgram {
    fn key(&self) -> &'static str {
        PROGRAM_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PROGRAM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let program_id = found["id"].to_owned();
        crate::screen::set_extractor(PROGRAM_NAME);
        crate::screen::screen(&format!(
            "[{PROGRAM_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let response = query_api(
            client,
            PROGRAM_NAME,
            &format!("dj/program/detail?id={program_id}"),
            &program_id,
            "Downloading program info",
        )?;
        let info = response
            .get("program")
            .ok_or_else(|| key_error(&program_id, "program"))?;
        let main_song = info
            .get("mainSong")
            .ok_or_else(|| key_error(&program_id, "mainSong"))?;
        let main_id = main_song
            .get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| key_error(&program_id, "id"))?;

        // `metainfo`, in the upstream key order.
        let mut metainfo: Vec<(&str, Value)> = Vec::new();
        if let Some(title) = str_field(info.get("name")) {
            metainfo.push(("title", title.into()));
        }
        if let Some(description) = str_field(info.get("description")) {
            metainfo.push(("description", description.into()));
        }
        if let Some(creator) = info.get("dj").and_then(|dj| str_field(dj.get("brand"))) {
            metainfo.push(("creator", creator.into()));
        }
        if let Some(thumbnail) = url_text(info.get("coverUrl")) {
            metainfo.push(("thumbnail", thumbnail.into()));
        }
        if let Some(timestamp) = int_or_none(info.get("createTime"), 1000) {
            metainfo.push(("timestamp", timestamp.into()));
        }

        let songs: Vec<&Value> = info
            .get("songs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();
        if !songs.is_empty() && yes_playlist(PROGRAM_NAME, &program_id, "playlist") {
            let mut entries = Vec::new();
            for song in std::iter::once(main_song).chain(songs.iter().copied()) {
                let Some(song_id) = id_text(song.get("id")) else {
                    continue;
                };
                let title = str_field(song.get("name"));
                entries.push(nrk::url_entry(
                    &format!("http://music.163.com/#/song?id={song_id}"),
                    "NetEaseMusic",
                    Some(&song_id),
                    None,
                    title,
                )?);
            }
            return Ok(Some(playlist(
                program_id,
                url,
                (PROGRAM_NAME, PROGRAM_KEY),
                metainfo,
                entries,
            )));
        }

        let formats = song_formats(client, PROGRAM_NAME, main_id)?;
        let mut fields: Vec<(&str, Value)> =
            vec![("id", main_id.to_string().into()), ("formats", Value::Null)];
        if let Some(duration) = int_or_none(main_song.get("duration"), 1000) {
            fields.push(("duration", duration.into()));
        }
        fields.extend(metainfo);
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (PROGRAM_NAME, PROGRAM_KEY),
            fields,
            formats,
        ))))
    }
}

impl Extractor for NetEaseDjRadio {
    fn key(&self) -> &'static str {
        DJRADIO_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = DJRADIO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let dj_id = found["id"].to_owned();
        crate::screen::set_extractor(DJRADIO_NAME);
        crate::screen::screen(&format!(
            "[{DJRADIO_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let mut metainfo: Vec<(&str, Value)> = Vec::new();
        let mut entries = Vec::new();
        let mut offset = 0;
        loop {
            let info = query_api(
                client,
                DJRADIO_NAME,
                &format!(
                    "dj/program/byradio?asc=false&limit={PAGE_SIZE}&radioId={dj_id}&offset={offset}"
                ),
                &dj_id,
                &format!("Downloading dj programs - {offset}"),
            )?;
            let programs = info
                .get("programs")
                .and_then(Value::as_array)
                .ok_or_else(|| key_error(&dj_id, "programs"))?;
            for program in programs {
                let Some(program_id) = id_text(program.get("id")) else {
                    return Err(key_error(&dj_id, "id"));
                };
                entries.push(nrk::url_entry(
                    &format!("http://music.163.com/#/program?id={program_id}"),
                    PROGRAM_KEY,
                    Some(&program_id),
                    None,
                    str_field(program.get("name")),
                )?);
            }
            if metainfo.is_empty()
                && let Some(radio) = programs.first().and_then(|program| program.get("radio"))
            {
                if let Some(title) = str_field(radio.get("name")) {
                    metainfo.push(("title", title.into()));
                }
                if let Some(description) = str_field(radio.get("desc")) {
                    metainfo.push(("description", description.into()));
                }
            }
            if info.get("more").and_then(Value::as_bool) != Some(true) {
                break;
            }
            offset += PAGE_SIZE;
        }

        Ok(Some(playlist(
            dj_id,
            url,
            (DJRADIO_NAME, DJRADIO_KEY),
            metainfo,
            entries,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(mv_suitable("https://music.163.com/#/mv?id=10958064"));
        assert!(!mv_suitable("https://music.163.com/#/song?id=550136151"));
        assert!(program_suitable(
            "http://music.163.com/#/program?id=10109055"
        ));
        assert!(program_suitable("https://music.163.com/#/dj?id=3706179315"));
        assert!(djradio_suitable("http://music.163.com/#/djradio?id=42"));
        assert!(!djradio_suitable("http://music.163.com/#/program?id=42"));
    }

    #[test]
    fn eapi_body_is_hex_encrypted_params() {
        let body = eapi_body("/api/x", &json!({"ids": "[1]"}), &Map::new());
        assert!(body.starts_with("params="));
        let hex = &body["params=".len()..];
        assert!(hex.len().is_multiple_of(32) && hex.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
