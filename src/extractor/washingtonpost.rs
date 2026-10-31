//! Upstream `WashingtonPostIE` and `WashingtonPostArticleIE`
//! (`yt_dlp/extractor/washingtonpost.py`): Washington Post videos as `url`
//! results of their Arc Publishing (`arcpublishing:wapo:UUID`) media, and
//! articles as playlists of their embedded videos.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct WashingtonPost;
pub struct WashingtonPostArticle;

pub(crate) const NAME: &str = "washingtonpost";
pub(crate) const KEY: &str = "WashingtonPost";
const ARTICLE_NAME: &str = "washingtonpost:article";
const ARTICLE_KEY: &str = "WashingtonPostArticle";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?:washingtonpost:|https?://(?:www\.)?washingtonpost\.com/(?:video|posttv)/(?:[^/]+/)*)(?P<id>[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12})",
    )
    .unwrap()
});
static ARTICLE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?washingtonpost\.com/(?:[^/]+/)*(?P<id>[^/?#]+)").unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+\bsrc=["'](?P<url>https?://(?:www\.)?washingtonpost\.com/video/c/embed/[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12})"#,
    )
    .unwrap()
});
static UUIDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:<div\s+class="posttv-video-embed[^>]*?data-uuid=|data-video-uuid=)"([^"]+)""#)
        .unwrap()
});

/// `WashingtonPostIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `WashingtonPostArticleIE.suitable`: not a `WashingtonPostIE` URL.
pub fn article_suitable(url: &str) -> bool {
    !suitable(url) && ARTICLE_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_EMBED_REGEX` iframes under `orderedSet`.
pub(crate) fn embed_urls(_url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Ok(embed) = Url::parse(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Washington Post {what} are not supported natively"))
}

impl Extractor for WashingtonPost {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);
        // `url_result('arcpublishing:wapo:' + video_id, 'ArcPublishing', video_id)`.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_URL.into(),
            format!("arcpublishing:wapo:{video_id}").into(),
        );
        follow.id = video_id;
        Ok(Some(ExtractResult::Video(follow)))
    }
}

/// `url_result(f'washingtonpost:{uuid}', 'WashingtonPost', uuid)` as a
/// playlist entry.
fn video_entry(uuid: &str) -> Result<VideoInfo> {
    let target = format!("washingtonpost:{uuid}");
    let url = Url::parse(&target).map_err(|_| unsupported("video UUIDs that are not URLs"))?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = uuid.to_owned();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": {"ie_key": KEY, "id": uuid, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for WashingtonPostArticle {
    fn key(&self) -> &'static str {
        ARTICLE_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !article_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = ARTICLE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let page_id = found["id"].to_owned();
        start(ARTICLE_NAME, url);
        let webpage = download_webpage(client, url, ARTICLE_NAME, &page_id)?;
        // `_og_search_title(webpage)` is not fatal.
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            crate::screen::warning(&format!(
                "[{ARTICLE_NAME}] unable to extract OpenGraph title; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let mut uuids: Vec<String> = UUIDS
            .captures_iter(&webpage)
            .map(|found| found[1].to_owned())
            .collect();
        if uuids.is_empty() {
            let json_data = super::search_nextjs_data(&webpage, ARTICLE_NAME, &page_id)?;
            // `for content_element in traverse_obj(...)`: a missing list
            // is not iterable.
            let elements =
                match json_data.pointer("/props/pageProps/globalContent/content_elements") {
                    None | Some(Value::Null) => {
                        return Err(Error::Reported("'NoneType' object is not iterable".into()));
                    }
                    Some(Value::Array(elements)) => elements,
                    Some(_) => return Err(unsupported("content elements that are not lists")),
                };
            for element in elements {
                let Value::Object(element) = element else {
                    return Err(unsupported("content elements that are not objects"));
                };
                if element.get("type").and_then(Value::as_str) != Some("video") {
                    continue;
                }
                match element.get("_id") {
                    Some(Value::String(uuid)) => uuids.push(uuid.clone()),
                    _ => return Err(unsupported("video IDs that are not strings")),
                }
            }
        }
        let entries = uuids
            .iter()
            .map(|uuid| video_entry(uuid))
            .collect::<Result<Vec<_>>>()?;
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;;after=id,title".into(),
        );
        if title.is_none() {
            extra.insert("title".into(), Value::Null);
        }
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: page_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: ARTICLE_NAME.into(),
            extractor_key: ARTICLE_KEY.into(),
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
            "https://www.washingtonpost.com/video/c/video/480ba4ee-1ec7-11e6-82c2-a7dcb313287d"
        ));
        assert!(suitable(
            "https://www.washingtonpost.com/video/world/egypt-finds-belongings-debris-from-plane-crash/2016/05/20/480ba4ee-1ec7-11e6-82c2-a7dcb313287d_video.html"
        ));
        assert!(suitable(
            "washingtonpost:480ba4ee-1ec7-11e6-82c2-a7dcb313287d"
        ));
        assert!(!article_suitable(
            "https://www.washingtonpost.com/posttv/world/iraq/2015/02/28/7c57e916-bf86-11e4-9dfb-03366e719af8_video.html"
        ));
        assert!(article_suitable(
            "https://www.washingtonpost.com/nation/2021/08/05/dixie-river-fire-california-climate/"
        ));
        assert_eq!(
            &ARTICLE_URL
                .captures(
                    "http://www.washingtonpost.com/sf/national/2014/03/22/sinkhole-of-bureaucracy/"
                )
                .unwrap()["id"],
            "sinkhole-of-bureaucracy"
        );
    }
}
