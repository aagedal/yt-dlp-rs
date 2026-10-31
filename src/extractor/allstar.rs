//! Upstream `AllstarIE` and `AllstarProfileIE` (`yt_dlp/extractor/allstar.py`):
//! a clip or montage from the GraphQL API as a direct MP4 (with its
//! thumbnails and uploader), and a user's clips, montages or mobile clips as
//! a paged playlist of those.
use super::{Extractor, download_json, post_json_note_headers};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Allstar;
pub struct AllstarProfile;

const NAME: &str = "Allstar";
const PROFILE_NAME: &str = "AllstarProfile";
const GRAPHQL: &str = "https://a1.allstar.gg/graphql";
const MEDIA: &str = "https://media.allstar.gg/";
const PAGE_SIZE: usize = 10;

/// Upstream's `(?P<type>...)\?(?P=type)=`: the query key must repeat the type,
/// which the regex crate cannot express, so the two are compared.
static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?allstar\.gg/(?P<type>clip|montage)\?(?P<key>clip|montage)=(?P<id>[^/?#&]+)",
    )
    .unwrap()
});

/// The `(type, id)` of a clip or montage URL.
fn video_match(url: &str) -> Option<(String, String)> {
    let found = VALID_URL.captures(url)?;
    (found["type"] == found["key"]).then(|| (found["type"].to_owned(), found["id"].to_owned()))
}
static PROFILE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?allstar\.gg/(?:profile\?user=|u/)(?P<id>[^/?#&]+)").unwrap()
});

const FIELDS: &str = "
    _id
    clipImageSource
    clipImageThumb
    clipLink
    clipTitle
    createdDate
    shareId
    user { _id }
    username
    views";
const EXTRA_FIELDS: &str = "
    clipLength
    clipSizeBytes";

/// The GraphQL query text of a `type` (`_QUERIES`).
fn query_for(kind: &str) -> Option<String> {
    Some(match kind {
        "clip" => format!(
            "query ($id: String!) {{ video: getClip(clipIdentifier: $id) {{ {FIELDS} {EXTRA_FIELDS} }} }}"
        ),
        "montage" => {
            format!(
                "query ($id: String!) {{ video: getMontage(clipIdentifier: $id) {{ {FIELDS} }} }}"
            )
        }
        "Clips" => format!(
            "query ($page: Int!, $user: String!, $game: Int) {{ videos: clips(search: createdDate, page: $page, user: $user, mobile: false, game: $game) {{ data {{ {FIELDS} {EXTRA_FIELDS} }} }} }}"
        ),
        "Montages" => format!(
            "query ($page: Int!, $user: String!) {{ videos: montages(search: createdDate, page: $page, user: $user) {{ data {{ {FIELDS} }} }} }}"
        ),
        "Mobile Clips" => format!(
            "query ($page: Int!, $user: String!) {{ videos: clips(search: createdDate, page: $page, user: $user, mobile: true) {{ data {{ {FIELDS} {EXTRA_FIELDS} }} }} }}"
        ),
        _ => return None,
    })
}

/// `_match_valid_url` of [`Allstar`].
pub fn suitable(url: &str) -> bool {
    video_match(url).is_some()
}

/// `_match_valid_url` of [`AllstarProfile`].
pub fn profile_suitable(url: &str) -> bool {
    PROFILE_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `urljoin(base, path)`: a path with a scheme (`b2://...`) stays as it is.
fn urljoin(base: &str, path: &str) -> Option<String> {
    static SCHEME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:[a-zA-Z][a-zA-Z0-9+.-]*:)?//").unwrap());
    if path.is_empty() {
        return None;
    }
    if SCHEME.is_match(path) {
        return Some(path.to_owned());
    }
    Url::parse(base)
        .and_then(|base| base.join(path))
        .map(String::from)
        .ok()
}

/// A string-valued key (`str`), absent for other types.
fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

/// `_parse_video_data(video_data)`: the clip as a direct MP4 info whose
/// keys are the upstream dict's, in its order.
fn parse_video_data(video: &Value) -> VideoInfo {
    let id = text(video.get("_id"));
    let display_id = text(video.get("shareId"));
    let title = text(video.get("clipTitle"));
    let url = text(video.get("clipLink")).and_then(|path| urljoin(MEDIA, &path));
    let thumbnails: Vec<Value> = ["clipImageThumb", "clipImageSource"]
        .iter()
        .filter_map(|key| text(video.get(*key)))
        .filter_map(|path| urljoin(MEDIA, &path))
        .map(|url| serde_json::json!({ "url": url }))
        .collect();
    let duration = super::streamable::int_or_none(video.get("clipLength"));
    let filesize = super::streamable::int_or_none(video.get("clipSizeBytes"));
    let timestamp = video
        .get("createdDate")
        .and_then(Value::as_i64)
        .or_else(|| {
            video
                .get("createdDate")
                .and_then(Value::as_str)
                .and_then(|text| text.parse::<i64>().ok())
        })
        .map(|millis| millis / 1000);
    let uploader = text(video.get("username"));
    let uploader_id = text(video.get("user").and_then(|user| user.get("_id")));
    let view_count = super::streamable::int_or_none(video.get("views"));

    let mut fields: Vec<(&str, Value)> = Vec::new();
    if let Some(id) = &id {
        fields.push(("id", id.clone().into()));
    }
    if let Some(display_id) = display_id {
        fields.push(("display_id", display_id.into()));
    }
    if let Some(title) = &title {
        fields.push(("title", title.clone().into()));
    }
    if let Some(url) = &url {
        fields.push(("url", url.clone().into()));
    }
    if !thumbnails.is_empty() {
        fields.push(("thumbnails", Value::Array(thumbnails)));
    }
    if !duration.is_null() {
        fields.push(("duration", duration));
    }
    if !filesize.is_null() {
        fields.push(("filesize", filesize));
    }
    if let Some(timestamp) = timestamp {
        fields.push(("timestamp", timestamp.into()));
    }
    if let Some(uploader) = uploader {
        fields.push(("uploader", uploader.into()));
    }
    if let Some(uploader_id) = &uploader_id {
        fields.push(("uploader_id", uploader_id.clone().into()));
    }
    if !view_count.is_null() {
        fields.push(("view_count", view_count));
    }
    if let (Some(id), Some(url)) = (&id, &url) {
        let basename = if url.contains("/clips/") {
            "clip"
        } else {
            "montage"
        };
        fields.push((
            "webpage_url",
            format!("https://allstar.gg/{basename}?{basename}={id}").into(),
        ));
    }
    let uploader_url = uploader_id
        .as_deref()
        .and_then(|uploader_id| urljoin("https://allstar.gg/u/", uploader_id));
    fields.push(("extractor_key", NAME.into()));
    fields.push(("extractor", NAME.into()));
    if let Some(uploader_url) = uploader_url {
        fields.push(("uploader_url", uploader_url.into()));
    }
    video_info(fields, id.unwrap_or_default(), url)
}

/// The video info of a dict's `fields`: `url` as its one format (the
/// extractor keys set in place), with `None` keys left out.
fn video_info(fields: Vec<(&str, Value)>, id: String, url: Option<String>) -> VideoInfo {
    let page = Url::parse("https://allstar.gg/").expect("static URL");
    let media = url
        .as_deref()
        .and_then(|url| Url::parse(url).ok())
        .unwrap_or_else(|| page.clone());
    let mut info = super::direct::with_format(
        super::direct::build_at(media.clone(), &media, "mp4".into()),
        "",
    );
    for format in &mut info.formats {
        format.ext = "mp4".into();
        format.format_id = String::new();
    }
    info.format_id = None;
    info.direct = None;
    info.id = id;
    let mut keys = Vec::new();
    for (name, value) in fields {
        keys.push(name.to_owned());
        match name {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            // The untyped dict is its own format.
            "filesize" => {
                for format in &mut info.formats {
                    format.filesize = value.as_u64();
                }
                crate::metadata_parser::set_field(&mut info, name, value);
            }
            "webpage_url" => {
                info.webpage_url =
                    Url::parse(value.as_str().unwrap_or_default()).unwrap_or_else(|_| page.clone());
            }
            "extractor" => info.extractor = value.as_str().unwrap_or_default().to_owned(),
            "extractor_key" => {
                info.extractor_key = value.as_str().unwrap_or_default().to_owned();
            }
            "url" => {}
            _ => crate::metadata_parser::set_field(&mut info, name, value),
        }
    }
    crate::info_order::mark_info(info, &format!("untyped-dict;{}", keys.join(",")))
}

/// `_call_api(query, variables, path, video_id, note)`: the GraphQL
/// response's `path`, or its errors.
fn call_api(
    client: &Client,
    kind: &str,
    variables: Value,
    path: &[&str],
    id: &str,
    note: Option<&str>,
) -> Result<Value> {
    let query = query_for(kind)
        .ok_or_else(|| Error::Reported(format!("[{NAME}] {id}: Unsupported query {kind:?}")))?;
    let body = serde_json::json!({ "variables": variables, "query": query }).to_string();
    let response = post_json_note_headers(
        client,
        &Url::parse(GRAPHQL)?,
        NAME,
        id,
        note.unwrap_or("Downloading JSON metadata"),
        &[("content-type", "application/json")],
        body,
    )?;
    if let Some(errors) = response.get("errors").and_then(Value::as_array) {
        let messages: Vec<&str> = errors
            .iter()
            .filter_map(|error| error.get("message").and_then(Value::as_str))
            .collect();
        if !messages.is_empty() {
            return Err(Error::Reported(messages.join("; ")));
        }
    }
    let mut value = &response;
    for step in path {
        value = match value.get(*step) {
            Some(next) => next,
            None => return Ok(Value::Null),
        };
    }
    Ok(value.clone())
}

impl Extractor for Allstar {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some((query_id, video_id)) = video_match(url.as_str()) else {
            return Ok(None);
        };
        announce(NAME, url);
        let kind = if query_id == "clip" {
            "clip"
        } else {
            "montage"
        };
        let video = call_api(
            client,
            kind,
            serde_json::json!({ "id": video_id }),
            &["data", "video"],
            &video_id,
            None,
        )?;
        let info = parse_video_data(&video);
        if info.id.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] Missing \"id\" field in extractor result; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        Ok(Some(ExtractResult::Video(info)))
    }
}

impl Extractor for AllstarProfile {
    fn key(&self) -> &'static str {
        PROFILE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PROFILE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        announce(PROFILE_NAME, url);
        let profile = download_json(
            client,
            &Url::parse(&format!(
                "https://api.allstar.gg/v1/users/profile/{display_id}"
            ))?,
            PROFILE_NAME,
            &display_id,
        )?;
        let user_id = profile
            .get("data")
            .and_then(|data| data.get("_id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{PROFILE_NAME}] {display_id}: Unable to extract the user id; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let username = text(
            profile
                .get("data")
                .and_then(|data| data.get("profile"))
                .and_then(|profile| profile.get("username")),
        );

        let query_pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        let game = query_pairs
            .iter()
            .find(|(key, _)| key == "game")
            .and_then(|(_, value)| value.parse::<i64>().ok());
        let query_id = query_pairs
            .iter()
            .find(|(key, _)| key == "view")
            .map_or("Clips", |(_, value)| value.as_str())
            .to_owned();
        if !matches!(query_id.as_str(), "Clips" | "Montages" | "Mobile Clips") {
            return Err(Error::Reported(format!(
                "Unsupported playlist URL type {query_id:?}"
            )));
        }

        let mut entries = Vec::new();
        for page in 1.. {
            let variables = match game {
                Some(game) => serde_json::json!({ "user": user_id, "page": page, "game": game }),
                None => serde_json::json!({ "user": user_id, "page": page, "game": null }),
            };
            let videos = call_api(
                client,
                &query_id,
                variables,
                &["data", "videos", "data"],
                &display_id,
                Some(&format!("Downloading page {page}")),
            )?;
            let videos = videos.as_array().cloned().unwrap_or_default();
            entries.extend(videos.iter().map(parse_video_data));
            if videos.len() < PAGE_SIZE {
                break;
            }
        }

        let query_word = query_id.to_lowercase();
        let query_word = query_word.split_whitespace().next().unwrap_or_default();
        let playlist_id = [
            Some(user_id.clone()),
            Some(query_word.to_owned()),
            game.map(|game| game.to_string()),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
        let title = [
            Some(username.unwrap_or(display_id)),
            Some(query_id.clone()),
            game.map(|game| game.to_string()),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" - ");
        let mut extra = Map::new();
        // `playlist_result(...)`: `id`, `title`, then `_type` and `entries`.
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "youtube-tab-plain;id,title,_type,entries".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title,
            webpage_url: url.clone(),
            extractor: PROFILE_NAME.into(),
            extractor_key: PROFILE_NAME.into(),
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
            "https://allstar.gg/clip?clip=64482c2da9eec30008a67d1b"
        ));
        assert!(suitable("https://allstar.gg/montage?montage=RILJMH6QOS"));
        assert!(!suitable(
            "https://allstar.gg/profile?user=62b8bdfc9021052f7905882d"
        ));
        assert!(profile_suitable(
            "https://allstar.gg/profile?user=62b8bdfc9021052f7905882d"
        ));
        assert!(profile_suitable(
            "https://allstar.gg/u/cherokee?game=730&view=Clips"
        ));
    }

    #[test]
    fn scheme_paths_are_kept() {
        assert_eq!(
            urljoin(MEDIA, "b2://bucket/clip.mp4").as_deref(),
            Some("b2://bucket/clip.mp4")
        );
        assert_eq!(
            urljoin(MEDIA, "62b8/montage/a.mp4").as_deref(),
            Some("https://media.allstar.gg/62b8/montage/a.mp4")
        );
    }
}
