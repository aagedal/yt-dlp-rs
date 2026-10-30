//! Upstream `SovietsClosetIE` and `SovietsClosetPlaylistIE`
//! (`yt_dlp/extractor/sovietscloset.py`): a stream's Nuxt `payload.js`
//! (its MediaDelivery `bunnyId` is handed to BunnyCdn as a
//! `url_transparent` result), and a game or category page's streams as
//! `url` entries from the global payload.
use super::nuxt::{Step, search_nuxt_data};
use super::{Extractor, download_webpage, download_webpage_note};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct SovietsCloset;
pub struct SovietsClosetPlaylist;

const NAME: &str = "SovietsCloset";
const PLAYLIST_NAME: &str = "SovietsClosetPlaylist";
const MEDIADELIVERY_REFERER: &str = "https://iframe.mediadelivery.net/";

static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?sovietscloset\.com/video/(?P<id>[0-9]+)/?").unwrap()
});
// Upstream's `(?!video)` lookahead, applied to the captured path below.
static PLAYLIST_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?sovietscloset\.com/(?P<id>[^#?]+)").unwrap());
static STATIC_BASE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(/_nuxt/static/\d+)").unwrap());

/// `_match_valid_url` for the video IE.
pub fn suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

/// `_match_valid_url` for the playlist IE (`(?!video)`).
fn playlist_path(url: &str) -> Option<String> {
    let id = PLAYLIST_URL.captures(url)?["id"].to_owned();
    (!id.starts_with("video")).then_some(id)
}

/// `_search_regex(r'(/_nuxt/static/\d+)', webpage, 'staticAssetsBase')`.
fn static_assets_base(webpage: &str, id: &str) -> Result<String> {
    let base = STATIC_BASE
        .captures(webpage)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract staticAssetsBase; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    Ok(format!("https://sovietscloset.com{base}"))
}

/// `parse_nuxt_jsonp(nuxt_jsonp_url, video_id, name)`.
fn parse_nuxt_jsonp(
    client: &Client,
    extractor: &'static str,
    url: &str,
    id: &str,
    name: &str,
) -> Result<Value> {
    let url = Url::parse(url)?;
    let nuxt_jsonp = download_webpage_note(
        client,
        &url,
        extractor,
        id,
        &format!("Downloading {name} __NUXT_JSONP__"),
    )?;
    // `_search_nuxt_data`'s default `traverse=('data', 0)`.
    search_nuxt_data(
        &nuxt_jsonp,
        extractor,
        Some(id),
        "__NUXT_JSONP__",
        true,
        &[Step::Key("data"), Step::Index(0)],
    )
}

/// `value[key]` with upstream's `KeyError` text.
fn required<'a>(value: &'a Value, id: &str, key: &str) -> Result<&'a Value> {
    value
        .get(key)
        .ok_or_else(|| super::clyp::key_error(id, key))
}

/// `try_get(stream, lambda x: x['subcategory']['name'], str)`.
fn subcategory_name(stream: &Value) -> Option<String> {
    stream
        .get("subcategory")
        .and_then(|subcategory| subcategory.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Python's `str()` of a JSON scalar, for the episode title.
fn python_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `SovietsClosetBaseIE.video_meta`: the fields of one stream, in order.
fn video_meta(
    video_id: &str,
    game_name: &str,
    category_name: Option<&str>,
    episode_number: Value,
    stream_date: Option<&str>,
) -> Vec<(&'static str, Value)> {
    let mut title = game_name.to_owned();
    if let Some(category) = category_name.filter(|category| *category != "Misc") {
        title.push_str(&format!(" - {category}"));
    }
    if super::nuxt::truthy(&episode_number) {
        title.push_str(&format!(" #{}", python_str(&episode_number)));
    }
    let timestamp = stream_date
        .and_then(|date| crate::dates::unified_timestamp(date, false))
        .map_or(Value::Null, Value::from);
    vec![
        ("id", video_id.into()),
        ("title", title.into()),
        ("http_headers", json!({"Referer": MEDIADELIVERY_REFERER})),
        ("uploader", "SovietWomble".into()),
        ("creator", "SovietWomble".into()),
        ("release_timestamp", timestamp.clone()),
        ("timestamp", timestamp),
        ("uploader_id", "SovietWomble".into()),
        ("uploader_url", "https://www.twitch.tv/SovietWomble".into()),
        ("was_live", true.into()),
        ("availability", "public".into()),
        ("series", game_name.into()),
        ("season", category_name.map_or(Value::Null, Value::from)),
        ("episode_number", episode_number),
    ]
}

/// `url_result(url, ie=...)` as a playlist entry carrying its fields (the
/// flat listing prints them; a non-transparent `url` result does not merge
/// them into the followed video).
fn url_entry(target: &str, ie_key: &'static str, fields: Vec<(&str, Value)>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ie_key.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    for (key, value) in fields {
        entry.insert(key.to_owned(), value);
    }
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": ie_key,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for SovietsCloset {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let base = static_assets_base(&webpage, &video_id)?;
        let nuxt = parse_nuxt_jsonp(
            client,
            NAME,
            &format!("{base}/video/{video_id}/payload.js"),
            &video_id,
            "video",
        )?;
        let stream = required(&nuxt, &video_id, "stream")?;
        let game = required(stream, &video_id, "game")?;
        let game_name = required(game, &video_id, "name")?
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let category_name = subcategory_name(stream);
        let stream_date = stream
            .get("date")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let bunny_id = required(stream, &video_id, "bunnyId")?
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let mut fields = video_meta(
            &video_id,
            &game_name,
            category_name.as_deref(),
            stream.get("number").cloned().unwrap_or(Value::Null),
            stream_date.as_deref(),
        );
        fields.push((
            "_old_archive_ids",
            json!([format!("sovietscloset {video_id}")]),
        ));

        // `url_result(embed, ie=BunnyCdnIE, url_transparent=True, **meta)`.
        let target = format!("https://iframe.mediadelivery.net/embed/5105/{bunny_id}");
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        let fields: Map<String, Value> = fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

impl Extractor for SovietsClosetPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(playlist_id) = playlist_path(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = playlist_id
            .strip_suffix('/')
            .unwrap_or(&playlist_id)
            .to_owned();
        crate::screen::set_extractor(PLAYLIST_NAME);
        crate::screen::screen(&format!(
            "[{PLAYLIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, PLAYLIST_NAME, &playlist_id)?;
        let base = static_assets_base(&webpage, &playlist_id)?;
        let global = parse_nuxt_jsonp(
            client,
            PLAYLIST_NAME,
            &format!("{base}/payload.js"),
            &playlist_id,
            "global",
        )?;
        let games = required(&global, &playlist_id, "games")?
            .as_array()
            .cloned()
            .unwrap_or_default();

        let (game_slug, category_slug) = match playlist_id.split_once('/') {
            Some((game, category)) if !category.contains('/') => {
                (game.to_lowercase(), category.to_lowercase())
            }
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "SovietsCloset paths with more than one category level are not supported natively"
                        .into(),
                ));
            }
            None => (playlist_id.to_lowercase(), "misc".to_owned()),
        };
        let slug_of = |value: &Value| {
            value
                .get("slug")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase()
        };
        let game = games
            .iter()
            .find(|game| slug_of(game) == game_slug)
            .ok_or_else(|| {
                Error::UnsupportedHtml5(format!(
                    "SovietsCloset game {game_slug:?} not found in the site's game list"
                ))
            })?;
        let subcategories = game
            .get("subcategories")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let category = subcategories
            .iter()
            .find(|category| {
                category
                    .get("slug")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_lowercase()
                    == category_slug
            })
            .or_else(|| subcategories.first())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "SovietsCloset games without subcategories are not supported natively".into(),
                )
            })?;
        let category_slug = match slug_of(category) {
            slug if slug.is_empty() => category_slug,
            slug => slug,
        };
        let game_name = game.get("name").and_then(Value::as_str).unwrap_or_default();
        let mut playlist_title = if game_name.is_empty() {
            game_slug.clone()
        } else {
            game_name.to_owned()
        };
        if category_slug != "misc" {
            let category_name = category
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map_or(category_slug.as_str(), |name| name);
            playlist_title.push_str(&format!(" - {category_name}"));
        }

        let streams = category
            .get("streams")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut entries = Vec::new();
        for (index, stream) in streams.iter().enumerate() {
            let stream_id = required(stream, &playlist_id, "id")?;
            let stream_id = python_str(stream_id);
            let target = format!("https://sovietscloset.com/video/{stream_id}");
            let fields = video_meta(
                &stream_id,
                game_name,
                category.get("name").and_then(Value::as_str),
                Value::from(index as i64 + 1),
                stream.get("date").and_then(Value::as_str),
            );
            entries.push(url_entry(&target, NAME, fields)?);
        }

        Ok(Some(super::nrk::playlist_result(
            url,
            PLAYLIST_NAME,
            playlist_id,
            Some(playlist_title),
            None,
            entries,
            false,
        )))
    }
}

/// URL matching for `--list-extractors`.
pub fn playlist_suitable(url: &str) -> bool {
    playlist_path(url).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://sovietscloset.com/video/1337"));
        assert!(suitable("https://sovietscloset.com/video/1337/"));
        assert!(!suitable("https://sovietscloset.com/The-Witcher"));
        assert_eq!(
            playlist_path("https://sovietscloset.com/Arma-3/Zeus-Games").as_deref(),
            Some("Arma-3/Zeus-Games")
        );
        assert_eq!(playlist_path("https://sovietscloset.com/video/1337"), None);
    }

    #[test]
    fn video_titles_follow_category_and_episode() {
        let fields = video_meta("13", "The Witcher", Some("Misc"), json!(13), None);
        assert_eq!(fields[1], ("title", "The Witcher #13".into()));
        let fields = video_meta("5", "Arma 3", Some("Zeus Games"), json!(5), None);
        assert_eq!(fields[1], ("title", "Arma 3 - Zeus Games #5".into()));
    }

    #[test]
    fn bunny_handoff_is_a_media_delivery_embed() {
        assert!(super::super::bunnycdn::suitable(
            "https://iframe.mediadelivery.net/embed/5105/2f0cfbf4-3588-43a9-a7d6-7c9ea3755e67"
        ));
    }
}
