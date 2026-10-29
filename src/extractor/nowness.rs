//! Upstream `NownessIE`, `NownessPlaylistIE` and `NownessSeriesIE`
//! (`yt_dlp/extractor/nowness.py`): the story, playlist and series APIs. Each
//! post's first usable video source is a Brightcove player (found in the
//! `nowness.com/iframe` page; `BrightcoveNew` URL), a Vimeo or a YouTube
//! video; `cinematique` sources are skipped. Brightcove Legacy players are
//! explicit unsupported errors.
use super::{Extractor, clyp::key_error, download_json_headers, download_webpage_note, follow_url};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Nowness;
pub struct NownessPlaylist;
pub struct NownessSeries;

const NAME: &str = "nowness";
const PLAYLIST_NAME: &str = "nowness:playlist";
const SERIES_NAME: &str = "nowness:series";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|cn)\.)?nowness\.com/(?:story|(?:series|category)/[^/]+)/(?P<id>[^/]+?)(?:$|[?#])").unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|cn)\.)?nowness\.com/playlist/(?P<id>\d+)").unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|cn)\.)?nowness\.com/series/(?P<id>[^/]+?)(?:$|[?#])").unwrap()
});
// `BrightcoveLegacyIE._extract_brightcove_urls`' embed patterns: a Legacy
// player URL is an unsupported Brightcove player.
static LEGACY_META: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<meta\s+(?:property|itemprop)=(?:'(?:og:video|embedURL)'|"(?:og:video|embedURL)")[^>]+content=['"](https?://(?:secure|c)\.brightcove\.com/[^'"]+)"#,
    )
    .unwrap()
});
static LEGACY_OBJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)<object\s*(?:[^>]+?class=['"][^>]*?BrightcoveExperience.*?['"]|[^>]*?>\s*<param\s+name="movie"\s+value="https?://[^/]*brightcove\.com/).+?>\s*</object>"#,
    )
    .unwrap()
});
static LEGACY_CUSTOM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"customBC\.createVideo\(").unwrap());
static LEGACY_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<iframe[^>]+src=(['"])(?:https?:)?//link\.brightcove\.com/services/player/"#)
        .unwrap()
});

/// `_match_id` of the story pattern.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id` of the playlist pattern.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `_match_id` of the series pattern.
pub fn series_suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Nowness {what} are not supported natively"))
}

/// `_extract_brightcove_urls` for `BrightcoveLegacyIE` (only whether a Legacy
/// player is embedded; a Legacy player is never extracted natively).
fn legacy_player_present(webpage: &str) -> bool {
    LEGACY_META.captures_iter(webpage).any(|found| {
        let url = &found[1];
        url.contains("playerKey") || url.contains("videoId") || url.contains("idVideo")
    }) || LEGACY_OBJECT.is_match(webpage)
        || LEGACY_CUSTOM.is_match(webpage)
        || LEGACY_LINK.is_match(webpage)
}

/// A Python value as text (f-string formatting of an ID).
fn id_text(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        _ => Err(unsupported("video IDs that are not strings or numbers")),
    }
}

/// `video_id`'s `media['source']`-named entry, the URL and the extractor
/// that `url_result` names.
struct Target {
    url: String,
    ie_key: &'static str,
    extractor: &'static str,
}

/// `NownessBaseIE._extract_url_result(post)`: the first video source of a
/// post that yields a URL (`None` when no source does).
fn post_target(
    client: &Client,
    ie: &'static str,
    post: &Value,
    id: &str,
) -> Result<Option<Target>> {
    let post_type = post.get("type").ok_or_else(|| key_error(id, "type"))?;
    if post_type.as_str() != Some("video") {
        return Ok(None);
    }
    let media = post.get("media").ok_or_else(|| key_error(id, "media"))?;
    for item in media.as_array().into_iter().flatten() {
        if item.get("type").and_then(Value::as_str) != Some("video") {
            continue;
        }
        let video_id = item
            .get("content")
            .ok_or_else(|| key_error(id, "content"))?;
        let source = item
            .get("source")
            .ok_or_else(|| key_error(id, "source"))?
            .as_str()
            .unwrap_or_default();
        match source {
            "brightcove" => {
                let video_id = id_text(video_id)?;
                let iframe = Url::parse(&format!("http://www.nowness.com/iframe?id={video_id}"))?;
                let player = download_webpage_note(
                    client,
                    &iframe,
                    NAME,
                    &video_id,
                    "Downloading player JavaScript",
                )?;
                if legacy_player_present(&player) {
                    return Err(unsupported("Brightcove Legacy players"));
                }
                let Some(url) = super::brightcove::brightcove_urls(&player)
                    .into_iter()
                    .next()
                else {
                    return Err(Error::Reported(format!(
                        "[{ie}] {id}: Could not find player definition; {}",
                        crate::BUG_REPORTS_MESSAGE
                    )));
                };
                return Ok(Some(Target {
                    url,
                    ie_key: "BrightcoveNew",
                    extractor: "brightcove:new",
                }));
            }
            "vimeo" => {
                return Ok(Some(Target {
                    url: format!("http://vimeo.com/{}", id_text(video_id)?),
                    ie_key: "Vimeo",
                    extractor: "vimeo",
                }));
            }
            "youtube" => {
                return Ok(Some(Target {
                    url: id_text(video_id)?,
                    ie_key: "Youtube",
                    extractor: "youtube",
                }));
            }
            // `pass` for `cinematique` (and any other source): the next media.
            _ => {}
        }
    }
    Ok(None)
}

/// `url_result(target, ie)` as a playlist entry, resolved when processed.
/// A bare YouTube ID resolves as its watch page.
fn url_entry(target: &Target) -> Result<VideoInfo> {
    let url = Url::parse(&target.url)
        .or_else(|_| Url::parse(&format!("https://www.youtube.com/watch?v={}", target.url)))?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), target.ie_key.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.url.clone().into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": url.as_str(),
            "force_videoid": null,
            "fields": {},
            "ie_key": target.ie_key,
            "extractor": target.extractor,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `_api_request(url, request_path)`'s JSON with the language header.
fn api_json(client: &Client, url: &str, ie: &'static str, id: &str, path: &str) -> Result<Value> {
    let language = if url.contains("cn.nowness.com") {
        "zh-cn"
    } else {
        "en-us"
    };
    let api = Url::parse(&format!("http://api.nowness.com/api/{path}"))?;
    download_json_headers(client, &api, ie, id, &[("X-Nowness-Language", language)])
}

/// The Nowness playlist/series entries: `url_entry`s of the posts that yield a URL.
fn post_entries<'a>(
    client: &Client,
    ie: &'static str,
    posts: impl Iterator<Item = &'a Value>,
    id: &str,
) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    for post in posts {
        if let Some(target) = post_target(client, ie, post, id)? {
            entries.push(url_entry(&target)?);
        }
    }
    Ok(entries)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for Nowness {
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
        let id = found["id"].to_owned();
        announce(NAME, url);
        let post = api_json(
            client,
            url.as_str(),
            NAME,
            &id,
            &format!("post/getBySlug/{id}"),
        )?;
        let Some(target) = post_target(client, NAME, &post, &id)? else {
            return Err(unsupported("posts without a supported video source"));
        };
        Ok(Some(follow_url(url, target.url)))
    }
}

impl Extractor for NownessPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        announce(PLAYLIST_NAME, url);
        let playlist = api_json(
            client,
            url.as_str(),
            PLAYLIST_NAME,
            &playlist_id,
            &format!("post?PlaylistId={playlist_id}"),
        )?;
        let items = playlist
            .get("items")
            .ok_or_else(|| key_error(&playlist_id, "items"))?;
        let entries = post_entries(
            client,
            PLAYLIST_NAME,
            items.as_array().into_iter().flatten(),
            &playlist_id,
        )?;
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: "NownessPlaylist".into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

impl Extractor for NownessSeries {
    fn key(&self) -> &'static str {
        SERIES_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let series_slug = found["id"].to_owned();
        announce(SERIES_NAME, url);
        let series = api_json(
            client,
            url.as_str(),
            SERIES_NAME,
            &series_slug,
            &format!("series/getBySlug/{series_slug}"),
        )?;
        let series_id = match series.get("id") {
            Some(Value::Number(number)) => number.to_string(),
            Some(Value::String(text)) => text.clone(),
            _ => return Err(key_error(&series_slug, "id")),
        };
        let posts = series
            .get("posts")
            .ok_or_else(|| key_error(&series_id, "posts"))?;
        let entries = post_entries(
            client,
            SERIES_NAME,
            posts.as_array().into_iter().flatten(),
            &series_id,
        )?;

        // `translations[0].get('title') or translations[0]['seoTitle']`.
        let mut title = None;
        let mut description = None;
        if let Some(first) = series
            .get("translations")
            .and_then(Value::as_array)
            .and_then(|translations| translations.first())
        {
            title = first
                .get("title")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
            if title.is_none() {
                title = Some(
                    first
                        .get("seoTitle")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .ok_or_else(|| key_error(&series_id, "seoTitle"))?,
                );
            }
            description = first
                .get("seoDescription")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        // `playlist_result(entries, id, title, description)`.
        let mut keys = vec!["id"];
        let mut extra = Map::new();
        if title.as_deref().is_some_and(|title| !title.is_empty()) {
            keys.push("title");
        }
        if let Some(description) = &description {
            keys.push("description");
            extra.insert("description".into(), description.clone().into());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: series_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: SERIES_NAME.into(),
            extractor_key: "NownessSeries".into(),
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
    fn urls_follow_upstream_valid_urls() {
        assert!(suitable(
            "https://www.nowness.com/story/candor-the-art-of-gesticulation"
        ));
        assert!(suitable(
            "https://cn.nowness.com/story/kasper-bjorke-ft-jaakko-eino-kalevi-tnr"
        ));
        assert!(suitable(
            "https://www.nowness.com/series/nowness-picks/jean-luc-godard-supercut"
        ));
        assert!(!suitable("https://www.nowness.com/series/60-seconds"));
        assert!(playlist_suitable(
            "https://www.nowness.com/playlist/3286/i-guess-thats-why-they-call-it-the-blues"
        ));
        assert!(series_suitable("https://www.nowness.com/series/60-seconds"));
        assert!(!series_suitable("https://www.nowness.com/"));
    }
}
