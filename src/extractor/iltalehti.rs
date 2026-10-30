//! Upstream `IltalehtiIE` (`yt_dlp/extractor/iltalehti.py`): the article's
//! `window.App` state (through `js_to_json`), its main media and body
//! `jwplayer` items as a playlist of `jwplatform:ID` results.
use super::{Extractor, download_webpage, search_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Iltalehti;

const NAME: &str = "Iltalehti";

// Upstream's ID group is a single character.
static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?iltalehti\.fi/[^/?#]+/a/(?P<id>[^/?#])")
        .unwrap()
});

static APP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<script>\s*window.App\s*=").unwrap());

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

/// `url_result('jwplatform:ID', 'JWPlatform')`.
pub(crate) fn jwplatform_entry(target: &str) -> Result<VideoInfo> {
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
            "ie_key": "JWPlatform",
            "extractor": "jwplatform",
            "entry": {"ie_key": "JWPlatform", "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for Iltalehti {
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
        let article_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &article_id)?;
        let info = match search_json(&APP, &webpage, true) {
            Some(Some(info)) => info,
            Some(None) => {
                return Err(Error::UnsupportedHtml5(
                    "Iltalehti app state that does not parse is not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {article_id}: Unable to extract json; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let items: Vec<&Value> = branch(info.pointer("/state/articles"))
            .into_iter()
            .filter_map(|article| article.get("items"))
            .collect();
        let mut props: Vec<&Value> = Vec::new();
        for item in &items {
            if let Some(main) = item
                .get("main_media")
                .and_then(|main| main.get("properties"))
                .filter(|value| !value.is_null())
            {
                props.push(main);
            }
            for part in branch(item.get("body")) {
                if let Some(properties) = part.get("properties").filter(|value| !value.is_null()) {
                    props.push(properties);
                }
            }
        }
        let mut targets: Vec<String> = Vec::new();
        for prop in props {
            if prop.get("provider").and_then(Value::as_str) != Some("jwplayer") {
                continue;
            }
            let video_id = match prop.get("id") {
                None | Some(Value::Null) => continue,
                Some(Value::String(text)) => text.clone(),
                Some(Value::Number(number)) => number.to_string(),
                Some(_) => {
                    return Err(Error::UnsupportedHtml5(
                        "non-string Iltalehti video IDs are not supported natively".into(),
                    ));
                }
            };
            let target = format!("jwplatform:{video_id}");
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        let entries = targets
            .iter()
            .map(|target| jwplatform_entry(target))
            .collect::<Result<Vec<_>>>()?;
        let title = items
            .iter()
            .filter_map(|item| item.get("canonical_title"))
            .find(|value| !value.is_null())
            .cloned();
        // `playlist_result(entries, article_id, title=...)`: the keyword
        // title (even `None`) before the ID.
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;title,id".into(),
        );
        let title = match title {
            Some(Value::String(text)) => text,
            None => {
                extra.insert("title".into(), Value::Null);
                String::new()
            }
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "non-string Iltalehti titles are not supported natively".into(),
                ));
            }
        };
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: article_id,
            title,
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
        assert!(suitable(
            "https://www.iltalehti.fi/ulkomaat/a/9fbd067f-94e4-46cd-8748-9d958eb4dae2"
        ));
        assert!(suitable("https://iltalehti.fi/politiikka/a/1"));
        assert!(!suitable("https://www.iltalehti.fi/ulkomaat/"));
    }
}
