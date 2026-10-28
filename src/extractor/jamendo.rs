//! Upstream `JamendoIE` and `JamendoAlbumIE` (`yt_dlp/extractor/jamendo.py`):
//! a track's `www.jamendo.com/api/tracks` JSON (with its artist and album
//! names) as four fixed audio formats and HEAD-checked cover thumbnails, and
//! an album's tracks as `url_transparent` entries carrying the album name.
use super::{Extractor, download_json_headers};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Jamendo;
pub struct JamendoAlbum;

const NAME: &str = "Jamendo";
const ALBUM_NAME: &str = "JamendoAlbum";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:licensing\.jamendo\.com/[^/]+|(?:www\.)?jamendo\.com)/track/(?P<id>[0-9]+)(?:/(?P<display_id>[^/?#&]+))?",
    )
    .unwrap()
});
static ALBUM_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?jamendo\.com/album/(?P<id>[0-9]+)").unwrap());

/// `JamendoIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `JamendoAlbumIE._match_id`.
pub fn album_suitable(url: &str) -> bool {
    ALBUM_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Jamendo {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `str(value)` for the JSON scalars an API ID can be.
fn py_str(value: Option<&Value>) -> Result<String> {
    match value.unwrap_or(&Value::Null) {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(if *flag { "True" } else { "False" }.into()),
        Value::Null => Err(unsupported("tracks without artist or album IDs")),
        _ => Err(unsupported("IDs that are not scalars")),
    }
}

/// `int_or_none`.
fn int_or_none(value: Option<&Value>) -> Value {
    let value = match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        Some(Value::Bool(flag)) => Some(i64::from(*flag)),
        _ => None,
    };
    value.map_or(Value::Null, Value::from)
}

/// Python's `random.random()` text: a clock-seeded xorshift64 fraction.
fn random_text() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos()),
    );
    let mut x = hasher.finish() | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    format!("{}", (x >> 11) as f64 / (1u64 << 53) as f64)
}

/// The `[0]` of an API response list.
fn first(response: Value) -> Result<Value> {
    match response {
        Value::Array(items) => items
            .into_iter()
            .next()
            .ok_or_else(|| Error::Reported("list index out of range".into())),
        _ => Err(unsupported("API responses that are not lists")),
    }
}

/// `_call_api(resource, resource_id, fatal)`: the response list's first
/// item. A failed non-fatal call (`False`, or `None` for invalid JSON) is
/// not subscriptable.
fn call_api(
    client: &Client,
    extractor: &'static str,
    resource: &str,
    resource_id: &str,
    fatal: bool,
) -> Result<Value> {
    let path = format!("/api/{resource}s");
    let rand = random_text();
    let digest = crate::sha1::digest(format!("{path}{rand}").as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let call = format!("${hex}*{rand}~");
    let mut url = Url::parse(&format!("https://www.jamendo.com{path}"))?;
    url.query_pairs_mut().append_pair("id[]", resource_id);
    match download_json_headers(
        client,
        &url,
        extractor,
        resource_id,
        &[("X-Jam-Call", call.as_str())],
    ) {
        Ok(response) => first(response),
        Err(Error::ExtractorRequest { source, .. }) if !fatal => {
            crate::screen::warning(&format!(
                "[{extractor}] {resource_id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Err(Error::Reported("'bool' object is not subscriptable".into()))
        }
        Err(Error::Reported(message)) if !fatal && message.contains(": Failed to parse JSON ") => {
            crate::screen::warning(&message);
            Err(Error::Reported(
                "'NoneType' object is not subscriptable".into(),
            ))
        }
        Err(error) => Err(error),
    }
}

/// `dict.get(key)` of an API object.
fn get<'a>(object: &'a Value, key: &str) -> Result<Option<&'a Value>> {
    match object {
        Value::Object(map) => Ok(map.get(key)),
        _ => Err(unsupported("API items that are not objects")),
    }
}

/// `int_or_none(cover_id.lstrip('size'))`.
fn cover_size(cover_id: &str) -> Value {
    let size = cover_id.trim_start_matches(['s', 'i', 'z', 'e']);
    int_or_none(Some(&Value::from(size)))
}

/// The track's cover thumbnails: each distinct URL that answers a HEAD
/// request (`errnote=False, fatal=False`), with its detected extension.
fn thumbnails(client: &Client, track: &Value, track_id: &str) -> Result<Vec<Value>> {
    let covers = match get(track, "cover")? {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Object(covers)) => covers,
        Some(cover) if !super::nuxt::truthy(cover) => return Ok(Vec::new()),
        Some(_) => return Err(unsupported("covers that are not objects")),
    };
    let mut urls: Vec<&str> = Vec::new();
    let mut thumbnails = Vec::new();
    for covers in covers.values() {
        let Value::Object(covers) = covers else {
            return Err(unsupported("cover sizes that are not objects"));
        };
        for (cover_id, cover_url) in covers {
            if !super::nuxt::truthy(cover_url) {
                continue;
            }
            let Value::String(cover_url) = cover_url else {
                return Err(unsupported("cover URLs that are not strings"));
            };
            if urls.contains(&cover_url.as_str()) {
                continue;
            }
            urls.push(cover_url);
            crate::screen::screen(&format!(
                "[{NAME}] {track_id}: Checking thumbnail extension"
            ));
            let Ok(target) = Url::parse(cover_url) else {
                return Err(unsupported("cover URLs that are not absolute"));
            };
            let response = match super::direct::head_with_headers(client, &target, &[]) {
                Ok(response) => response,
                Err(Error::Http(_) | Error::HttpRetryAfter { .. }) => continue,
                Err(error) => return Err(error),
            };
            let size = cover_size(cover_id);
            thumbnails.push(json!({
                "id": cover_id,
                "ext": super::direct::urlhandle_detect_ext(&response, Some("jpg")),
                "url": cover_url,
                "width": size,
                "height": size,
            }));
        }
    }
    Ok(thumbnails)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for Jamendo {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let track_id = found["id"].to_owned();
        let display_id = found
            .name("display_id")
            .map_or(Value::Null, |found| found.as_str().into());
        announce(NAME, url);
        let track = call_api(client, NAME, "track", &track_id, true)?;
        let title = match get(&track, "name")? {
            Some(Value::String(name)) => name.clone(),
            Some(_) => return Err(unsupported("track names that are not strings")),
            None => return Err(key_error(&track_id, "name")),
        };
        let artist_id = py_str(get(&track, "artistId")?)?;
        let artist = call_api(client, NAME, "artist", &artist_id, false)?;
        let album_id = py_str(get(&track, "albumId")?)?;
        let album = call_api(client, NAME, "album", &album_id, false)?;

        let mut formats = Vec::new();
        for (quality, (format_id, sub_domain, ext)) in [
            ("mp31", "mp3l", "mp3"),
            ("mp32", "mp3d", "mp3"),
            ("ogg1", "ogg", "ogg"),
            ("flac", "flac", "flac"),
        ]
        .into_iter()
        .enumerate()
        {
            formats.push(super::dict_format(
                vec![
                    (
                        "url",
                        format!(
                            "https://{sub_domain}.jamendo.com/?trackid={track_id}&format={format_id}&from=app-97dab294"
                        )
                        .into(),
                    ),
                    ("format_id", format_id.into()),
                    ("ext", ext.into()),
                    ("quality", quality.into()),
                ],
                NAME,
                &track_id,
            )?);
        }

        let thumbnails = thumbnails(client, &track, &track_id)?;

        let mut tags = Vec::new();
        match get(&track, "tags")? {
            Some(Value::Array(items)) => {
                for tag in items {
                    match get(tag, "name")? {
                        Some(name) if super::nuxt::truthy(name) => tags.push(name.clone()),
                        _ => {}
                    }
                }
            }
            Some(value) if super::nuxt::truthy(value) => {
                return Err(unsupported("tag lists that are not lists"));
            }
            _ => {}
        }

        let stats = match get(&track, "stats")? {
            Some(stats @ Value::Object(_)) => stats.clone(),
            Some(value) if super::nuxt::truthy(value) => {
                return Err(unsupported("stats that are not objects"));
            }
            _ => json!({}),
        };
        let license = match get(&track, "licenseCC")? {
            Some(Value::Array(parts)) if !parts.is_empty() => {
                let parts = parts
                    .iter()
                    .map(|part| part.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| unsupported("licenses that are not strings"))?;
                Value::from(parts.join("-"))
            }
            Some(value) if super::nuxt::truthy(value) => {
                return Err(unsupported("licenses that are not lists"));
            }
            _ => Value::Null,
        };
        let field = |object: &Value, key: &str| -> Result<Value> {
            Ok(get(object, key)?.cloned().unwrap_or(Value::Null))
        };

        Ok(Some(super::dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", track_id.clone().into()),
                ("display_id", display_id),
                ("thumbnails", Value::Array(thumbnails)),
                ("title", title.clone().into()),
                ("description", field(&track, "description")?),
                ("duration", int_or_none(get(&track, "duration")?)),
                ("artist", field(&artist, "name")?),
                ("track", title.into()),
                ("album", field(&album, "name")?),
                ("formats", Value::Null),
                ("license", license),
                ("timestamp", int_or_none(get(&track, "dateCreated")?)),
                ("view_count", int_or_none(stats.get("listenedAll"))),
                ("like_count", int_or_none(stats.get("favorited"))),
                ("average_rating", int_or_none(stats.get("averageNote"))),
                ("tags", Value::Array(tags)),
            ],
            formats,
        )))
    }
}

/// An album track's `url_transparent` entry of `JamendoIE`.
fn track_entry(track_id: &str, album_name: &Value) -> Result<VideoInfo> {
    let target = format!("https://www.jamendo.com/track/{track_id}");
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = track_id.to_owned();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {"album": album_name},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": {
                "_type": "url_transparent",
                "url": target,
                "ie_key": NAME,
                "id": track_id,
                "album": album_name,
            },
        }),
    );
    Ok(info)
}

impl Extractor for JamendoAlbum {
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
        let album_id = found["id"].to_owned();
        announce(ALBUM_NAME, url);
        let album = call_api(client, ALBUM_NAME, "album", &album_id, true)?;
        let album_name = get(&album, "name")?.cloned().unwrap_or(Value::Null);

        let mut entries = Vec::new();
        match get(&album, "tracks")? {
            Some(Value::Array(tracks)) => {
                for track in tracks {
                    let track_id = match get(track, "id")? {
                        Some(id) if super::nuxt::truthy(id) => id,
                        _ => continue,
                    };
                    let track_id = match track_id {
                        Value::String(id) => id.clone(),
                        Value::Number(id) => id.to_string(),
                        _ => return Err(unsupported("track IDs that are not scalars")),
                    };
                    entries.push(track_entry(&track_id, &album_name)?);
                }
            }
            Some(value) if super::nuxt::truthy(value) => {
                return Err(unsupported("track lists that are not lists"));
            }
            _ => {}
        }

        // `clean_html(try_get(album, lambda x: x['description']['en'], str))`.
        let description = get(&album, "description")?
            .and_then(|description| description.get("en"))
            .and_then(Value::as_str)
            .map(crate::html::clean_html);
        let title = match &album_name {
            Value::String(name) if !name.is_empty() => Some(name.clone()),
            Value::Null | Value::String(_) => None,
            _ => return Err(unsupported("album names that are not strings")),
        };
        let mut extra = Map::new();
        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        if let Some(description) = description {
            extra.insert("description".into(), description.into());
            keys.push("description");
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: album_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: ALBUM_NAME.into(),
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
        assert!(suitable(
            "https://www.jamendo.com/track/196219/stories-from-emona-i"
        ));
        assert!(suitable(
            "https://licensing.jamendo.com/en/track/1496667/energetic-rock"
        ));
        assert!(suitable("https://jamendo.com/track/1"));
        assert!(!suitable("https://www.jamendo.com/album/121486"));
        assert!(album_suitable(
            "https://www.jamendo.com/album/121486/duck-on-cover"
        ));
        assert!(!album_suitable("https://licensing.jamendo.com/en/album/1"));
    }

    #[test]
    fn cover_ids_strip_size_characters() {
        assert_eq!(cover_size("size300"), json!(300));
        assert_eq!(cover_size("sizeX"), Value::Null);
        assert_eq!(cover_size("ezis45"), json!(45));
    }
}
