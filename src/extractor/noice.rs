//! Upstream `NoicePodcastIE` (`yt_dlp/extractor/noice.py`): Noice podcast
//! episodes from the page's Next.js `contentDetails`: its raw content and
//! content URLs (HLS playlists or MP3 files) with the episode, season,
//! catalog and aggregation metadata, falling back to the page's meta tags.
use super::{Extractor, dict_format, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct NoicePodcast;

const NAME: &str = "NoicePodcast";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://open\.noice\.id/content/(?P<id>[a-fA-F0-9-]+)").unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Noice {what} are not supported natively"))
}

/// `_html_search_meta(names, webpage)`: the first name present, else a
/// warning naming the first.
fn search_meta(webpage: &str, names: &[&str]) -> Option<Value> {
    let found = names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name));
    if found.is_none() {
        super::search_warning(NAME, names[0]);
    }
    found.map(Value::from)
}

fn truthy(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| super::nuxt::truthy(value)).cloned()
}

fn parse_iso8601(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map_or(Value::Null, Value::from)
}

fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        Some(Value::Bool(value)) => i64::from(*value).into(),
        _ => Value::Null,
    }
}

impl Extractor for NoicePodcast {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let next_data = super::search_nextjs_data(&webpage, NAME, &display_id)?;
        let mut data = &next_data;
        for key in ["props", "pageProps", "contentDetails"] {
            data = data.get(key).ok_or_else(|| key_error(&display_id, key))?;
        }
        if !data.is_object() {
            return Err(unsupported("content details that are not objects"));
        }
        // `traverse_obj(nextjs_data, (('rawContentUrl', 'url'), ))`.
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        for key in ["rawContentUrl", "url"] {
            let media_url = match data.get(key) {
                None | Some(Value::Null) => continue,
                Some(Value::String(media_url)) => media_url,
                Some(_) => return Err(unsupported("media URLs that are not strings")),
            };
            if super::direct::determine_ext(media_url).as_deref() == Some("m3u8") {
                let manifest = Url::parse(media_url)
                    .map_err(|_| unsupported("playlist URLs that are not URLs"))?;
                let mut stream =
                    extract_m3u8_fatal_master(client, &manifest, NAME, &display_id, None, true)?;
                super::hls::without_ext(&mut stream);
                formats.extend(stream.formats);
                super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            } else {
                formats.push(dict_format(
                    vec![
                        ("url", media_url.clone().into()),
                        ("ext", "mp3".into()),
                        ("vcodec", "none".into()),
                        ("acodec", "mp3".into()),
                    ],
                    NAME,
                    &display_id,
                )?);
            }
        }
        let get = |key: &str| data.get(key);
        let id = truthy(get("id")).unwrap_or_else(|| display_id.clone().into());
        let id = match id {
            Value::String(id) => id,
            Value::Number(id) => id.to_string(),
            _ => return Err(unsupported("IDs that are not strings")),
        };
        let title = truthy(get("title"))
            .or_else(|| search_meta(&webpage, &["og:title"]))
            .unwrap_or(Value::Null);
        let description = truthy(get("description"))
            .or_else(|| {
                get("htmlDescription")
                    .and_then(Value::as_str)
                    .map(crate::html::clean_html)
                    .filter(|text| !text.is_empty())
                    .map(Value::from)
            })
            .or_else(|| search_meta(&webpage, &["description", "og:description"]))
            .unwrap_or(Value::Null);
        let thumbnail = truthy(get("image"))
            .or_else(|| search_meta(&webpage, &["og:image"]))
            .unwrap_or(Value::Null);
        let updated =
            truthy(get("updatedAt")).or_else(|| search_meta(&webpage, &["og:updated_time"]));
        let categories: Vec<Value> = match get("genres") {
            Some(Value::Array(genres)) => genres
                .iter()
                .filter_map(|genre| genre.get("name"))
                .filter(|name| !name.is_null())
                .cloned()
                .collect(),
            Some(Value::Object(genres)) => genres
                .values()
                .filter_map(|genre| genre.get("name"))
                .filter(|name| !name.is_null())
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let channel_id = data
            .pointer("/catalog/id")
            .filter(|value| !value.is_null())
            .or_else(|| get("catalogId"))
            .cloned()
            .unwrap_or(Value::Null);
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", id.into()),
            ("title", title),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("description", description),
            ("thumbnail", thumbnail),
            ("timestamp", parse_iso8601(get("createdAt"))),
            ("release_timestamp", parse_iso8601(get("publishedAt"))),
            ("modified_timestamp", parse_iso8601(updated.as_ref())),
            ("duration", int_or_none(get("duration"))),
            ("categories", Value::Array(categories)),
            ("season", get("seasonName").cloned().unwrap_or(Value::Null)),
            ("season_number", int_or_none(get("seasonNumber"))),
            (
                "channel",
                data.pointer("/catalog/title")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            ("channel_id", channel_id),
        ];
        if let Some(aggregations) = data.pointer("/meta/aggregations") {
            for (key, source) in [
                ("like_count", "likes"),
                ("dislike_count", "dislikes"),
                ("comment_count", "comments"),
                ("channel_follower_count", "followers"),
            ] {
                if let Some(value) = aggregations.get(source).filter(|value| !value.is_null()) {
                    fields.push((key, value.clone()));
                }
            }
        }
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://open.noice.id/content/7694bb04-ff0f-40fa-a60b-5b39f29584b2"
        ));
        assert!(!suitable("https://open.noice.id/catalog/abc"));
    }
}
