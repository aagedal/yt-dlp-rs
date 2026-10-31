//! Upstream `MixcloudIE` (`yt_dlp/extractor/mixcloud.py`): the GraphQL
//! cloudcast lookup (requested with upstream's impersonation warning), its
//! XOR-encrypted stream URLs (progressive, HLS, DASH), comments and
//! metadata.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Mixcloud;

const NAME: &str = "mixcloud";
const KEY: &str = "Mixcloud";
const DECRYPTION_KEY: &[u8] = b"IFYOUWANTTHEARTISTSTOGETPAIDDONOTDOWNLOADFROMMIXCLOUD";
const CLOUDCAST_FIELDS: &str = "audioLength
    comments(first: 100) {
      edges {
        node {
          comment
          created
          user {
            displayName
            username
          }
        }
      }
      totalCount
    }
    description
    favorites {
      totalCount
    }
    featuringArtistList
    isExclusive
    name
    owner {
      displayName
      url
      username
    }
    picture(width: 1024, height: 1024) {
        url
    }
    plays
    publishDate
    reposts {
      totalCount
    }
    streamInfo {
      dashUrl
      hlsUrl
      url
    }
    tags {
      tag {
        name
      }
    }
    restrictedReason
    id";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:www|beta|m)\.)?mixcloud\.com/(?P<user>[^/]+)/(?P<slug>[^/]+)",
    )
    .unwrap()
});

/// `MixcloudIE._match_valid_url` (`(?!stream|uploads|...)` checked apart).
fn match_url(url: &str) -> Option<(String, String)> {
    let found = VALID_URL.captures(url)?;
    let slug = &found["slug"];
    if ["stream", "uploads", "favorites", "listens", "playlists"]
        .iter()
        .any(|reserved| slug.starts_with(reserved))
    {
        return None;
    }
    let decode = |text: &str| {
        percent_encoding::percent_decode_str(text)
            .decode_utf8_lossy()
            .into_owned()
    };
    Some((decode(&found["user"]), decode(slug)))
}

/// `MixcloudIE.suitable`.
pub fn suitable(url: &str) -> bool {
    match_url(url).is_some()
}

/// `_decrypt_xor_cipher(key, base64.b64decode(url))`.
fn decrypt(encoded: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    Some(
        bytes
            .iter()
            .zip(DECRYPTION_KEY.iter().cycle())
            .map(|(byte, key)| char::from(byte ^ key))
            .collect(),
    )
}

/// An `ExtractorError`: the URL pattern has no `id` group, so there is no
/// temporary ID before the message.
fn fail(_id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {message}"))
}

fn int(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
        .map(Value::from)
}

impl Extractor for Mixcloud {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some((username, slug)) = match_url(url.as_str()) else {
            return Ok(None);
        };
        let track_id = format!("{username}_{slug}");
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let query = format!(
            "{{\n  cloudcastLookup(lookup: {{username: \"{username}\", slug: \"{slug}\"}}) {{\n    {CLOUDCAST_FIELDS}\n  }}\n}}"
        );
        let mut api = Url::parse("https://app.mixcloud.com/graphql")?;
        api.query_pairs_mut().append_pair("query", &query);
        let response = {
            let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
                crate::screen::warning_once(&format!("[{NAME}] {message}"));
            });
            super::download_json(client, &api, NAME, &track_id)?
        };
        let cloudcast = response
            .pointer("/data/cloudcastLookup")
            .filter(|value| !value.is_null())
            .cloned()
            .ok_or_else(|| fail(&track_id, "Track not found"))?;
        match cloudcast.get("restrictedReason").and_then(Value::as_str) {
            Some("tracklist") => {
                return Err(fail(
                    &track_id,
                    "Track unavailable in your country due to licensing restrictions",
                ));
            }
            Some("repeat_play") => {
                return Err(fail(
                    &track_id,
                    "You have reached your play limit for this track",
                ));
            }
            Some(reason) if !reason.is_empty() => {
                return Err(fail(&track_id, "Track is restricted"));
            }
            _ => {}
        }
        let stream_info = cloudcast.get("streamInfo").cloned().unwrap_or(Value::Null);
        let mut formats = Vec::new();
        for key in ["url", "hlsUrl", "dashUrl"] {
            let Some(encoded) = stream_info
                .get(key)
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty())
            else {
                continue;
            };
            let Some(decrypted) = decrypt(encoded) else {
                continue;
            };
            match key {
                "hlsUrl" => {
                    if let Some(stream) = Url::parse(&decrypted).ok().and_then(|manifest| {
                        super::extract_m3u8_optional(
                            client,
                            &manifest,
                            NAME,
                            &track_id,
                            Some("hls"),
                            false,
                        )
                    }) {
                        formats.extend(stream.formats);
                    }
                }
                "dashUrl" => {
                    if let Some(stream) = Url::parse(&decrypted).ok().and_then(|manifest| {
                        super::extract_mpd_optional_note(
                            client,
                            &manifest,
                            NAME,
                            &track_id,
                            Some("dash"),
                            "Downloading MPD manifest",
                        )
                    }) {
                        let mut stream = stream;
                        if !stream.subtitles.is_empty() {
                            crate::screen::warning_once(&format!(
                                "[{NAME}] Ignoring subtitle tracks found in the DASH manifest; if any subtitle tracks are missing, {}",
                                crate::BUG_REPORTS_MESSAGE
                            ));
                            stream.subtitles = Default::default();
                        }
                        formats.extend(stream.formats);
                    }
                }
                _ => {
                    let ext = super::direct::determine_ext(&decrypted)
                        .unwrap_or_else(|| "unknown_video".into());
                    let mut format: crate::VideoFormat = serde_json::from_value(json!({
                        "format_id": "http",
                        "url": decrypted,
                        "vcodec": "none",
                        "downloader_options": {"http_chunk_size": 5242880},
                        "ext": ext,
                    }))
                    .map_err(|error| fail(&track_id, &error.to_string()))?;
                    crate::info_order::mark_format(
                        &mut format,
                        "dict:format_id,url,vcodec,downloader_options",
                    );
                    formats.push(format);
                }
            }
        }
        if formats.is_empty() && cloudcast.get("isExclusive").and_then(Value::as_bool) == Some(true)
        {
            return Err(fail(
                &track_id,
                "This video is only available for registered users. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies",
            ));
        }
        let mut comments = Vec::new();
        for node in cloudcast
            .pointer("/comments/edges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|edge| edge.get("node").filter(|node| node.is_object()))
        {
            let Some(text) = node
                .get("comment")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            let mut comment = Map::new();
            comment.insert("text".into(), text.into());
            if let Some(author) = node.pointer("/user/displayName").filter(|v| v.is_string()) {
                comment.insert("author".into(), author.clone());
            }
            if let Some(author_id) = node.pointer("/user/username").filter(|v| v.is_string()) {
                comment.insert("author_id".into(), author_id.clone());
            }
            if let Some(timestamp) = node
                .get("created")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
            {
                comment.insert("timestamp".into(), timestamp.into());
            }
            comments.push(Value::Object(comment));
        }
        let mut dict = Map::new();
        dict.insert("id".into(), track_id.clone().into());
        dict.insert("formats".into(), json!([]));
        dict.insert("comments".into(), Value::Array(comments));
        let text = |path: &str| {
            cloudcast
                .pointer(path)
                .filter(|value| value.is_string())
                .cloned()
        };
        let list = |values: Vec<Value>| (!values.is_empty()).then_some(Value::Array(values));
        let tags: Vec<Value> = cloudcast
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| tag.pointer("/tag/name").and_then(Value::as_str))
            .filter(|tag| !tag.is_empty())
            .map(Value::from)
            .collect();
        let artists: Vec<Value> = cloudcast
            .get("featuringArtistList")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|artist| !artist.is_empty())
            .map(Value::from)
            .collect();
        for (key, value) in [
            ("title", text("/name")),
            ("description", text("/description")),
            (
                "thumbnail",
                cloudcast
                    .pointer("/picture/url")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .map(Value::from),
            ),
            (
                "timestamp",
                cloudcast
                    .get("publishDate")
                    .and_then(Value::as_str)
                    .and_then(crate::dates::parse_iso8601)
                    .map(Value::from),
            ),
            ("duration", int(cloudcast.get("audioLength"))),
            ("uploader", text("/owner/displayName")),
            ("uploader_id", text("/owner/username")),
            (
                "uploader_url",
                cloudcast
                    .pointer("/owner/url")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .map(Value::from),
            ),
            ("view_count", int(cloudcast.get("plays"))),
            (
                "like_count",
                int(cloudcast.pointer("/favorites/totalCount")),
            ),
            (
                "repost_count",
                int(cloudcast.pointer("/reposts/totalCount")),
            ),
            (
                "comment_count",
                int(cloudcast.pointer("/comments/totalCount")),
            ),
            ("tags", list(tags)),
            ("artists", list(artists)),
        ] {
            if let Some(value) = value {
                dict.insert(key.into(), value);
            }
        }
        let first = formats
            .first()
            .map_or_else(|| url.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        if formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.formats = formats;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        let titled = dict.contains_key("title");
        if !titled {
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        let mut keys = Vec::new();
        for (key, value) in dict {
            keys.push(key.clone());
            match key.as_str() {
                "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
                "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                "formats" => {}
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!(
                "dict{};{}",
                if titled { "" } else { "-untitled" },
                keys.join(",")
            ),
        )))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

// Upstream `MixcloudUserIE` and `MixcloudPlaylistIE` (`MixcloudPlaylistBaseIE`):
// paged GraphQL `userLookup` / `playlistLookup` results of cloudcast URLs.

const USER_NAME: &str = "mixcloud:user";
const USER_KEY: &str = "MixcloudUser";
const PLAYLIST_NAME: &str = "mixcloud:playlist";
const PLAYLIST_KEY: &str = "MixcloudPlaylist";
const USER_NODE: &str = "slug
          url
          owner { username }";
const PLAYLIST_NODE: &str = "cloudcast {
            slug
            url
            owner { username }
          }";

static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?mixcloud\.com/(?P<id>[^/]+)/(?P<type>uploads|favorites|listens|stream)?/?$",
    )
    .unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?mixcloud\.com/(?P<user>[^/]+)/playlists/(?P<playlist>[^/]+)/?$",
    )
    .unwrap()
});

/// `MixcloudUserIE.suitable`.
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

/// `MixcloudPlaylistIE.suitable`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `url_result(url, MixcloudIE, video_id)` as a playlist entry.
fn url_entry(target: &str, id: Option<String>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), KEY.into());
    if let Some(id) = id {
        entry.insert("id".into(), id.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `MixcloudPlaylistBaseIE._call_api` plus its `_real_extract` paging for
/// the `user` (`userLookup`, slug = uploads, favorites, ...) or `playlist`
/// (`playlistLookup`) root.
fn playlist_extract(
    client: &Client,
    url: &Url,
    (name, key): (&'static str, &'static str),
    username: &str,
    slug: &str,
) -> Result<Option<ExtractResult>> {
    let is_playlist = name == PLAYLIST_NAME;
    let (root, title_key, description_key, node) = if is_playlist {
        ("playlist", "name", "description", PLAYLIST_NODE)
    } else {
        ("user", "displayName", "biog", USER_NODE)
    };
    let list_type = if is_playlist { "items" } else { slug };
    let lookup_key = format!("{root}Lookup");
    let lookup_slug = if is_playlist {
        format!(", slug: \"{slug}\"")
    } else {
        String::new()
    };
    let playlist_id = format!("{username}_{slug}");
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));

    let mut entries = Vec::new();
    let mut after: Option<String> = None;
    let playlist = loop {
        let list_filter = after
            .as_deref()
            .map_or_else(String::new, |cursor| format!(", after: \"{cursor}\""));
        let query = format!(
            "{{\n  {lookup_key}(lookup: {{username: \"{username}\"{lookup_slug}}}) {{\n    {title_key}\n    {description_key}\n    {list_type}(first: 100{list_filter}) {{\n      edges {{\n        node {{\n          {node}\n        }}\n      }}\n      pageInfo {{\n        endCursor\n        hasNextPage\n      }}\n    }}\n  }}\n}}"
        );
        let mut api = Url::parse("https://app.mixcloud.com/graphql")?;
        api.query_pairs_mut().append_pair("query", &query);
        let response = {
            let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
                crate::screen::warning_once(&format!("[{name}] {message}"));
            });
            super::download_json(client, &api, name, &playlist_id)?
        };
        let playlist = response
            .pointer(&format!("/data/{lookup_key}"))
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(|| key_error(&playlist_id, "data"))?;
        let items = playlist
            .get(list_type)
            .filter(|value| truthy(Some(value)))
            .cloned()
            .unwrap_or_else(|| json!({}));
        for edge in items
            .get("edges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let node = edge
                .get("node")
                .filter(|value| truthy(Some(value)))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let cloudcast = if is_playlist {
                node.get("cloudcast")
                    .filter(|value| truthy(Some(value)))
                    .cloned()
                    .unwrap_or_else(|| json!({}))
            } else {
                node
            };
            let Some(cloudcast_url) = cloudcast
                .get("url")
                .and_then(Value::as_str)
                .filter(|target| !target.is_empty())
            else {
                continue;
            };
            let item_slug = cloudcast.get("slug").and_then(Value::as_str);
            let owner = cloudcast.pointer("/owner/username").and_then(Value::as_str);
            let video_id = match (item_slug, owner) {
                (Some(item_slug), Some(owner)) if !item_slug.is_empty() && !owner.is_empty() => {
                    Some(format!("{owner}_{item_slug}"))
                }
                _ => None,
            };
            entries.push(url_entry(cloudcast_url, video_id)?);
        }
        let page_info = items
            .get("pageInfo")
            .ok_or_else(|| key_error(&playlist_id, "pageInfo"))?;
        let has_next_page = truthy(page_info.get("hasNextPage"));
        after = page_info
            .get("endCursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if !has_next_page {
            break playlist;
        }
    };

    let raw_title = playlist
        .get(title_key)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let title = if is_playlist {
        raw_title.to_owned()
    } else {
        format!("{raw_title} ({slug})")
    };
    let description = playlist
        .get(description_key)
        .filter(|value| truthy(Some(value)))
        .cloned();
    let mut shape = vec!["id", "title"];
    let mut extra = Map::new();
    if let Some(description) = description {
        extra.insert("description".into(), description);
        shape.push("description");
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", shape.join(",")).into(),
    );
    Ok(Some(ExtractResult::Playlist(PlaylistInfo {
        id: playlist_id,
        title,
        webpage_url: url.clone(),
        extractor: name.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })))
}

/// Python's truthiness for a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

/// `KeyError` from a missing field: no `[IE]` prefix.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

pub struct MixcloudUser;

impl Extractor for MixcloudUser {
    fn key(&self) -> &'static str {
        USER_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = USER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let username = found["id"].to_owned();
        let slug = found
            .name("type")
            .map_or("uploads", |kind| kind.as_str())
            .to_owned();
        playlist_extract(client, url, (USER_NAME, USER_KEY), &username, &slug)
    }
}

pub struct MixcloudPlaylist;

impl Extractor for MixcloudPlaylist {
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
        let username = found["user"].to_owned();
        let slug = found["playlist"].to_owned();
        playlist_extract(client, url, (PLAYLIST_NAME, PLAYLIST_KEY), &username, &slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert_eq!(
            match_url("http://www.mixcloud.com/dholbach/cryptkeeper/"),
            Some(("dholbach".into(), "cryptkeeper".into()))
        );
        assert!(suitable(
            "https://beta.mixcloud.com/RedLightRadio/nosedrip-15-red-light-radio-01-18-2016/"
        ));
        assert!(!suitable("https://www.mixcloud.com/dholbach/uploads/"));
        assert!(!suitable("https://www.mixcloud.com/dholbach/"));
    }

    #[test]
    fn stream_urls_are_xor_decrypted() {
        let plain = "https://stream.example/a.m4a";
        let cipher: Vec<u8> = plain
            .bytes()
            .zip(DECRYPTION_KEY.iter().cycle())
            .map(|(byte, key)| byte ^ key)
            .collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(cipher);
        assert_eq!(decrypt(&encoded).as_deref(), Some(plain));
    }

    #[test]
    fn user_and_playlist_urls_follow_upstream_valid_url() {
        assert!(user_suitable("http://www.mixcloud.com/dholbach/"));
        assert!(user_suitable("https://www.mixcloud.com/FirstEar/stream/"));
        assert!(!user_suitable(
            "https://www.mixcloud.com/dholbach/cryptkeeper/"
        ));
        assert!(playlist_suitable(
            "https://www.mixcloud.com/maxvibes/playlists/jazzcat-on-ness-radio/"
        ));
        assert!(!playlist_suitable("https://www.mixcloud.com/maxvibes/"));
    }
}
