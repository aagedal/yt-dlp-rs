//! Upstream `JioSaavnSongIE`, `JioSaavnAlbumIE`, `JioSaavnPlaylistIE`,
//! `JioSaavnArtistIE`, `JioSaavnShowIE` and `JioSaavnShowPlaylistIE`
//! (`yt_dlp/extractor/jiosaavn.py`) with the `JioSaavnBaseIE` helpers they use:
//! the `webapi.get` JSON, the song and episode metadata (`_extract_song`,
//! `_extract_episode`), their per-bitrate `generateAuthToken` URLs and the
//! `url_transparent` song and episode entries of the list extractors.
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
pub struct JioSaavnPlaylist;
pub struct JioSaavnArtist;
pub struct JioSaavnShow;
pub struct JioSaavnShowPlaylist;

const SONG_NAME: &str = "jiosaavn:song";
const SONG_KEY: &str = "JioSaavnSong";
const ALBUM_NAME: &str = "jiosaavn:album";
const ALBUM_KEY: &str = "JioSaavnAlbum";
const PLAYLIST_NAME: &str = "jiosaavn:playlist";
const PLAYLIST_KEY: &str = "JioSaavnPlaylist";
const ARTIST_NAME: &str = "jiosaavn:artist";
const ARTIST_KEY: &str = "JioSaavnArtist";
const SHOW_NAME: &str = "jiosaavn:show";
const SHOW_KEY: &str = "JioSaavnShow";
const SHOW_PLAYLIST_NAME: &str = "jiosaavn:show:playlist";
const SHOW_PLAYLIST_KEY: &str = "JioSaavnShowPlaylist";
const API_URL: &str = "https://www.jiosaavn.com/api.php";
const VALID_BITRATES: [&str; 5] = ["16", "32", "64", "128", "320"];
/// `_PAGE_SIZE` of the playlist and artist extractors.
const PAGE_SIZE: usize = 50;
/// `_PAGE_SIZE` of the show-playlist extractor.
const SHOW_PAGE_SIZE: usize = 10;
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
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"{URL_BASE}/(?:s/playlist/(?:[^/?#]+/){{2}}|featured/[^/?#]+/)(?P<id>[^/?#]+)"
    ))
    .unwrap()
});
static ARTIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(r"{URL_BASE}/artist/[^/?#]+/(?P<id>[^/?#]+)")).unwrap()
});
static SHOW_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"{URL_BASE}/shows/[^/?#]+/(?P<id>[^/?#]{{11,}})/?(?:$|[?#])"
    ))
    .unwrap()
});
static SHOW_PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"{URL_BASE}/shows/(?P<show>[^#/?]+)/(?P<season>\d+)/[^/?#]+"
    ))
    .unwrap()
});
static INITIAL_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__INITIAL_DATA__\s*=").unwrap());
static ART_SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-\d+x\d+\.").unwrap());

/// `_match_valid_url` for the song extractor.
pub fn song_suitable(url: &str) -> bool {
    SONG_URL.is_match(url)
}

/// `_match_valid_url` for the album extractor.
pub fn album_suitable(url: &str) -> bool {
    ALBUM_URL.is_match(url)
}

/// `_match_valid_url` for the playlist extractor.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `_match_valid_url` for the artist extractor.
pub fn artist_suitable(url: &str) -> bool {
    ARTIST_URL.is_match(url)
}

/// `_match_valid_url` for the show (episode) extractor.
pub fn show_suitable(url: &str) -> bool {
    SHOW_URL.is_match(url)
}

/// `_match_valid_url` for the show-playlist extractor.
pub fn show_playlist_suitable(url: &str) -> bool {
    SHOW_PLAYLIST_URL.is_match(url)
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
    call_api_params(client, extractor, kind, token, note, &[])
}

/// `_call_api(type_, token, note, params)`: the `webapi.get` query, its base
/// keys updated in place by `params` (as a Python dict update does).
fn call_api_params(
    client: &Client,
    extractor: &'static str,
    kind: &str,
    token: &str,
    note: &str,
    params: &[(&str, &str)],
) -> Result<Value> {
    let mut query: Vec<(&str, &str)> = vec![
        ("__call", "webapi.get"),
        ("_format", "json"),
        ("_marker", "0"),
        ("ctx", "web6dot0"),
        ("token", token),
        ("type", kind),
    ];
    for (key, value) in params {
        match query.iter_mut().find(|(name, _)| name == key) {
            Some(slot) => slot.1 = value,
            None => query.push((key, value)),
        }
    }
    let url = super::update_url_query(&Url::parse(API_URL)?, &query);
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
    // `setdefault('encrypted_media_url', more_info.encrypted_media_url)`: the
    // episodes keep it in `more_info`.
    let encrypted = item
        .get("encrypted_media_url")
        .or_else(|| item.get("more_info")?.get("encrypted_media_url"))
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

/// `_extract_song` / `_extract_episode`: an item's info fields.
type ItemFields = Vec<(&'static str, Value)>;

/// One `JioSaavnBaseIE` media kind: its endpoint, the response key of its
/// item list, its IE names and its `_extract_song`/`_extract_episode` parse.
#[derive(Clone, Copy)]
struct Media {
    endpoint: &'static str,
    list: &'static str,
    name: &'static str,
    key: &'static str,
    parse: fn(&Value, Option<&str>) -> ItemFields,
}

const SONG: Media = Media {
    endpoint: "song",
    list: "songs",
    name: SONG_NAME,
    key: SONG_KEY,
    parse: song_fields,
};
const EPISODE: Media = Media {
    endpoint: "episode",
    list: "episodes",
    name: SHOW_NAME,
    key: SHOW_KEY,
    parse: episode_fields,
};

/// A list value (`[]` when absent).
fn list_of<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// `_extract_episode(episode_data, url)`: the song fields without the archive
/// IDs, plus the show's `more_info` (description, release time, series,
/// season, episode and cast).
fn episode_fields(episode: &Value, url: Option<&str>) -> Vec<(&'static str, Value)> {
    let mut fields: Vec<(&'static str, Value)> = song_fields(episode, url)
        .into_iter()
        .filter(|(key, _)| *key != "_old_archive_ids")
        .collect();
    let info = episode.get("more_info");
    let info_text = |key: &str| {
        info.and_then(|info| info.get(key))
            .and_then(Value::as_str)
            .map(Value::from)
    };
    let info_int = |key: &str| {
        Some(super::streamable::int_or_none(
            info.and_then(|info| info.get(key)),
        ))
    };
    let cast = episode
        .get("starring")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(|text| {
            text.split(", ")
                .map(Value::from)
                .collect::<Vec<Value>>()
                .into()
        });
    let extra = [
        ("description", info_text("description")),
        (
            "timestamp",
            info.and_then(|info| info.get("release_time"))
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_timestamp(date, true))
                .map(Value::from),
        ),
        ("series", info_text("show_title")),
        ("series_id", info_text("show_id")),
        ("season", info_text("season_title")),
        ("season_number", info_int("season_no")),
        ("season_id", info_text("season_id")),
        ("episode_number", info_int("episode_number")),
        ("cast", cast),
    ];
    for (key, value) in extra {
        let Some(value) = value.filter(|value| !value.is_null()) else {
            continue;
        };
        match fields.iter_mut().find(|(name, _)| *name == key) {
            Some(slot) => slot.1 = value,
            None => fields.push((key, value)),
        }
    }
    fields
}

/// `_extract_jiosaavn_result(url, endpoint, response_key, parse_func)`: the
/// page's metadata and formats, or the smuggled id and media URL of a
/// `url_transparent` list entry.
fn extract_media(client: &Client, url: &Url, pattern: &Regex, media: &Media) -> Result<VideoInfo> {
    let (bare, smuggled) = super::brightcove::unsmuggle(url.as_str());
    let bare = Url::parse(&bare)?;
    let transparent = smuggled.get("id").is_some_and(super::nuxt::truthy)
        && smuggled
            .get("encrypted_media_url")
            .is_some_and(super::nuxt::truthy);
    let (fields, item) = if transparent {
        // A `url_transparent` entry: its metadata and media URL are known.
        let item = Value::Object(smuggled);
        (
            vec![("id", item["id"].clone()), ("formats", Value::Null)],
            item,
        )
    } else {
        let Some(found) = pattern.captures(bare.as_str()) else {
            return Err(Error::Reported("Unsupported URL".into()));
        };
        let token = found["id"].to_owned();
        let response = call_api(client, media.name, media.endpoint, &token, "API")?;
        let item = response
            .get(media.list)
            .ok_or_else(|| super::clyp::key_error(&token, media.list))?
            .get(0)
            .cloned()
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let mut fields = (media.parse)(&item, Some(bare.as_str()));
        fields.push(("formats", Value::Null));
        (fields, item)
    };
    let formats = extract_formats(client, &item, media.name)?;
    Ok(dict_info(&bare, (media.name, media.key), fields, formats))
}

/// `url_result(url, ie, url_transparent=True, **info)`: the entry's fields in
/// the upstream key order, then its IE key, type and URL.
fn transparent_entry(
    target: &str,
    fields: Vec<(&'static str, Value)>,
    media: &Media,
) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    for (key, value) in fields {
        entry.insert(key.into(), value);
    }
    entry.insert("ie_key".into(), media.key.into());
    entry.insert("_type".into(), "url_transparent".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": media.key,
            "extractor": media.name,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `_yield_items(playlist_data, keys)`: a `url_transparent` entry per item
/// with a permalink and an ID (its media URL smuggled in the fragment).
fn yield_items(items: &[Value], media: &Media) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    for item in items {
        let has_id = item.get("id").is_some_and(super::nuxt::truthy);
        let has_permalink = item.get("perma_url").is_some_and(super::nuxt::truthy);
        if !(has_id && has_permalink) {
            continue;
        }
        let fields = (media.parse)(item, None);
        let Some(webpage) = fields
            .iter()
            .find(|(key, _)| *key == "webpage_url")
            .and_then(|(_, value)| value.as_str())
        else {
            continue;
        };
        let mut smuggled = Map::new();
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            smuggled.insert("id".into(), id.into());
        }
        let encrypted = first_truthy(with_more_info(item, "encrypted_media_url"));
        if let Some(encrypted) = encrypted.and_then(|value| value.as_str().map(str::to_owned)) {
            smuggled.insert("encrypted_media_url".into(), encrypted.into());
        }
        let target = super::brightcove::smuggle(webpage, &Value::Object(smuggled));
        entries.push(transparent_entry(&target, fields, media)?);
    }
    Ok(entries)
}

/// The playlist-shaped result of `entries` with an optional title.
fn list_result(
    id: String,
    title: Option<String>,
    url: &Url,
    (name, key): (&'static str, &'static str),
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut keys = vec!["id"];
    if title.is_some() {
        keys.push("title");
    }
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

/// `_call_api('playlist', ...)` page `page` (1-based) of a playlist.
fn playlist_page(client: &Client, token: &str, page: usize) -> Result<Value> {
    let page = page.to_string();
    let size = PAGE_SIZE.to_string();
    call_api_params(
        client,
        PLAYLIST_NAME,
        "playlist",
        token,
        &format!("playlist page {page}"),
        &[("p", &page), ("n", &size)],
    )
}

/// `_call_api('artist', ...)` page `page` (0-based) of an artist.
fn artist_page(client: &Client, token: &str, page: usize) -> Result<Value> {
    let page_text = page.to_string();
    let size = PAGE_SIZE.to_string();
    call_api_params(
        client,
        ARTIST_NAME,
        "artist",
        token,
        &format!("artist page {}", page + 1),
        &[
            ("p", &page_text),
            ("n_song", &size),
            ("n_album", &size),
            ("sub_type", ""),
            ("includeMetaTags", ""),
            ("api_version", "4"),
            ("category", "alphabetical"),
            ("sort_order", "asc"),
        ],
    )
}

/// `_call_api('show', ...)` page `page` (1-based) of a season's episodes.
fn show_page(client: &Client, show_id: &str, season_id: &str, page: usize) -> Result<Value> {
    let page = page.to_string();
    call_api_params(
        client,
        SHOW_PLAYLIST_NAME,
        "show",
        show_id,
        &format!("show page {page}"),
        &[
            ("p", &page),
            ("__call", "show.getAllEpisodes"),
            ("show_id", show_id),
            ("season_number", season_id),
            ("api_version", "4"),
            ("sort_order", "desc"),
        ],
    )
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for JioSaavnSong {
    fn key(&self) -> &'static str {
        SONG_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !SONG_URL.is_match(url.as_str()) {
            return Ok(None);
        }
        announce(SONG_NAME, url);
        extract_media(client, url, &SONG_URL, &SONG).map(Some)
    }
}

impl Extractor for JioSaavnShow {
    fn key(&self) -> &'static str {
        SHOW_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        if !SHOW_URL.is_match(url.as_str()) {
            return Ok(None);
        }
        announce(SHOW_NAME, url);
        extract_media(client, url, &SHOW_URL, &EPISODE).map(Some)
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
        announce(ALBUM_NAME, url);
        let album = call_api(client, ALBUM_NAME, "album", &display_id, "API")?;
        let title = album
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let entries = yield_items(list_of(&album, "songs"), &SONG)?;
        Ok(Some(list_result(
            display_id,
            title,
            url,
            (ALBUM_NAME, ALBUM_KEY),
            entries,
        )))
    }
}

impl Extractor for JioSaavnPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        announce(PLAYLIST_NAME, url);
        let first = playlist_page(client, &display_id, 1)?;
        let list_count = first
            .get("list_count")
            .ok_or_else(|| super::clyp::key_error(&display_id, "list_count"))?;
        let count = match list_count {
            Value::String(text) => text.trim().parse::<i64>().unwrap_or_default(),
            other => other.as_i64().unwrap_or_default(),
        };
        let total_pages = usize::try_from((count.max(0) + PAGE_SIZE as i64 - 1) / PAGE_SIZE as i64)
            .unwrap_or_default();
        let mut entries = Vec::new();
        for page in 0..total_pages {
            let data = if page == 0 {
                first.clone()
            } else {
                playlist_page(client, &display_id, page + 1)?
            };
            entries.extend(yield_items(list_of(&data, "songs"), &SONG)?);
        }
        let title = first
            .get("listname")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(Some(list_result(
            display_id,
            title,
            url,
            (PLAYLIST_NAME, PLAYLIST_KEY),
            entries,
        )))
    }
}

impl Extractor for JioSaavnArtist {
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
        announce(ARTIST_NAME, url);
        let first = artist_page(client, &artist_id, 0)?;
        let title = first.get("name").and_then(Value::as_str).map(str::to_owned);
        // `_entries`: the top songs of each page until a page without any.
        let mut entries = Vec::new();
        for page in 0.. {
            let data = if page == 0 {
                first.clone()
            } else {
                artist_page(client, &artist_id, page)?
            };
            let songs: Vec<Value> = list_of(&data, "topSongs")
                .iter()
                .filter(|song| song.is_object())
                .cloned()
                .collect();
            if songs.is_empty() {
                break;
            }
            entries.extend(yield_items(&songs, &SONG)?);
        }
        Ok(Some(list_result(
            artist_id,
            title,
            url,
            (ARTIST_NAME, ARTIST_KEY),
            entries,
        )))
    }
}

impl Extractor for JioSaavnShowPlaylist {
    fn key(&self) -> &'static str {
        SHOW_PLAYLIST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SHOW_PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = format!("{}-{}", &found["show"], &found["season"]);
        let season_id = found["season"].to_owned();
        announce(SHOW_PLAYLIST_NAME, url);
        let webpage = super::download_webpage(client, url, SHOW_PLAYLIST_NAME, &playlist_id)?;
        // `_search_json(r'window\.__INITIAL_DATA__\s*=', ..., transform_source=js_to_json)`.
        let initial = match super::search_json(&INITIAL_DATA, &webpage, true) {
            Some(Some(initial)) => initial,
            _ => {
                return Err(Error::Reported(format!(
                    "[{SHOW_PLAYLIST_NAME}] {playlist_id}: Unable to extract initial data; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let show_info = initial
            .get("showView")
            .ok_or_else(|| super::clyp::key_error(&playlist_id, "showView"))?;
        let show_id = match show_info.get("current_id") {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => return Err(super::clyp::key_error(&playlist_id, "current_id")),
        };
        let title = show_info
            .get("show")
            .and_then(|show| show.get("title"))
            .and_then(|title| title.get("text"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        // `OnDemandPagedList`: pages of 10 until a page that is not full.
        let mut entries = Vec::new();
        for page in 0.. {
            let data = show_page(client, &show_id, &season_id, page + 1)?;
            let page_entries = yield_items(data.as_array().map_or(&[], Vec::as_slice), &EPISODE)?;
            let full = page_entries.len() >= SHOW_PAGE_SIZE;
            entries.extend(page_entries);
            if !full {
                break;
            }
        }
        Ok(Some(list_result(
            playlist_id,
            title,
            url,
            (SHOW_PLAYLIST_NAME, SHOW_PLAYLIST_KEY),
            entries,
        )))
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
