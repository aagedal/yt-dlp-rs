//! Upstream `GameJoltIE`, `GameJoltGameIE` and `GameJoltGameSoundtrackIE`
//! (`yt_dlp/extractor/gamejolt.py`): a post's `site-api` JSON with its HLS,
//! DASH, MP4 and image media, a game's post feed, and a soundtrack's song
//! URLs. GIF posts, article-less comment handling and the other gamejolt
//! classes (users, communities, search) are not supported natively.
use super::{
    Extractor, dict_format, dict_info, direct, download_json_headers, extract_m3u8_fatal_master,
    extract_mpd_fatal, post_json_note_headers,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct GameJolt;
pub struct GameJoltGame;
pub struct GameJoltGameSoundtrack;

const NAME: &str = "GameJolt";
const GAME_NAME: &str = "GameJoltGame";
const SOUNDTRACK_NAME: &str = "GameJoltGameSoundtrack";
const API_BASE: &str = "https://gamejolt.com/site-api/";
const ACCEPT: [(&str, &str); 1] = [("Accept", "image/webp,*/*")];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"https?://(?:www\.)?gamejolt\.com/p/(?:[\w-]*-)?(?P<id>\w{8})")
        .unwrap()
});
static GAME_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"https?://(?:www\.)?gamejolt\.com/games/[\w-]+/(?P<id>\d+)")
        .unwrap()
});
static SOUNDTRACK_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"https?://(?:www\.)?gamejolt\.com/get/soundtrack(?:\?|\#!?)(?:.*?[&;])??game=(?P<id>(?:\d+)+)",
    )
    .unwrap()
});

/// `_match_id` of [`GameJolt`].
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id` of [`GameJoltGame`].
pub fn game_suitable(url: &str) -> bool {
    GAME_URL.is_match(url)
}

/// `_match_id` of [`GameJoltGameSoundtrack`].
pub fn soundtrack_suitable(url: &str) -> bool {
    SOUNDTRACK_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("GameJolt {what} are not supported natively"))
}

/// `value[key]`'s `KeyError`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_call_api(endpoint)['payload']`.
fn call_api(client: &Client, endpoint: &str, id: &str) -> Result<Value> {
    let url = Url::parse(&format!("{API_BASE}{endpoint}"))?;
    let body = download_json_headers(client, &url, NAME, id, &ACCEPT)?;
    body.get("payload")
        .cloned()
        .ok_or_else(|| key_error(id, "payload"))
}

/// Upstream `_parse_content_as_text`: a rich-text document's paragraphs
/// joined with newlines.
fn content_as_text(content: &Value) -> String {
    let outer = content
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut joined = Vec::new();
    for outer_content in &outer {
        if outer_content.get("type").and_then(Value::as_str) != Some("paragraph") {
            joined.push(content_as_text(outer_content));
            continue;
        }
        let mut text = String::new();
        for inner in outer_content
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match inner.get("text").and_then(Value::as_str) {
                Some(part) if !part.is_empty() => text.push_str(part),
                _ if inner.get("type").and_then(Value::as_str) == Some("hardBreak") => {
                    text.push('\n');
                }
                _ => {}
            }
        }
        joined.push(text);
    }
    joined.join("\n")
}

/// `_parse_json(value, id, fatal=False) or {}` for a JSON string field.
fn parse_embedded(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .unwrap_or_else(|| json!({}))
}

/// `traverse_obj(lead_content, ('content', ..., 'content', ..., 'marks',
/// ..., 'attrs', 'tag'))`: the hashtag marks, in order.
fn lead_tags(lead: &Value) -> Vec<Value> {
    let mut tags = Vec::new();
    for paragraph in lead
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for inner in paragraph
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for mark in inner
                .get("marks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(tag) = mark
                    .get("attrs")
                    .and_then(|attrs| attrs.get("tag"))
                    .filter(|tag| tag.is_string())
                {
                    tags.push(tag.clone());
                }
            }
        }
    }
    tags
}

/// Upstream `int_or_none(value, scale=...)`.
fn scaled_int(value: Option<&Value>, scale: i64) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64().map(|number| number / scale),
        Value::String(text) => text.trim().parse::<i64>().ok().map(|number| number / scale),
        _ => None,
    }
}

/// `int_or_none(value)`.
fn int_value(value: Option<&Value>) -> Option<i64> {
    scaled_int(value, 1)
}

/// What [`parse_post`] makes of a post.
#[allow(clippy::large_enum_variant)]
enum Post {
    Video(VideoInfo),
    /// `url_result(embed_url)`: followed by the caller.
    Embed(String),
}

/// Upstream `_parse_post`: a post's video (or embed) and its metadata.
fn parse_post(client: &Client, post: &Value) -> Result<Post> {
    let post_id = post
        .get("hash")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error("", "hash"))?
        .to_owned();
    let lead_content = parse_embedded(post.get("lead_content"));
    let description = match post.get("leadStr").and_then(Value::as_str) {
        Some(text) if !text.is_empty() => text.to_owned(),
        _ => content_as_text(&lead_content),
    };
    let mut full_description = None;
    if post
        .get("has_article")
        .is_some_and(|value| value.as_bool() == Some(true))
    {
        let article = match post.get("article_content").and_then(Value::as_str) {
            Some(text) if !text.is_empty() => serde_json::from_str::<Value>(text).ok(),
            _ => {
                let post_key = post.get("id").map_or_else(String::new, |id| match id {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                });
                let fetched = call_api(client, &format!("web/posts/article/{post_key}"), &post_id)?;
                fetched.get("article").cloned()
            }
        };
        full_description = Some(content_as_text(&article.unwrap_or_else(|| json!({}))));
    }

    let user = post.get("user").cloned().unwrap_or_else(|| json!({}));
    let user_display = user
        .get("display_name")
        .or_else(|| user.get("name"))
        .filter(|value| value.as_str().is_some_and(|text| !text.is_empty()))
        .cloned();
    let categories: Vec<Value> = post
        .get("communities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|category| {
            let name = category
                .get("community")
                .and_then(|community| community.get("name"));
            let channel = category.get("channel");
            let title = channel
                .and_then(|channel| {
                    channel
                        .get("display_title")
                        .filter(|value| value.as_str().is_some_and(|text| !text.is_empty()))
                        .or_else(|| channel.get("title"))
                })
                .and_then(Value::as_str);
            match (name.and_then(Value::as_str), title) {
                (Some(name), Some(title)) => Value::from(format!("{name} - {title}")),
                _ => Value::Null,
            }
        })
        .collect();
    let tags = lead_tags(&lead_content);
    let uploader_url = user
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(|url| format!("https://gamejolt.com{url}"));

    let webpage_url = post
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map_or_else(
            || format!("https://gamejolt.com/p/{post_id}"),
            str::to_owned,
        );
    let mut fields: Vec<(&str, Value)> = vec![
        ("extractor_key", NAME.into()),
        ("extractor", NAME.into()),
        ("webpage_url", webpage_url.clone().into()),
        ("id", post_id.clone().into()),
        ("title", description.clone().into()),
        (
            "description",
            full_description
                .unwrap_or_else(|| description.clone())
                .into(),
        ),
        (
            "display_id",
            post.get("slug").cloned().unwrap_or(Value::Null),
        ),
        ("uploader", user_display.unwrap_or(Value::Null)),
        (
            "uploader_id",
            user.get("username").cloned().unwrap_or(Value::Null),
        ),
        (
            "uploader_url",
            uploader_url.map_or(Value::Null, Value::from),
        ),
        ("categories", categories.into()),
        // `traverse_obj` gives an empty list for a post without hashtags.
        ("tags", Value::Array(tags)),
        (
            "like_count",
            int_value(post.get("like_count")).map_or(Value::Null, Value::from),
        ),
        (
            "comment_count",
            int_value(post.get("comment_count")).unwrap_or(0).into(),
        ),
        (
            "timestamp",
            scaled_int(post.get("added_on"), 1000).map_or(Value::Null, Value::from),
        ),
        (
            "release_timestamp",
            scaled_int(post.get("published_on"), 1000).map_or(Value::Null, Value::from),
        ),
    ];
    // `embed_url` is a post without video formats: the GIF and embed paths.
    let video = post
        .get("videos")
        .and_then(Value::as_array)
        .and_then(|videos| videos.iter().find(|video| video.is_object()))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut formats = Vec::new();
    let mut thumbnails = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for media in video
        .get("media")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let media_url = media
            .get("img_url")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&post_id, "img_url"))?
            .to_owned();
        let mimetype = media.get("filetype").and_then(Value::as_str).unwrap_or("");
        let ext = direct::determine_ext(&media_url).unwrap_or_default();
        let media_id = media.get("type").cloned().unwrap_or(Value::Null);
        let media_id_text = media_id.as_str().map(str::to_owned);
        if mimetype == "application/vnd.apple.mpegurl" || ext == "m3u8" {
            let manifest = Url::parse(&media_url)?;
            let stream = extract_m3u8_fatal_master(
                client,
                &manifest,
                NAME,
                &post_id,
                media_id_text.as_deref(),
                true,
            )?;
            formats.extend(stream.formats);
            merge_subtitles(&mut subtitles, stream.subtitles);
        } else if mimetype == "application/dash+xml" || ext == "mpd" {
            let manifest = Url::parse(&media_url)?;
            let stream = extract_mpd_fatal(client, &manifest, NAME, &post_id, true)?;
            formats.extend(stream.formats);
            merge_subtitles(&mut subtitles, stream.subtitles);
        } else if mimetype.contains("image") {
            thumbnails.push(json!({
                "id": media_id,
                "url": media_url,
                "width": media.get("width").cloned().unwrap_or(Value::Null),
                "height": media.get("height").cloned().unwrap_or(Value::Null),
                "filesize": media.get("filesize").cloned().unwrap_or(Value::Null),
            }));
        } else {
            let mut fields = vec![
                ("format_id", media_id.clone()),
                ("url", media_url.clone().into()),
                ("width", media.get("width").cloned().unwrap_or(Value::Null)),
                (
                    "height",
                    media.get("height").cloned().unwrap_or(Value::Null),
                ),
                (
                    "filesize",
                    media.get("filesize").cloned().unwrap_or(Value::Null),
                ),
            ];
            fields.push((
                "acodec",
                if media_url.contains("video-card") {
                    "none".into()
                } else {
                    Value::Null
                },
            ));
            formats.push(dict_format(fields, NAME, &post_id)?);
        }
    }

    if !formats.is_empty() {
        fields.push(("formats", Value::Null));
        fields.push(("subtitles", Value::Null));
        fields.push(("thumbnails", Value::Array(thumbnails)));
        if let Some(views) = int_value(video.get("view_count")) {
            fields.push(("view_count", views.into()));
        }
        let mut info = dict_info(&Url::parse(&webpage_url)?, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        return Ok(Post::Video(thumbnail_order(info)));
    }

    if post
        .get("media")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|media| {
            media
                .get("img_url")
                .and_then(Value::as_str)
                .is_some_and(|url| direct::determine_ext(url).as_deref() == Some("gif"))
        })
    {
        return Err(unsupported("GIF posts"));
    }
    let embed = post
        .get("embeds")
        .and_then(Value::as_array)
        .and_then(|embeds| {
            embeds
                .iter()
                .find_map(|embed| embed.get("url").and_then(Value::as_str))
        })
        .filter(|url| !url.is_empty());
    if let Some(embed) = embed {
        return Ok(Post::Embed(embed.to_owned()));
    }
    // No formats: upstream's info dict without `formats`, which processing
    // reports as an error.
    let info = dict_info(&Url::parse(&webpage_url)?, (NAME, NAME), fields, Vec::new());
    Ok(Post::Video(info))
}

/// The thumbnails' `{'id', 'url', 'width', 'height', 'filesize'}` key order
/// (the typed thumbnails would print `url` first).
fn thumbnail_order(mut info: VideoInfo) -> VideoInfo {
    if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
        shape.push_str(";thumbnail!=id,url,width,height,filesize");
    }
    info
}

/// `subtitles.update(...)` per manifest.
fn merge_subtitles(target: &mut crate::Subtitles, source: crate::Subtitles) {
    for (language, formats) in source.entries {
        target.insert(language, formats);
    }
}

/// `playlist_result(entries, id, title, description)`.
fn playlist_result(
    url: &Url,
    (extractor, key): (&'static str, &'static str),
    id: String,
    title: Option<String>,
    description: Option<String>,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut extra = Map::new();
    let mut keys = vec!["id"];
    if title.as_deref().is_some_and(|text| !text.is_empty()) {
        keys.push("title");
    }
    if let Some(description) = description.filter(|text| !text.is_empty()) {
        keys.push("description");
        extra.insert("description".into(), description.into());
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: extractor.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

/// `_entries(endpoint, list_id)`: each page's posts, following the feed's
/// `scrollId` until a page comes back empty.
fn post_entries(client: &Client, endpoint: &str, id: &str) -> Result<Vec<VideoInfo>> {
    let mut items = call_api(client, endpoint, id)?
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let url = Url::parse(&format!("{API_BASE}{endpoint}"))?;
    let mut entries = Vec::new();
    while !items.is_empty() {
        for item in &items {
            let model = item
                .get("action_resource_model")
                .ok_or_else(|| key_error(id, "action_resource_model"))?;
            match parse_post(client, model)? {
                Post::Video(info) => entries.push(info),
                Post::Embed(_) => return Err(unsupported("embedded posts in a feed")),
            }
        }
        let scroll_id = items
            .last()
            .and_then(|item| item.get("scroll_id"))
            .cloned()
            .unwrap_or(Value::Null);
        let body = json!({"scrollDirection": "from", "scrollId": scroll_id}).to_string();
        let page = post_json_note_headers(
            client,
            &url,
            NAME,
            id,
            "Downloading game posts",
            &ACCEPT,
            body,
        )?;
        items = page
            .get("payload")
            .and_then(|payload| payload.get("items"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
    }
    Ok(entries)
}

impl Extractor for GameJolt {
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
        let post_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let payload = call_api(client, &format!("web/posts/view/{post_id}"), &post_id)?;
        let post = payload
            .get("post")
            .ok_or_else(|| key_error(&post_id, "post"))?;
        match parse_post(client, post)? {
            Post::Video(info) => Ok(Some(ExtractResult::Video(info))),
            Post::Embed(target) => {
                // `url_result(embed_url)`, followed.
                let mut follow = direct::build_at(url.clone(), url, String::new());
                follow.formats.clear();
                follow.direct = None;
                follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
                Ok(Some(ExtractResult::Video(follow)))
            }
        }
    }
}

impl Extractor for GameJoltGame {
    fn key(&self) -> &'static str {
        GAME_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = GAME_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let game_id = found["id"].to_owned();
        crate::screen::set_extractor(GAME_NAME);
        crate::screen::screen(&format!(
            "[{GAME_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let game = call_api(client, &format!("web/discover/games/{game_id}"), &game_id)?
            .get("game")
            .cloned()
            .ok_or_else(|| key_error(&game_id, "game"))?;
        let description = content_as_text(&parse_embedded(game.get("description_content")));
        let entries = post_entries(client, &format!("web/posts/fetch/game/{game_id}"), &game_id)?;
        Ok(Some(playlist_result(
            url,
            (GAME_NAME, GAME_NAME),
            game_id,
            game.get("title").and_then(Value::as_str).map(str::to_owned),
            Some(description),
            entries,
        )))
    }
}

impl Extractor for GameJoltGameSoundtrack {
    fn key(&self) -> &'static str {
        SOUNDTRACK_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SOUNDTRACK_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let game_id = found["id"].to_owned();
        crate::screen::set_extractor(SOUNDTRACK_NAME);
        crate::screen::screen(&format!(
            "[{SOUNDTRACK_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let overview = call_api(
            client,
            &format!("web/discover/games/overview/{game_id}"),
            &game_id,
        )?;
        let mut entries = Vec::new();
        for song in overview
            .get("songs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(song_url) = song.get("url").and_then(Value::as_str) else {
                continue;
            };
            let song_url = Url::parse(song_url)?;
            // A url-only entry: its format is the URL itself (`format_id` 0).
            let mut info = direct::with_format(
                direct::build_at(song_url.clone(), &song_url, "mp3".into()),
                "",
            );
            for format in &mut info.formats {
                format.ext = "mp3".into();
            }
            info.ext = "mp3".into();
            info.direct = None;
            info.id = scalar_text(song.get("id"));
            if let Some(title) = song.get("title").and_then(Value::as_str) {
                info.title = title.to_owned();
            }
            crate::metadata_parser::set_field(
                &mut info,
                "release_timestamp",
                scaled_int(song.get("posted_on"), 1000).map_or(Value::Null, Value::from),
            );
            // Upstream's url-only entries are processed as the parent's.
            info.extractor = SOUNDTRACK_NAME.into();
            info.extractor_key = SOUNDTRACK_NAME.into();
            info.webpage_url = url.clone();
            info.extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                "untyped-dict;id,title,url,release_timestamp".into(),
            );
            entries.push(info);
        }
        let title = [
            overview.get("microdata").and_then(|data| data.get("name")),
            overview.get("twitter").and_then(|data| data.get("title")),
            overview.get("fb").and_then(|data| data.get("title")),
        ]
        .into_iter()
        .flatten()
        .find_map(|value| value.as_str().filter(|text| !text.is_empty()))
        .map(str::to_owned);
        Ok(Some(playlist_result(
            url,
            (SOUNDTRACK_NAME, SOUNDTRACK_NAME),
            game_id,
            title,
            None,
            entries,
        )))
    }
}

/// `str_or_none(value)` for an id that may be a number.
fn scalar_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://gamejolt.com/p/introducing-ramses-jackson-some-fnf-himbo-i-ve-been-animating-fo-c6achnzu"
        ));
        assert!(suitable(
            "https://gamejolt.com/p/i-fuckin-broke-chaos-d56h3eue"
        ));
        assert!(!suitable("https://gamejolt.com/@BlazikenSuperStar"));
        assert!(game_suitable(
            "https://gamejolt.com/games/Friday4Fun/655124"
        ));
        assert!(!game_suitable("https://gamejolt.com/games/Friday4Fun"));
        assert!(soundtrack_suitable(
            "https://gamejolt.com/get/soundtrack?foo=bar&game=657899"
        ));
        assert!(!soundtrack_suitable("https://gamejolt.com/get/soundtrack"));
    }
}
