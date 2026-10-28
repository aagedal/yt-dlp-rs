//! Upstream `LeFigaroVideoEmbedIE` and `LeFigaroVideoSectionIE`
//! (`yt_dlp/extractor/lefigaro.py`): Le Figaro video embeds as `url`
//! results of their JW Platform media, and video sections as a paged list
//! (20 per page) of embed `url` results from the GraphQL playlist.
use super::{Extractor, download_json_note, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct LeFigaroVideoEmbed;
pub struct LeFigaroVideoSection;

const EMBED_NAME: &str = "LeFigaroVideoEmbed";
const SECTION_NAME: &str = "LeFigaroVideoSection";
const PAGE_SIZE: usize = 20;

static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://video\.lefigaro\.fr/embed/[^?#]+/(?P<id>[\w-]+)").unwrap()
});
static SECTION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://video\.lefigaro\.fr/figaro/(?P<id>[\w-]+)/?(?:[#?]|$)").unwrap()
});

/// `LeFigaroVideoEmbedIE._match_id`.
pub fn embed_suitable(url: &str) -> bool {
    EMBED_URL.is_match(url)
}

/// `LeFigaroVideoSectionIE._match_id`.
pub fn section_suitable(url: &str) -> bool {
    SECTION_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for LeFigaroVideoEmbed {
    fn key(&self) -> &'static str {
        EMBED_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = EMBED_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(EMBED_NAME, url);
        let webpage = download_webpage(client, url, EMBED_NAME, &display_id)?;
        let next_data = super::search_nextjs_data(&webpage, EMBED_NAME, &display_id)?;
        let mut player = &next_data;
        for key in [
            "props",
            "pageProps",
            "initialProps",
            "pageData",
            "playerData",
        ] {
            player = player.get(key).ok_or_else(|| key_error(&display_id, key))?;
        }
        let video_id = match player.get("videoId") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "Le Figaro video IDs that are not strings are not supported natively".into(),
                ));
            }
            None => return Err(key_error(&display_id, "videoId")),
        };
        // `url_result(f'jwplatform:{id}', title=..., ...)`: a `url` result,
        // whose fields processing does not merge.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_URL.into(),
            format!("jwplatform:{video_id}").into(),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

/// `_get_api_response(display_id, page_num, note)`.
fn api_response(client: &Client, display_id: &str, page: usize, note: &str) -> Result<Value> {
    let variables = crate::output::python_json_dumps(&json!({
        "slug": display_id,
        "videosLimit": PAGE_SIZE,
        "sort": "DESC",
        "order": "PUBLISHED_AT",
        "page": page,
    }));
    let api = super::update_url_query(
        &Url::parse("https://api-graphql.lefigaro.fr/graphql")?,
        &[
            (
                "id",
                "flive-website_UpdateListPage_1fb260f996bca2d78960805ac382544186b3225f5bedb43ad08b9b8abef79af6",
            ),
            ("variables", variables.as_str()),
        ],
    );
    download_json_note(client, &api, SECTION_NAME, display_id, note)
}

/// `url_result(video['embedUrl'], LeFigaroVideoEmbedIE, **fields)`.
fn embed_entry(video: &Value, display_id: &str) -> Result<VideoInfo> {
    let target = match video.get("embedUrl") {
        Some(Value::String(url)) => url.clone(),
        Some(_) => {
            return Err(Error::UnsupportedHtml5(
                "Le Figaro embed URLs that are not strings are not supported natively".into(),
            ));
        }
        None => return Err(key_error(display_id, "embedUrl")),
    };
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    // A flat `url` result's ID is its extractor's temporary ID.
    if let Some(found) = EMBED_URL.captures(&target) {
        info.id = found["id"].to_owned();
    }
    let mut entry = Map::new();
    for (key, source) in [
        ("title", "name"),
        ("description", "description"),
        ("thumbnail", "thumbnailUrl"),
    ] {
        if let Some(value) = video.get(source).filter(|value| !value.is_null()) {
            entry.insert(key.into(), value.clone());
        }
    }
    entry.insert("ie_key".into(), EMBED_NAME.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.clone().into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": EMBED_NAME,
            "extractor": EMBED_NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// Whether a playlist's pending state is a Le Figaro section's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == SECTION_NAME
}

/// `InAdvancePagedList(page_func, pages, 20)`.
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let display_id = state["display_id"].as_str().unwrap_or_default().to_owned();
    let page_count = state["page_count"].as_u64().unwrap_or(0) as usize;
    let mut fetch = |page: usize| -> Result<Vec<VideoInfo>> {
        if page >= page_count {
            return Ok(Vec::new());
        }
        let response = api_response(
            client,
            &display_id,
            page + 1,
            &format!("Downloading page {}", page + 1),
        )?;
        let items = response
            .pointer("/data/playlist/jsonLd")
            .ok_or_else(|| key_error(&display_id, "jsonLd"))?;
        let first = match items {
            // An uncaught `IndexError` is reported as its message.
            Value::Array(items) => items
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            _ => return Err(key_error(&display_id, "jsonLd")),
        };
        let videos = match first.get("itemListElement") {
            Some(Value::Array(videos)) => videos,
            _ => return Err(key_error(&display_id, "itemListElement")),
        };
        videos
            .iter()
            .map(|video| embed_entry(video, &display_id))
            .collect()
    };
    let placeholder = || {
        embed_entry(
            &json!({"embedUrl": "https://video.lefigaro.fr/embed/None/None"}),
            &display_id,
        )
    };
    super::paged::fetch_requested(state, limit, entries, PAGE_SIZE, &mut fetch, &placeholder)
}

impl Extractor for LeFigaroVideoSection {
    fn key(&self) -> &'static str {
        SECTION_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SECTION_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(SECTION_NAME, url);
        let response = api_response(client, &display_id, 1, "Downloading JSON metadata")?;
        let initial = response
            .get("data")
            .ok_or_else(|| key_error(&display_id, "data"))?
            .get("playlist")
            .ok_or_else(|| key_error(&display_id, "playlist"))?;
        let video_count = match initial.get("videoCount") {
            Some(Value::Number(number)) => number.as_f64().unwrap_or_default(),
            _ => return Err(key_error(&display_id, "videoCount")),
        };
        let page_count = (video_count / PAGE_SIZE as f64).ceil().max(0.0) as u64;
        let title = match initial.get("title") {
            Some(Value::String(title)) if !title.is_empty() => Some(title.clone()),
            _ => None,
        };
        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;{}{}",
                keys.join(","),
                crate::info_order::GENERATOR
            )
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id.clone(),
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: SECTION_NAME.into(),
            extractor_key: SECTION_NAME.into(),
            nested: Vec::new(),
            pending: Some(crate::PendingEntries {
                state: json!({
                    "kind": SECTION_NAME,
                    "display_id": display_id,
                    "page_count": page_count,
                    "random_access": true,
                }),
            }),
            entries: Vec::new(),
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(embed_suitable(
            "https://video.lefigaro.fr/embed/figaro/video/les-francais-ne-veulent-ils-plus-travailler/"
        ));
        assert!(section_suitable("https://video.lefigaro.fr/figaro/factu/"));
        assert!(!section_suitable(
            "https://video.lefigaro.fr/figaro/video/x/"
        ));
    }
}
