//! Upstream `XHamsterIE` and `XHamsterUserIE` (`yt_dlp/extractor/xhamster.py`):
//! a video page's `window.initials` JSON (its `sources`, the xPlayer HLS and
//! standard sources, some of them hex-ciphered with the site's byte
//! generators) and a user's `/videos` pages. The page's old layout (without
//! `initials`) is not supported natively.
use super::{Extractor, dict_format, dict_info, download_webpage_handle};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::sync::LazyLock;
use url::Url;

pub struct XHamster;
pub struct XHamsterUser;

const NAME: &str = "XHamster";
const USER_NAME: &str = "XHamsterUser";

const DOMAINS: &str =
    r"(?:xhamster\.(?:com|one|desi)|xhms\.pro|xhamster\d+\.(?:com|desi)|xhday\.com|xhvid\.com)";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^https?://(?:[^/?#]+\.)?{DOMAINS}/(?:movies/(?P<id>[\dA-Za-z]+)/(?P<display_id>[^/]*)\.html|videos/(?P<display_id_2>[^/]*)-(?P<id_2>[\dA-Za-z]+))"
    ))
    .unwrap()
});
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^https?://(?:[^/?#]+\.)?{DOMAINS}/(?:(?P<user>users)|creators)/(?P<id>[^/?#&]+)"
    ))
    .unwrap()
});
static MOBILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(https?://(?:.+?\.)?)m\.").unwrap());
static ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<div[^>]+id=["']videoClosed["'][^>]*>(.+?)</div>"#).unwrap());
static INITIALS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"window\.initials\s*=\s*(\{.+?\})\s*;\s*</script>|window\.initials\s*=\s*(\{.+?\})\s*;",
    )
    .unwrap()
});
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)[pP]").unwrap());
static HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-fA-F]{12,}$").unwrap());
static HEX_ANY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-fA-F]+$").unwrap());
static CIPHERED_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/(?P<hex>[0-9a-fA-F]{12,})(?P<rem>[/,].+)$").unwrap());
static ENTRY_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(<a[^>]+class=["'].*?\bvideo-thumb__image-container[^>]+>)"#).unwrap()
});
static NEXT_PAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+data-page=["']next[^>]+>"#).unwrap());

/// `_match_valid_url` of [`XHamster`].
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_valid_url` of [`XHamsterUser`].
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

/// `_ByteGenerator`: the site's keystream algorithms 1 to 7 (`uint32` state).
struct ByteGenerator {
    algorithm: u8,
    state: u32,
}

impl ByteGenerator {
    fn new(algorithm: u8, seed: i32) -> Option<Self> {
        (1..=7).contains(&algorithm).then_some(Self {
            algorithm,
            state: seed as u32,
        })
    }

    fn next_byte(&mut self) -> u8 {
        (self.step() & 0xFF) as u8
    }

    fn step(&mut self) -> u32 {
        let s = self.state;
        match self.algorithm {
            1 => {
                self.state = s.wrapping_mul(1664525).wrapping_add(1013904223);
                self.state
            }
            2 => {
                let mut x = s ^ (s << 13);
                x ^= x >> 17;
                x ^= x << 5;
                self.state = x;
                x
            }
            3 => {
                let mut x = s.wrapping_add(0x9e3779b9);
                self.state = x;
                x ^= x >> 16;
                x = x.wrapping_mul(0x85ebca77);
                x ^= x >> 13;
                x = x.wrapping_mul(0xc2b2ae3d);
                x ^ (x >> 16)
            }
            4 => {
                let mut x = s.wrapping_add(0x6d2b79f5);
                self.state = x;
                x = x.rotate_left(7);
                x = x.wrapping_add(0x9e3779b9);
                x ^= x >> 11;
                x.wrapping_mul(0x27d4eb2d)
            }
            5 => {
                let mut x = s ^ (s << 7);
                x ^= x >> 9;
                x ^= x << 8;
                x = x.wrapping_add(0xa5a5a5a5);
                self.state = x;
                x
            }
            6 => {
                let x = s.wrapping_mul(0x2c9277b5).wrapping_add(0xac564b05);
                self.state = x;
                let mixed = x ^ (x >> 18);
                let shift = (x >> 27) & 31;
                mixed >> shift
            }
            _ => {
                let x = s.wrapping_add(0x9e3779b9);
                self.state = x;
                let mut e = x ^ (x << 5);
                e = e.wrapping_mul(0x7feb352d);
                e ^= e >> 15;
                e.wrapping_mul(0x846ca68b)
            }
        }
    }
}

/// `_decipher_hex_string`: `None` (with a warning) for an unknown algorithm.
fn decipher_hex(hex: &str, format_id: &str) -> Option<String> {
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .filter_map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok())
        .collect();
    if bytes.len() < 5 {
        return None;
    }
    let seed = i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    let Some(mut generator) = ByteGenerator::new(bytes[0], seed) else {
        crate::screen::warning(&format!(
            "[{NAME}] Skipping format \"{format_id}\": Unknown algorithm ID \"{}\"",
            bytes[0]
        ));
        return None;
    };
    Some(
        bytes[5..]
            .iter()
            .map(|byte| char::from(byte ^ generator.next_byte()))
            .collect(),
    )
}

/// `_decipher_format_url`: a hex ciphertext or a URL with one in its path.
fn decipher_format_url(format_url: &str, format_id: &str) -> Option<String> {
    if HEX.is_match(format_url) {
        return decipher_hex(format_url, format_id);
    }
    let Ok(parsed) = Url::parse(format_url) else {
        if HEX_ANY.is_match(format_url) {
            return None;
        }
        crate::screen::warning(&format!(
            "[{NAME}] Skipping format \"{format_id}\": invalid ciphertext"
        ));
        return None;
    };
    let Some(found) = CIPHERED_PATH.captures(parsed.path()) else {
        crate::screen::warning(&format!(
            "[{NAME}] Skipping format \"{format_id}\": unsupported URL format"
        ));
        return None;
    };
    let deciphered = decipher_hex(&found["hex"], format_id)?;
    let mut result = parsed.clone();
    result.set_path(&format!("/{deciphered}{}", &found["rem"]));
    Some(result.to_string())
}

/// `url_result(url, ie=...)` as a playlist entry (its `ie_key` only).
fn url_entry(target: &str) -> Result<VideoInfo> {
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

/// `_fixup_formats`: the codec named by the URL's `.av1.`/`.h264.` segment.
fn codec_of(format_url: &str) -> Option<&'static str> {
    ["av1", "h264"]
        .into_iter()
        .find(|codec| format_url.contains(&format!(".{codec}.")))
}

/// `_extract_m3u8_formats(url, video_id, 'mp4', m3u8_id='hls')`.
fn master_formats(client: &Client, format_url: &str, video_id: &str) -> Vec<crate::VideoFormat> {
    let Ok(manifest) = Url::parse(format_url) else {
        return Vec::new();
    };
    super::extract_m3u8_master_optional(
        client,
        &manifest,
        NAME,
        video_id,
        Some("hls"),
        false,
        false,
    )
    .map(|info| info.formats)
    .unwrap_or_default()
}

/// `int_or_none(search(r'^(\d+)[pP]', quality))`.
fn height_of(quality: &str) -> Value {
    HEIGHT
        .captures(quality)
        .and_then(|found| found[1].parse::<i64>().ok())
        .map_or(Value::Null, Value::from)
}

/// `dict_get`-style lookup of a JSON object by key.
fn object<'a>(value: &'a Value, key: &str) -> Option<&'a Map<String, Value>> {
    value.get(key).and_then(Value::as_object)
}

impl Extractor for XHamster {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found
            .name("id")
            .or_else(|| found.name("id_2"))
            .map_or(String::new(), |m| m.as_str().to_owned());
        let display_id = found
            .name("display_id")
            .or_else(|| found.name("display_id_2"))
            .map_or(Value::Null, |m| m.as_str().into());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let desktop_url = Url::parse(&MOBILE.replace(url.as_str(), "$1"))?;
        let (webpage, final_url) = {
            let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
                crate::screen::warning_once(&format!("[{NAME}] {message}"));
            });
            download_webpage_handle(client, &desktop_url, NAME, &video_id)?
        };

        if let Some(error) = ERROR.captures(&webpage) {
            return Err(Error::Reported(crate::html::clean_html(&error[1])));
        }
        let age_limit = crate::html::rta_search(&webpage);

        let initials: Map<String, Value> = INITIALS
            .captures(&webpage)
            .and_then(|found| found.get(1).or_else(|| found.get(2)))
            .and_then(|text| serde_json::from_str::<Value>(text.as_str()).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        if initials.is_empty() {
            return Err(Error::UnsupportedHtml5(
                "XHamster pages without window.initials (the old layout) are not supported natively"
                    .into(),
            ));
        }
        let video = initials.get("videoModel").cloned().unwrap_or(Value::Null);
        let title = video.get("title").cloned().unwrap_or(Value::Null);

        let mut formats: Vec<crate::VideoFormat> = Vec::new();
        let mut format_urls: HashSet<String> = HashSet::new();
        let referer = final_url.as_str();
        let http_headers = || json!({ "Referer": referer });
        // `download` sizes by quality, from `video['sources']`.
        let mut format_sizes: Map<String, Value> = Map::new();
        let sources = object(&video, "sources").cloned().unwrap_or_default();
        if let Some(download) = sources.get("download").and_then(Value::as_object) {
            for (quality, format_dict) in download {
                if let Some(size) = format_dict.get("size").and_then(Value::as_f64) {
                    format_sizes.insert(quality.clone(), size.into());
                }
            }
        }
        let size_of = |quality: &str| format_sizes.get(quality).cloned().unwrap_or(Value::Null);

        for (format_id, formats_dict) in &sources {
            let Some(formats_dict) = formats_dict.as_object() else {
                continue;
            };
            for (quality, format_item) in formats_dict {
                // The download links are generated on demand; skipped.
                if format_id == "download" {
                    continue;
                }
                let Some(format_url) = format_item.as_str().and_then(super::json_ld::url_or_none)
                else {
                    continue;
                };
                if !format_urls.insert(format_url.clone()) {
                    continue;
                }
                let ext = super::direct::determine_ext(&format_url).unwrap_or_else(|| "mp4".into());
                let mut fields = vec![
                    ("format_id", format!("{format_id}-{quality}").into()),
                    ("url", format_url.as_str().into()),
                    ("ext", ext.into()),
                    ("height", height_of(quality)),
                    ("filesize", size_of(quality)),
                    ("http_headers", http_headers()),
                ];
                if let Some(codec) = codec_of(&format_url) {
                    fields.push(("vcodec", codec.into()));
                }
                formats.push(dict_format(fields, NAME, &video_id)?);
            }
        }

        if let Some(xplayer_sources) = initials
            .get("xplayerSettings")
            .and_then(|settings| object(settings, "sources"))
        {
            if let Some(hls_sources) = xplayer_sources.get("hls").and_then(Value::as_object) {
                for key in ["url", "fallback"] {
                    let Some(hls_url) = hls_sources.get(key).and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(hls_url) = decipher_format_url(hls_url, &format!("hls-{key}")) else {
                        continue;
                    };
                    if !format_urls.insert(hls_url.clone()) {
                        continue;
                    }
                    formats.extend(master_formats(client, &hls_url, &video_id));
                }
            }
            if let Some(standard_sources) =
                xplayer_sources.get("standard").and_then(Value::as_object)
            {
                for (identifier, formats_list) in standard_sources {
                    let Some(formats_list) = formats_list.as_array() else {
                        continue;
                    };
                    for standard_format in formats_list {
                        let Some(standard_format) = standard_format.as_object() else {
                            continue;
                        };
                        for key in ["url", "fallback"] {
                            let Some(standard_url) =
                                standard_format.get(key).and_then(Value::as_str)
                            else {
                                continue;
                            };
                            let quality = text_or_none(standard_format.get("quality"))
                                .or_else(|| text_or_none(standard_format.get("label")))
                                .unwrap_or_default();
                            // `join_nonempty(identifier, quality)`.
                            let format_id = [identifier.as_str(), quality.as_str()]
                                .iter()
                                .filter(|part| !part.is_empty())
                                .copied()
                                .collect::<Vec<_>>()
                                .join("-");
                            let Some(standard_url) = decipher_format_url(standard_url, &format_id)
                            else {
                                continue;
                            };
                            if !format_urls.insert(standard_url.clone()) {
                                continue;
                            }
                            let ext = super::direct::determine_ext(&standard_url)
                                .unwrap_or_else(|| "mp4".into());
                            if ext == "m3u8" {
                                formats.extend(master_formats(client, &standard_url, &video_id));
                                continue;
                            }
                            let mut fields = vec![
                                ("format_id", format_id.as_str().into()),
                                ("url", standard_url.as_str().into()),
                                ("ext", ext.as_str().into()),
                                ("height", height_of(&quality)),
                                ("filesize", size_of(&quality)),
                                ("http_headers", http_headers()),
                                ("__needs_testing", true.into()),
                            ];
                            if let Some(codec) = codec_of(&standard_url) {
                                fields.push(("vcodec", codec.into()));
                            }
                            formats.push(dict_format(fields, NAME, &video_id)?);
                        }
                    }
                }
            }
        }

        let categories = video
            .get("categories")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|category| category.get("name").and_then(Value::as_str))
                    .map(Value::from)
                    .collect::<Vec<_>>()
            });
        let uploader_url = object(&video, "author")
            .and_then(|author| author.get("pageURL"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let uploader_id = uploader_url
            .as_deref()
            .map(|url| url.rsplit('/').next().unwrap_or_default().to_owned());
        let rating = |key: &str| {
            object(&video, "rating")
                .and_then(|rating| rating.get(key))
                .map_or(Value::Null, |value| {
                    super::streamable::int_or_none(Some(value))
                })
        };
        let int = |key: &str| super::streamable::int_or_none(video.get(key));
        let fields = vec![
            ("id", video_id.as_str().into()),
            ("display_id", display_id),
            ("title", title),
            (
                "description",
                video.get("description").cloned().unwrap_or(Value::Null),
            ),
            ("timestamp", int("created")),
            (
                "uploader",
                object(&video, "author")
                    .and_then(|author| author.get("name"))
                    .and_then(Value::as_str)
                    .map_or(Value::Null, Value::from),
            ),
            (
                "uploader_url",
                uploader_url.as_deref().map_or(Value::Null, Value::from),
            ),
            (
                "uploader_id",
                uploader_id.as_deref().map_or(Value::Null, Value::from),
            ),
            (
                "thumbnail",
                video.get("thumbURL").cloned().unwrap_or(Value::Null),
            ),
            ("duration", int("duration")),
            ("view_count", int("views")),
            ("like_count", rating("likes")),
            ("dislike_count", rating("dislikes")),
            ("comment_count", int("comments")),
            ("age_limit", age_limit.map_or(Value::from(18), Value::from)),
            ("categories", categories.map_or(Value::Null, Value::Array)),
            ("formats", Value::Null),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.format_sort_fields = vec!["res".into(), "proto:m3u8".into(), "tbr".into()];
        Ok(Some(info))
    }
}

/// `str_or_none` of a JSON scalar.
fn text_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

impl Extractor for XHamsterUser {
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
        let is_user = found.name("user").is_some();
        let user_id = found["id"].to_owned();
        crate::screen::set_extractor(USER_NAME);
        crate::screen::screen(&format!(
            "[{USER_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let (prefix, suffix) = if is_user {
            ("users", "videos")
        } else {
            ("creators", "exclusive")
        };
        let mut next_page_url = format!("https://xhamster.com/{prefix}/{user_id}/{suffix}/1");
        let mut entries = Vec::new();
        loop {
            let page_target = Url::parse(&next_page_url)?;
            let (page, _) = download_webpage_handle(client, &page_target, USER_NAME, &user_id)?;
            for tag in ENTRY_TAG.captures_iter(&page) {
                let attrs = crate::html_elements::extract_attributes(&tag[1]);
                let Some(video_url) = attrs
                    .get("href")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                else {
                    continue;
                };
                if !suitable(&video_url) {
                    continue;
                }
                entries.push(url_entry(&video_url)?);
            }
            let Some(next) = NEXT_PAGE.captures(&page) else {
                break;
            };
            let attrs = crate::html_elements::extract_attributes(&next[0]);
            let Some(href) = attrs
                .get("href")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                break;
            };
            next_page_url = href;
        }

        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: user_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: USER_NAME.into(),
            extractor_key: USER_NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra: untitled_extra(),
        })))
    }
}

/// Playlist without a title: upstream's dict keeps only `id` of the title keys.
fn untitled_extra() -> Map<String, Value> {
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "playlist-dict;id".into(),
    );
    extra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://xhamster.com/videos/femaleagent-shy-beauty-takes-the-bait-1509445"
        ));
        assert!(suitable(
            "https://xhamster.com/movies/2221348/britney_spears.html"
        ));
        assert!(user_suitable(
            "https://xhamster.com/users/netvideogirls/videos"
        ));
        assert!(user_suitable("https://xhamster.com/creators/firatkaan"));
        assert!(!user_suitable("https://xhamster.com/"));
    }

    #[test]
    fn byte_generator_matches_upstream_algorithm_one() {
        let mut generator = ByteGenerator::new(1, 0).unwrap();
        // 0 * 1664525 + 1013904223 = 0x3C6EF35F, low byte 0x5F.
        assert_eq!(generator.next_byte(), 0x5F);
        assert!(ByteGenerator::new(8, 0).is_none());
    }
}
