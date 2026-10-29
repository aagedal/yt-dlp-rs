//! Upstream `NZZIE` (`yt_dlp/extractor/nzz.py`): an nzz.ch article's
//! `jw-video-jw*` scripts' `var settings` JW Player data, each parsed with
//! `_parse_jwplayer_data` under the article ID, as a generator playlist.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Nzz;

const NAME: &str = "NZZ";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?nzz\.ch/(?:[^/]+/)*[^/?#]+-ld\.(?P<id>\d+)").unwrap()
});
static SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<script[^>]* data-hid="jw-video-jw[^>]+>(.+?)</script>"#).unwrap()
});
static SETTINGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+settings\s*=[^{]*").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_json(r'var\s+settings\s*=[^{]*', script, 'settings', page_id,
/// fatal=False)`: the settings, else `{}` after a warning.
fn settings(script: &str, page_id: &str) -> Value {
    match super::search_json_detailed(&SETTINGS, script, &str::to_owned) {
        Some(Ok(data)) => data,
        Some(Err(error)) => {
            // `ExtractorError(f'{id}: Failed to parse JSON', cause=e)`.
            let cause = error.map_or_else(String::new, |error| {
                format!(
                    " (caused by JSONDecodeError({}))",
                    crate::batch::py_repr(&error)
                )
            });
            crate::screen::warning(&format!(
                "[{NAME}] {page_id}: Unable to extract settings - Failed to parse JSON: {page_id}: Failed to parse JSON{cause}; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
            Value::Object(serde_json::Map::new())
        }
        None => {
            search_warning(NAME, "settings");
            Value::Object(serde_json::Map::new())
        }
    }
}

/// One `_parse_jwplayer_data(settings, page_id)` entry (`require_title`).
fn entry(client: &Client, url: &Url, data: &Value, page_id: &str) -> Result<Option<VideoInfo>> {
    let Some(object) = data.as_object() else {
        // `_parse_jwplayer_data` returns `[]` (falsy) for non-dicts.
        return Ok(None);
    };
    // A dict without a usable `playlist` is its own (single) item, so an
    // empty one is upstream's `KeyError('title')`.
    let item_missing_title = match object.get("playlist") {
        Some(Value::Array(_)) => false,
        Some(item) if super::lnk::truthy(Some(item)) => item
            .as_object()
            .is_some_and(|item| !item.contains_key("title")),
        _ => !object.contains_key("title"),
    };
    if item_missing_title {
        return Err(Error::Reported(format!(
            "{page_id}: An extractor error has occurred. (caused by KeyError('title')); {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    let no_formats = || {
        Error::Reported(format!(
            "[{NAME}] {page_id}: No video formats found!; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    };
    let Some(entry) = super::jwplayer::parse_required(data, page_id, url)? else {
        if matches!(object.get("playlist"), Some(Value::Array(items)) if items.iter().all(|item| !item.is_object()))
        {
            // No dict items: `[]`, skipped.
            return Ok(None);
        }
        return Err(no_formats());
    };
    let Some((mut info, keys)) =
        super::html5::jwplayer_entry_info(client, entry, url, page_id, NAME)?
    else {
        return Err(no_formats());
    };
    info.webpage_url = url.clone();
    info.extractor = NAME.into();
    info.extractor_key = NAME.into();
    let shape = format!("jwplatform;{}", keys.join(","));
    Ok(Some(crate::info_order::mark_info(info, &shape)))
}

impl Extractor for Nzz {
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
        let page_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &page_id)?;
        // Upstream's `_entries` generator: each script is parsed (and its
        // manifests requested) as the playlist is processed (see
        // `fetch_entries`).
        let scripts: Vec<Value> = SCRIPT
            .captures_iter(&webpage)
            .map(|found| Value::from(&found[1]))
            .collect();
        let state = serde_json::json!({
            "kind": "nzz",
            "url": url.as_str(),
            "page_id": page_id,
            "scripts": scripts,
        });
        let ExtractResult::Playlist(mut playlist) =
            super::nrk::playlist_result(url, NAME, page_id, None, None, Vec::new(), true)
        else {
            unreachable!("playlist_result returns a playlist");
        };
        playlist.pending = Some(crate::PendingEntries { state });
        Ok(Some(ExtractResult::Playlist(playlist)))
    }
}

/// Whether a playlist's pending state is an NZZ article's players.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == "nzz"
}

/// The pending players of an NZZ article, parsed in order until `limit`
/// entries exist.
pub(crate) fn fetch_entries(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let Value::Object(mut state) = state else {
        return Ok(None);
    };
    let url = Url::parse(state["url"].as_str().unwrap_or_default())?;
    let page_id = state["page_id"].as_str().unwrap_or_default().to_owned();
    let mut scripts: std::collections::VecDeque<Value> = match state.shift_remove("scripts") {
        Some(Value::Array(scripts)) => scripts.into(),
        _ => Default::default(),
    };
    while let Some(script) = scripts.pop_front() {
        if limit.is_some_and(|limit| entries.len() >= limit) {
            scripts.push_front(script);
            break;
        }
        let data = settings(script.as_str().unwrap_or_default(), &page_id);
        if let Some(info) = entry(client, &url, &data, &page_id)? {
            entries.push(info);
        }
    }
    if scripts.is_empty() {
        return Ok(None);
    }
    state.insert("scripts".into(), Value::Array(scripts.into()));
    Ok(Some(Value::Object(state)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.nzz.ch/zuerich/gymizyte/gymizyte-schreiben-schueler-heute-noch-diktate-ld.9153"
        ));
        assert!(suitable(
            "https://www.nzz.ch/video/nzz-standpunkte/cvp-auf-der-suche-nach-dem-mass-der-mitte-ld.1368112"
        ));
        assert!(!suitable("https://www.nzz.ch/video"));
    }
}
