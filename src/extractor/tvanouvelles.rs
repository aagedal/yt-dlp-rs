//! Upstream `TVANouvellesIE` and `TVANouvellesArticleIE`
//! (`yt_dlp/extractor/tvanouvelles.py`): a video as a `url` result of the
//! TVA Nouvelles Brightcove player, and an article's `data-video-id` videos
//! as a playlist of `url` results of those video pages.
use super::{Extractor, download_webpage};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TvaNouvelles;
pub struct TvaNouvellesArticle;

const NAME: &str = "TVANouvelles";
const ARTICLE_NAME: &str = "TVANouvellesArticle";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?tvanouvelles\.ca/videos/(?P<id>\d+)").unwrap()
});
static ARTICLE_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?tvanouvelles\.ca/(?:[^/]+/)+(?P<id>[^/?#&]+)").unwrap()
});
static VIDEO_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-video-id=(["'])?(?P<id>\d+)"#).unwrap());

/// `TVANouvellesIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `TVANouvellesArticleIE.suitable`: not a `TVANouvellesIE` URL.
pub fn article_suitable(url: &str) -> bool {
    !suitable(url) && ARTICLE_VALID_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for TvaNouvelles {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let brightcove_id = found["id"].to_owned();
        announce(NAME, url);
        // `url_result(BRIGHTCOVE_URL_TEMPLATE % brightcove_id, 'BrightcoveNew', brightcove_id)`.
        let target = format!(
            "http://players.brightcove.net/1741764581/default_default/index.html?videoId={brightcove_id}"
        );
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

/// `url_result(url, ie='TVANouvelles', video_id=ID)`, resolved when processed.
fn url_entry(target: &str, video_id: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), NAME.into());
    entry.insert("id".into(), video_id.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for TvaNouvellesArticle {
    fn key(&self) -> &'static str {
        ARTICLE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = ARTICLE_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        announce(ARTICLE_NAME, url);
        let webpage = download_webpage(client, url, ARTICLE_NAME, &display_id)?;
        let mut entries = Vec::new();
        for found in VIDEO_ID.captures_iter(&webpage) {
            let id = &found["id"];
            entries.push(url_entry(
                &format!("http://www.tvanouvelles.ca/videos/{id}"),
                id,
            )?);
        }
        // `_og_search_title(webpage, fatal=False)` and
        // `_og_search_description(webpage)` (also not fatal).
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            super::search_warning(ARTICLE_NAME, "OpenGraph title");
        }
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            super::search_warning(ARTICLE_NAME, "OpenGraph description");
        }
        // `playlist_result`: a falsy title and a `None` description are left out.
        let title = title.filter(|title| !title.is_empty());
        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        let mut extra = Map::new();
        if let Some(description) = description {
            keys.push("description");
            extra.insert("description".into(), description.into());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: ARTICLE_NAME.into(),
            extractor_key: ARTICLE_NAME.into(),
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
        assert!(suitable("http://www.tvanouvelles.ca/videos/5117035533001"));
        assert!(!article_suitable(
            "http://www.tvanouvelles.ca/videos/5117035533001"
        ));
        assert!(article_suitable(
            "http://www.tvanouvelles.ca/2016/11/17/des-policiers-qui-ont-la-meche-un-peu-courte"
        ));
        assert!(!article_suitable("http://www.tvanouvelles.ca/"));
    }
}
