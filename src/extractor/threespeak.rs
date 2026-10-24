//! Upstream `ThreeSpeakIE` and `ThreeSpeakUserIE`
//! (`yt_dlp/extractor/threespeak.py`): a watch page's doubly encoded
//! `JSON.parse('...')` post (its `ogvideo` and IPFS HLS playlists and the
//! original MP4 file), and a user page's `data-payout` videos as a playlist.
use super::{
    Extractor, dict_format, dict_info, download_webpage, extract_m3u8_master_optional,
    search_warning,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct ThreeSpeak;
pub struct ThreeSpeakUser;

const NAME: &str = "ThreeSpeak";
const USER_NAME: &str = "ThreeSpeakUser";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?3speak\.tv/watch\?v=[^/]+/(?P<id>[^/$&#?]+)").unwrap()
});
static USER_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?3speak\.tv/user/(?P<id>[^/$&?#]+)").unwrap());
static JSON_PARSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"JSON\.parse\('([^']+)'\)").unwrap());
static OG_M3U8: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<meta\s?property="ogvideo"\s?content="([^"]+)">"#).unwrap());
static PAYOUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-payout\s?=\s?"([^"]+)""#).unwrap());

/// `ThreeSpeakIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `ThreeSpeakUserIE._match_id`.
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

fn parse_error(id: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Failed to parse JSON; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `try_get(value, lambda x: x[k1][k2]...)`: string keys on objects,
/// integer keys on arrays (and strings, as Python indexing does).
fn try_get(value: &Value, path: &[&str]) -> Value {
    let mut current = value.clone();
    for key in path {
        current = match (&current, key.parse::<usize>()) {
            (Value::Object(map), _) => match map.get(*key) {
                Some(next) => next.clone(),
                None => return Value::Null,
            },
            (Value::Array(items), Ok(index)) => match items.get(index) {
                Some(next) => next.clone(),
                None => return Value::Null,
            },
            (Value::String(text), Ok(index)) => match text.chars().nth(index) {
                Some(next) => next.to_string().into(),
                None => return Value::Null,
            },
            _ => return Value::Null,
        };
    }
    current
}

fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

impl Extractor for ThreeSpeak {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        // `_html_search_regex` (fatal): the match through `clean_html`.
        let json_str = JSON_PARSE
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract json; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        // The JSON string itself is escaped, hence the double parsing.
        let decoded: Value =
            serde_json::from_str(&format!("\"{json_str}\"")).map_err(|_| parse_error(&id))?;
        let data_json: Value = match &decoded {
            Value::String(text) => serde_json::from_str(text).map_err(|_| parse_error(&id))?,
            _ => return Err(parse_error(&id)),
        };
        let metadata = data_json
            .get("json_metadata")
            .ok_or_else(|| super::clyp::key_error(&id, "json_metadata"))?;
        let video_json: Value = match metadata {
            Value::String(text) => serde_json::from_str(text).map_err(|_| parse_error(&id))?,
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "3Speak json_metadata values that are not strings are not supported natively"
                        .into(),
                ));
            }
        };

        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let og_m3u8 = OG_M3U8
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned());
        if og_m3u8.is_none() {
            search_warning(NAME, "og m3u8");
        }
        let mut manifests = Vec::new();
        if let Some(og_m3u8) = og_m3u8.filter(|url| !url.is_empty()) {
            manifests.push((og_m3u8, "https"));
        }
        let ipfs = try_get(&video_json, &["video", "info", "ipfs"]);
        if truthy(&ipfs) {
            let Value::String(ipfs) = ipfs else {
                return Err(Error::UnsupportedHtml5(
                    "3Speak IPFS values that are not strings are not supported natively".into(),
                ));
            };
            manifests.push((format!("https://ipfs.3speak.tv/ipfs/{ipfs}"), "ipfs"));
        }
        for (manifest, m3u8_id) in manifests {
            let manifest = Url::parse(&manifest)?;
            if let Some(stream) = extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &id,
                Some(m3u8_id),
                true,
                false,
            ) {
                formats.extend(stream.formats);
                super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        }
        let mp4_file = try_get(&video_json, &["video", "info", "file"]);
        if truthy(&mp4_file) {
            let file = match &mp4_file {
                Value::String(file) => file.clone(),
                other => other.to_string(),
            };
            formats.push(dict_format(
                vec![
                    (
                        "url",
                        format!("https://threespeakvideo.b-cdn.net/{id}/{file}").into(),
                    ),
                    ("ext", "mp4".into()),
                    ("format_id", "https-mp4".into()),
                    (
                        "duration",
                        try_get(&video_json, &["video", "info", "duration"]),
                    ),
                    (
                        "filesize",
                        try_get(&video_json, &["video", "info", "filesize"]),
                    ),
                    ("quality", 11.into()),
                    ("format_note", "Original file".into()),
                ],
                NAME,
                &id,
            )?);
        }

        let title = data_json
            .get("title")
            .filter(|title| truthy(title))
            .or_else(|| data_json.get("root_title"))
            .cloned()
            .unwrap_or(Value::Null);
        let upload_date = data_json
            .get("created")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_strdate(date, true))
            .map_or(Value::Null, Value::from);
        let mut fields = vec![("id", Value::from(id.clone()))];
        if title.is_string() {
            fields.push(("title", title));
        } else if !title.is_null() {
            return Err(Error::UnsupportedHtml5(
                "3Speak titles that are not strings are not supported natively".into(),
            ));
        }
        fields.extend([
            (
                "uploader",
                data_json.get("author").cloned().unwrap_or(Value::Null),
            ),
            (
                "description",
                try_get(&video_json, &["video", "content", "description"]),
            ),
            ("tags", try_get(&video_json, &["video", "content", "tags"])),
            ("thumbnail", try_get(&video_json, &["image", "0"])),
            ("upload_date", upload_date),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ]);
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

/// `url_result(f'https://3speak.tv/watch?v={video}', ie=ThreeSpeakIE.ie_key())`.
fn video_entry(video: &str) -> Result<VideoInfo> {
    let target = format!("https://3speak.tv/watch?v={video}");
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // `ie.get_temp_id(url)`: the watch URL's `_match_id`.
    info.id = VALID_URL
        .captures(&target)
        .map(|found| found["id"].to_owned())
        .unwrap_or_default();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": {"ie_key": NAME, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for ThreeSpeakUser {
    fn key(&self) -> &'static str {
        USER_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = USER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(USER_NAME);
        crate::screen::screen(&format!(
            "[{USER_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, USER_NAME, &id)?;
        let entries = PAYOUT
            .captures_iter(&webpage)
            .map(|found| video_entry(&found[1]))
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
            extractor: USER_NAME.into(),
            extractor_key: USER_NAME.into(),
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
        assert!(suitable("https://3speak.tv/watch?v=dannyshine/wjgoxyfy"));
        assert!(!suitable("https://3speak.tv/watch?v=dannyshine"));
        assert!(user_suitable("https://3speak.tv/user/theycallmedan"));
        assert!(!user_suitable("https://3speak.tv/watch?v=a/b"));
    }

    #[test]
    fn try_get_indexes_like_python() {
        let value = json!({"image": ["a.png"], "s": "xyz"});
        assert_eq!(try_get(&value, &["image", "0"]), json!("a.png"));
        assert_eq!(try_get(&value, &["s", "1"]), json!("y"));
        assert_eq!(try_get(&value, &["image", "3"]), Value::Null);
    }
}
