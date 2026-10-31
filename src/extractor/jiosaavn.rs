//! Upstream `JioSaavnSongIE` and `JioSaavnAlbumIE` (`yt_dlp/extractor/jiosaavn.py`)
//! with the `JioSaavnBaseIE` helpers they use: the `webapi.get` JSON, the song
//! metadata (`_extract_song`), its per-bitrate `generateAuthToken` URLs and the
//! album's `url_transparent` song entries. The show, playlist, show-playlist
//! and artist extractors are not ported.
use super::{Extractor, dict_format, dict_info, direct};
use crate::info::PlaylistInfo;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct JioSaavnSong;
pub struct JioSaavnAlbum;

const SONG_NAME: &str = "jiosaavn:song";
const SONG_KEY: &str = "JioSaavnSong";
const ALBUM_NAME: &str = "jiosaavn:album";
const ALBUM_KEY: &str = "JioSaavnAlbum";
const API_URL: &str = "https://www.jiosaavn.com/api.php";
const VALID_BITRATES: [&str; 5] = ["16", "32", "64", "128", "320"];
const URL_BASE: &str = r"^https?://(?:www\.)?(?:jio)?saavn\.com";

static SONG_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"{URL_BASE}(?:/song/[^/?#]+/|/s/song/(?:[^/?#]+/){{3}})(?P<id>[^/?#]+)"
    ))
    .unwrap()
});
static ALBUM_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(r"{URL_BASE}/album/[^/?#]+/(?P<id>[^/?#]+)")).unwrap()
});
static ART_SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-\d+x\d+\.").unwrap());

/// `_match_valid_url` for the song extractor.
pub fn song_suitable(url: &str) -> bool {
    SONG_URL.is_match(url)
}

/// `_match_valid_url` for the album extractor.
pub fn album_suitable(url: &str) -> bool {
    ALBUM_URL.is_match(url)
}

/// `JioSaavnBaseIE.requested_bitrates`: the `bitrate` extractor argument.
fn requested_bitrates() -> Result<Vec<String>> {
    let bitrates = crate::manifest_args::configuration_arg("JioSaavn", "bitrate", false)
        .unwrap_or_else(|| vec!["128".into(), "320".into()]);
    let invalid: Vec<&String> = bitrates
        .iter()
        .filter(|bitrate| !VALID_BITRATES.contains(&bitrate.as_str()))
        .collect();
    if !invalid.is_empty() {
        let names: Vec<&str> = invalid.iter().map(|bitrate| bitrate.as_str()).collect();
        return Err(Error::Reported(format!(
            "Invalid bitrate(s): {}. Valid bitrates are: {}",
            names.join(", "),
            VALID_BITRATES.join(", ")
        )));
    }
    Ok(bitrates)
}

/// `_download_json` with the `what` errnote of an API request.
fn download_api(
    client: &Client,
    url: &Url,
    (extractor, id): (&'static str, &str),
    note: &str,
    what: &'static str,
) -> Result<Value> {
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let response = direct::request(client, url, None).map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor,
            id: id.to_owned(),
            what,
            source: Box::new(error),
        },
        other => other,
    })?;
    let text = response.text().map_err(Error::Http)?;
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_call_api(type_, token, note)`: the `webapi.get` JSON.
fn call_api(
    client: &Client,
    extractor: &'static str,
    kind: &str,
    token: &str,
    note: &str,
) -> Result<Value> {
    let url = super::update_url_query(
        &Url::parse(API_URL)?,
        &[
            ("__call", "webapi.get"),
            ("_format", "json"),
            ("_marker", "0"),
            ("ctx", "web6dot0"),
            ("token", token),
            ("type", kind),
        ],
    );
    download_api(
        client,
        &url,
        (extractor, token),
        &format!("Downloading {note} JSON"),
        "API JSON",
    )
}

/// `traverse_obj(..., (keys, {str}, any))`-style first truthy candidate.
fn first_truthy(candidates: Vec<Option<Value>>) -> Option<Value> {
    candidates.into_iter().flatten().find(super::nuxt::truthy)
}

/// The value at `key` of `node`, and of its `more_info` (`(None, 'more_info')`).
fn with_more_info(song: &Value, key: &str) -> Vec<Option<Value>> {
    vec![
        song.get(key).cloned(),
        song.get("more_info")
            .and_then(|info| info.get(key))
            .cloned(),
    ]
}

/// `_extract_song(song_data, url)`: the song's info fields in upstream key
/// order (`None` values left out, except `artists`).
fn song_fields(song: &Value, url: Option<&str>) -> Vec<(&'static str, Value)> {
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let id = song.get("id").and_then(text);
    let title = first_truthy(
        ["song", "title"]
            .iter()
            .map(|key| {
                song.get(*key)
                    .and_then(Value::as_str)
                    .map(|text| Value::from(crate::html::clean_html(text)))
            })
            .collect(),
    );
    let album = first_truthy(
        with_more_info(song, "album")
            .into_iter()
            .map(|value| {
                value.and_then(|value| value.as_str().map(crate::html::clean_html).map(Value::from))
            })
            .collect(),
    );
    let duration = first_truthy(
        with_more_info(song, "duration")
            .into_iter()
            .map(|value| Some(super::streamable::int_or_none(value.as_ref())))
            .collect(),
    );
    let label = |key: &str| {
        first_truthy(
            with_more_info(song, key)
                .into_iter()
                .map(|value| value.and_then(|value| value.as_str().map(Value::from)))
                .collect(),
        )
    };
    let channel_url = first_truthy(
        with_more_info(song, "label_url")
            .into_iter()
            .map(|value| {
                value
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .and_then(|path| {
                        Url::parse("https://www.jiosaavn.com/")
                            .ok()?
                            .join(&path)
                            .ok()
                    })
                    .map(|url| Value::from(url.as_str()))
            })
            .collect(),
    );
    let release_date = first_truthy(
        with_more_info(song, "release_date")
            .into_iter()
            .map(|value| {
                value
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .and_then(|date| crate::dates::unified_strdate(&date, true))
                    .map(Value::from)
            })
            .collect(),
    );
    let release_year = super::streamable::int_or_none(song.get("year"));
    let thumbnail = song
        .get("image")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map(|url| Value::from(ART_SIZE.replace(&url, "-500x500.").into_owned()));
    let view_count = super::streamable::int_or_none(song.get("play_count"));
    let language = song
        .get("language")
        .and_then(Value::as_str)
        .map(|code| {
            let code = code.to_lowercase();
            crate::embed_subtitles::languages::short_to_long(&code)
                .unwrap_or("und")
                .to_owned()
        })
        .map_or(Value::Null, Value::from);
    let webpage_url = song
        .get("perma_url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none);

    // `artists` from `more_info.artistMap.primary_artists`, then the song's
    // `primary_artists` and `featured_artists` strings.
    let mut artists: Vec<String> = Vec::new();
    if let Some(primary) = song
        .get("more_info")
        .and_then(|info| info.get("artistMap"))
        .and_then(|map| map.get("primary_artists"))
        .and_then(Value::as_array)
    {
        artists.extend(
            primary
                .iter()
                .filter_map(|artist| artist.get("name").and_then(Value::as_str))
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        );
    }
    if let Some(primary) = song
        .get("primary_artists")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        artists.extend(primary.split(", ").map(str::to_owned));
    }
    if let Some(featured) = song
        .get("featured_artists")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        artists.extend(featured.split(", ").map(str::to_owned));
    }
    let mut seen: Vec<String> = Vec::new();
    for artist in artists {
        if !seen.contains(&artist) {
            seen.push(artist);
        }
    }
    let artists = if seen.is_empty() {
        Value::Null
    } else {
        Value::Array(seen.into_iter().map(Value::from).collect())
    };

    let mut fields: Vec<(&'static str, Value)> = Vec::new();
    // `None` values are left out of the info dict (`traverse_obj`).
    let mut push = |key: &'static str, value: Option<Value>| {
        if let Some(value) = value.filter(|value| !value.is_null()) {
            fields.push((key, value));
        }
    };
    push("id", id.map(Value::from));
    push("title", title);
    push("album", album);
    push("duration", duration);
    push("channel", label("label"));
    push("channel_id", label("label_id"));
    push("channel_url", channel_url);
    push("release_date", release_date);
    push("release_year", Some(release_year));
    push("thumbnail", thumbnail);
    push("view_count", Some(view_count));
    push("language", Some(language));
    push("webpage_url", webpage_url.clone().map(Value::from));
    fields.push(("artists", artists));
    let page_url = webpage_url.or_else(|| url.map(str::to_owned));
    if let Some(page_url) = page_url {
        let display_id = Url::parse(&page_url)
            .ok()
            .and_then(|parsed| {
                parsed
                    .path()
                    .trim_matches('/')
                    .rsplit('/')
                    .next()
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        fields.push(("display_id", Value::from(display_id.as_str())));
        fields.push((
            "_old_archive_ids",
            json!([format!("jiosaavnsong {display_id}")]),
        ));
    }
    fields
}

/// `_extract_formats(item_data)`: one `m4a` format per requested bitrate
/// whose `generateAuthToken` call returns an `auth_url`.
fn extract_formats(
    client: &Client,
    item: &Value,
    extractor: &'static str,
) -> Result<Vec<crate::VideoFormat>> {
    let item_id = item.get("id").and_then(Value::as_str).unwrap_or_default();
    let encrypted = item
        .get("encrypted_media_url")
        .and_then(Value::as_str)
        .ok_or_else(|| super::clyp::key_error(item_id, "encrypted_media_url"))?;
    let mut formats = Vec::new();
    for bitrate in requested_bitrates()? {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("__call", "song.generateAuthToken")
            .append_pair("_format", "json")
            .append_pair("bitrate", &bitrate)
            .append_pair("url", encrypted)
            .finish();
        let note = format!("Downloading format info for {bitrate}");
        let media = super::post_json_errnote_optional(
            client,
            &Url::parse(API_URL)?,
            (extractor, item_id),
            (note.as_str(), "Unable to download JSON metadata"),
            &[("Content-Type", "application/x-www-form-urlencoded")],
            body,
        )?;
        let auth_url = media
            .as_ref()
            .and_then(|media| media.get("auth_url"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let Some(auth_url) = auth_url else {
            crate::screen::warning(&format!(
                "[{extractor}] Unable to extract format info for {bitrate}"
            ));
            continue;
        };
        let media = media.unwrap_or(Value::Null);
        let ext = match media.get("type").and_then(Value::as_str) {
            Some("mp4") => Value::from("m4a"),
            Some(other) => Value::from(other),
            None => Value::Null,
        };
        let abr: i64 = bitrate.parse().unwrap_or_default();
        let mut format = dict_format(
            vec![
                ("url", Value::from(auth_url)),
                ("ext", ext),
                ("format_id", Value::from(bitrate.as_str())),
                ("abr", Value::from(abr)),
                ("vcodec", Value::from("none")),
            ],
            extractor,
            item_id,
        )?;
        // Upstream's `int(bitrate)` stays an int through processing.
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    Ok(formats)
}

/// `_extract_jiosaavn_result(url, 'song', 'songs', _extract_song)`.
fn extract_song(client: &Client, url: &Url) -> Result<VideoInfo> {
    let (bare, smuggled) = super::brightcove::unsmuggle(url.as_str());
    let bare = Url::parse(&bare)?;
    let transparent = smuggled.get("id").is_some_and(super::nuxt::truthy)
        && smuggled
            .get("encrypted_media_url")
            .is_some_and(super::nuxt::truthy);
    let (fields, item) = if transparent {
        // A `url_transparent` album entry: its metadata and media URL are known.
        let item = Value::Object(smuggled);
        (
            vec![("id", item["id"].clone()), ("formats", Value::Null)],
            item,
        )
    } else {
        let Some(found) = SONG_URL.captures(bare.as_str()) else {
            return Err(Error::Reported("Unsupported URL".into()));
        };
        let token = found["id"].to_owned();
        let response = call_api(client, SONG_NAME, "song", &token, "API")?;
        let song = response
            .get("songs")
            .ok_or_else(|| super::clyp::key_error(&token, "songs"))?
            .get(0)
            .cloned()
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let mut fields = song_fields(&song, Some(bare.as_str()));
        fields.push(("formats", Value::Null));
        (fields, song)
    };
    let formats = extract_formats(client, &item, SONG_NAME)?;
    Ok(dict_info(&bare, (SONG_NAME, SONG_KEY), fields, formats))
}

/// `url_result(url, JioSaavnSongIE, url_transparent=True, **info)`: the
/// entry's fields in the upstream key order, then its IE key, type and URL.
fn transparent_entry(target: &str, fields: Vec<(&'static str, Value)>) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    for (key, value) in fields {
        entry.insert(key.into(), value);
    }
    entry.insert("ie_key".into(), SONG_KEY.into());
    entry.insert("_type".into(), "url_transparent".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": SONG_KEY,
            "extractor": SONG_NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `_yield_items(album_data, 'songs')`: a `url_transparent` entry per song
/// with a permalink and an ID (its media URL smuggled in the fragment).
fn album_entries(album: &Value) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    for song in album
        .get("songs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let has_id = song.get("id").is_some_and(super::nuxt::truthy);
        let has_permalink = song.get("perma_url").is_some_and(super::nuxt::truthy);
        if !(has_id && has_permalink) {
            continue;
        }
        let fields = song_fields(song, None);
        let Some(webpage) = fields
            .iter()
            .find(|(key, _)| *key == "webpage_url")
            .and_then(|(_, value)| value.as_str())
        else {
            continue;
        };
        let mut smuggled = Map::new();
        if let Some(id) = song.get("id").and_then(Value::as_str) {
            smuggled.insert("id".into(), id.into());
        }
        let encrypted = first_truthy(with_more_info(song, "encrypted_media_url"));
        if let Some(encrypted) = encrypted.and_then(|value| value.as_str().map(str::to_owned)) {
            smuggled.insert("encrypted_media_url".into(), encrypted.into());
        }
        let target = super::brightcove::smuggle(webpage, &Value::Object(smuggled));
        entries.push(transparent_entry(&target, fields)?);
    }
    Ok(entries)
}

impl Extractor for JioSaavnSong {
    fn key(&self) -> &'static str {
        SONG_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !SONG_URL.is_match(url.as_str()) {
            return Ok(None);
        }
        crate::screen::set_extractor(SONG_NAME);
        crate::screen::screen(&format!(
            "[{SONG_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        extract_song(client, url).map(Some)
    }
}

impl Extractor for JioSaavnAlbum {
    fn key(&self) -> &'static str {
        ALBUM_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ALBUM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(ALBUM_NAME);
        crate::screen::screen(&format!(
            "[{ALBUM_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let album = call_api(client, ALBUM_NAME, "album", &display_id, "API")?;
        let title = album
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let entries = album_entries(&album)?;
        let mut extra = Map::new();
        let mut keys = vec!["id"];
        if !title.is_empty() {
            keys.push("title");
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id,
            title,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(song_suitable(
            "https://www.jiosaavn.com/song/leja-re/OQsEfQFVUXk"
        ));
        assert!(song_suitable(
            "https://www.saavn.com/s/song/hindi/Saathiya/O-Humdum-Suniyo-Re/KAMiazoCblU"
        ));
        assert!(!song_suitable(
            "https://www.jiosaavn.com/album/96/buIOjYZDrNA_"
        ));
        assert!(album_suitable(
            "https://www.jiosaavn.com/album/96/buIOjYZDrNA_"
        ));
        assert!(!album_suitable(
            "https://www.jiosaavn.com/song/leja-re/OQsEfQFVUXk"
        ));
    }

    #[test]
    fn artists_are_split_and_deduplicated() {
        let song = json!({
            "id": "1", "song": "Title", "primary_artists": "A, B",
            "featured_artists": "B, C", "perma_url": "https://www.jiosaavn.com/song/t/XYZ"
        });
        let fields = song_fields(&song, None);
        let artists = fields.iter().find(|(key, _)| *key == "artists").unwrap();
        assert_eq!(artists.1, json!(["A", "B", "C"]));
    }
}
