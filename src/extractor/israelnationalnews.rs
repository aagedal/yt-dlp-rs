//! Upstream `IsraelNationalNewsIE` (`yt_dlp/extractor/israelnationalnews.py`):
//! the article API's `Content2` embed `src` URLs as a playlist of YouTube
//! results.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct IsraelNationalNews;

const NAME: &str = "IsraelNationalNews";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?israelnationalnews\.com/news/(?P<id>\d+)")
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `traverse_obj(..., ...)`: a dict's values or a list's items.
fn branch(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Object(map)) => map.values().collect(),
        Some(Value::Array(items)) => items.iter().collect(),
        _ => Vec::new(),
    }
}

/// `url_result(src, 'Youtube')`.
fn youtube_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": "Youtube",
            "extractor": "youtube",
            "entry": {"ie_key": "Youtube", "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for IsraelNationalNews {
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
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://www.israelnationalnews.com/Generic/NewAPI/Item?type=0&Item={id}"
        ))?;
        let article = download_json(client, &api, NAME, &id)?;
        // `traverse_obj(article_json, ('Content2', ..., 'content', ...,
        // 'attrs', 'src'))`.
        let mut urls: Vec<String> = Vec::new();
        for block in branch(article.get("Content2")) {
            for part in branch(block.get("content")) {
                match part.get("attrs").and_then(|attrs| attrs.get("src")) {
                    None | Some(Value::Null) => {}
                    Some(Value::String(src)) => urls.push(src.clone()),
                    Some(_) => {
                        return Err(Error::UnsupportedHtml5(
                            "non-string Israel National News embed sources are not supported natively"
                                .into(),
                        ));
                    }
                }
            }
        }
        if urls.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {id}: This article does not have any videos"
            )));
        }
        let mut unique: Vec<String> = Vec::new();
        for src in urls {
            if !unique.contains(&src) {
                unique.push(src);
            }
        }
        let entries = unique
            .iter()
            .map(|target| youtube_entry(target))
            .collect::<Result<Vec<_>>>()?;
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
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
        assert!(suitable("https://www.israelnationalnews.com/news/354520"));
        assert!(suitable("http://israelnationalnews.com/news/1/slug"));
        assert!(!suitable("https://www.israelnationalnews.com/news/abc"));
    }
}
