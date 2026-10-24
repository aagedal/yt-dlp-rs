//! Upstream `NetzkinoIE` (`yt_dlp/extractor/netzkino.py`): the details page's
//! `__NEXT_DATA__` dehydrated `CmsMovie` query (DRM licences refused) and its
//! PMD video URL as an untyped result with the movie's metadata.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Netzkino;

const NAME: &str = "Netzkino";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?netzkino\.de/details/(?P<id>[^/?#]+)").unwrap()
});
static ABSOLUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[a-zA-Z][a-zA-Z0-9+.-]*:)?//").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Iterate a JSON list's items or a dict's values (`...`).
fn items(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(map)) => map.values().collect(),
        _ => Vec::new(),
    }
}

/// `(KEY, 'nodes', ..., INNER, LEAF, {clean_html}, filter)`.
fn node_texts(movie: &Map<String, Value>, key: &str, inner: &str, leaf: &str) -> Vec<Value> {
    items(movie.get(key).and_then(|value| value.get("nodes")))
        .into_iter()
        .filter_map(|node| node.get(inner)?.get(leaf)?.as_str())
        .map(crate::html::clean_html)
        .filter(|text| !text.is_empty())
        .map(Value::from)
        .collect()
}

/// `int_or_none` (`None` when absent).
fn int_value(value: Option<&Value>) -> Option<Value> {
    Some(super::streamable::int_or_none(value)).filter(|value| !value.is_null())
}

/// `urljoin('https://pmd.netzkino-seite.netzkino.de/', path)`.
fn pmd_url(path: Option<&Value>) -> Option<String> {
    let path = path?.as_str().filter(|path| !path.is_empty())?;
    if ABSOLUTE.is_match(path) {
        return Some(path.to_owned());
    }
    Url::parse("https://pmd.netzkino-seite.netzkino.de/")
        .ok()?
        .join(path)
        .ok()
        .map(String::from)
}

impl Extractor for Netzkino {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let next_js_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        // `('props', '__dehydratedState', 'queries', ..., 'state', 'data',
        // 'data', lambda _, v: v['__typename'] == 'CmsMovie', any)`.
        let queries = next_js_data
            .get("props")
            .and_then(|props| props.get("__dehydratedState"))
            .and_then(|state| state.get("queries"));
        let empty = Map::new();
        let movie = items(queries)
            .into_iter()
            .filter_map(|query| query.get("state")?.get("data")?.get("data"))
            .flat_map(|data| items(Some(data)))
            .find(|value| value.get("__typename").and_then(Value::as_str) == Some("CmsMovie"));
        let movie = match movie {
            None => &empty,
            Some(Value::Object(movie)) => movie,
            Some(_) => unreachable!("a value with a key is an object"),
        };
        let drm = items(movie.get("licenses").and_then(|value| value.get("nodes")))
            .into_iter()
            .filter_map(|node| node.get("properties")?.as_str())
            .any(|property| property == "DRM");
        if drm {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: This video is DRM protected"
            )));
        }
        let title = movie
            .get("originalTitle")
            .and_then(Value::as_str)
            .map(crate::html::clean_html);
        let mut fields: Vec<(&str, Value)> = Vec::new();
        if let Some(age_limit) = int_value(movie.get("fskRating")) {
            fields.push(("age_limit", age_limit));
        }
        if let Some(alt_title) = title.clone().filter(|title| !title.is_empty()) {
            fields.push(("alt_title", alt_title.into()));
        }
        let cast = node_texts(movie, "cast", "person", "name");
        if !cast.is_empty() {
            fields.push(("cast", Value::Array(cast)));
        }
        let mut creators = node_texts(movie, "directors", "person", "name");
        creators.extend(node_texts(movie, "writers", "person", "name"));
        if !creators.is_empty() {
            fields.push(("creators", Value::Array(creators)));
        }
        let categories = node_texts(movie, "categories", "category", "title");
        if !categories.is_empty() {
            fields.push(("categories", Value::Array(categories)));
        }
        let description = movie
            .get("longSynopsis")
            .and_then(Value::as_str)
            .map(crate::html::clean_html)
            .filter(|text| !text.is_empty());
        if let Some(description) = description {
            fields.push(("description", description.into()));
        }
        if let Some(duration) = int_value(movie.get("runtimeInSeconds")) {
            fields.push(("duration", duration));
        }
        let location = movie
            .get("productionCountry")
            .and_then(Value::as_str)
            .map(crate::html::clean_html)
            .filter(|text| !text.is_empty());
        if let Some(location) = location {
            fields.push(("location", location.into()));
        }
        if let Some(year) = int_value(movie.get("productionYear")) {
            fields.push(("release_year", year));
        }
        let thumbnail = movie
            .get("coverImage")
            .and_then(|image| image.get("masterUrl"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let media = pmd_url(
            movie
                .get("videoSource")
                .and_then(|source| source.get("pmdUrl")),
        );
        let mut keys = vec!["id"];
        if title.is_some() {
            keys.push("title");
        }
        let mut info = match &media {
            Some(media) => {
                let media = Url::parse(media)?;
                let ext = super::direct::determine_ext(media.as_str())
                    .unwrap_or_else(|| "unknown_video".into());
                let mut info = super::direct::with_format(
                    super::direct::build_at(media.clone(), &media, ext.clone()),
                    "",
                );
                for format in &mut info.formats {
                    format.ext = ext.clone();
                    format.format_id = String::new();
                }
                info.format_id = None;
                info
            }
            None => {
                let mut info = super::direct::build_at(url.clone(), url, String::new());
                info.formats.clear();
                info.extra
                    .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
                info
            }
        };
        info.direct = None;
        info.id = video_id;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        match title {
            Some(title) => info.title = title,
            None => {
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        for (key, value) in fields {
            crate::metadata_parser::set_field(&mut info, key, value);
            keys.push(key);
        }
        if let Some(thumbnail) = thumbnail {
            info.metadata.thumbnail = Some(thumbnail);
            keys.push("thumbnail");
        }
        if media.is_some() {
            keys.push("url");
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!(
                "untyped-dict;{}{}",
                keys.join(","),
                if keys.contains(&"title") {
                    ""
                } else {
                    "-untitled"
                }
            ),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.netzkino.de/details/snow-beast"));
        assert!(suitable("https://netzkino.de/details/x?y=1"));
        assert!(!suitable("https://www.netzkino.de/kategorie/horror"));
    }
}
