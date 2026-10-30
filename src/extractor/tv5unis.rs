//! Upstream `TV5UnisVideoIE` and `TV5UnisIE` (`yt_dlp/extractor/tv5unis.py`):
//! the `api.tv5unis.ca` GraphQL product as an HLS result (with its
//! `limelightmedia` archive ID), geo-restricted to Canada.
use super::{Extractor, dict_info, download_json, extract_m3u8_fatal_master, update_url_query};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TV5UnisVideo;
pub struct TV5Unis;

const API: &str = "https://api.tv5unis.ca/graphql";
const GEO_COUNTRIES: [&str; 1] = ["CA"];

static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?tv5unis\.ca/videos/[^/?#]+/(?P<id>\d+)").unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?tv5unis\.ca/videos/(?P<id>[^/?#]+)(?:/saisons/(?P<season_number>\d+)/episodes/(?P<episode_number>\d+))?/?(?:[?#&]|$)",
    )
    .unwrap()
});

/// `value[key]`'s escaped `KeyError`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// Upstream `_real_extract` of `TV5UnisBaseIE`: the GraphQL query for one
/// product (`query_name` with `args`), then its HLS formats and metadata.
fn extract_product(
    client: &Client,
    page: &Url,
    key: &'static str,
    name: &'static str,
    query_name: &str,
    args: &str,
    id: &str,
) -> Result<VideoInfo> {
    let query = format!(
        "{{\n  {query_name}({args}) {{\n    title\n    summary\n    tags\n    duration\n    seasonNumber\n    episodeNumber\n    collection {{\n      title\n    }}\n    rating {{\n      name\n    }}\n    videoElement {{\n      __typename\n      ... on Video {{\n        mediaId\n        encodings {{\n          hls {{\n            url\n          }}\n        }}\n      }}\n      ... on RestrictedVideo {{\n        code\n        reason\n      }}\n    }}\n  }}\n}}"
    );
    let api = update_url_query(&Url::parse(API)?, &[("query", &query)]);
    let response = download_json(client, &api, name, id)?;
    let product = response
        .get("data")
        .and_then(|data| data.get(query_name))
        .ok_or_else(|| key_error(id, query_name))?;
    let video = product.get("videoElement").unwrap_or(&Value::Null);
    if video.is_null() {
        return Err(Error::Reported(format!(
            "[{name}] {id}: This content is no longer available"
        )));
    }

    if video.get("__typename").and_then(Value::as_str) == Some("RestrictedVideo") {
        let code = video.get("code").filter(|code| !code.is_null());
        if code.and_then(Value::as_i64) == Some(1001) {
            return Err(crate::geo::restricted(
                &format!(
                    "[{name}] {id}: This video is not available from your location due to geo restriction"
                ),
                &GEO_COUNTRIES,
            ));
        }
        let mut message = String::from("This video is restricted");
        if let Some(code) = code {
            message.push_str(&format!(", error code {code}"));
        }
        if let Some(reason) = video
            .get("reason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty())
        {
            message.push_str(&format!(": {}", reason.strip_suffix('.').unwrap_or(reason)));
        }
        return Err(Error::Reported(format!("[{name}] {id}: {message}")));
    }

    let media_id = video
        .get("mediaId")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(id, "mediaId"))?
        .to_owned();
    let hls = video
        .get("encodings")
        .and_then(|encodings| encodings.get("hls"))
        .and_then(|hls| hls.get("url"))
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(&media_id, "url"))?;
    let manifest = Url::parse(hls)?;
    let formats = extract_m3u8_fatal_master(client, &manifest, name, &media_id, None, true)?;

    let mut fields: Vec<(&str, Value)> = vec![
        ("id", Value::from(media_id.clone())),
        (
            "_old_archive_ids",
            json!([format!("limelightmedia {media_id}")]),
        ),
        ("formats", Value::Null),
        ("subtitles", Value::Null),
    ];
    let string = |key: &str| {
        product
            .get(key)
            .and_then(Value::as_str)
            .map(|text| Value::from(text.to_owned()))
    };
    let mut push = |name: &'static str, value: Option<Value>| {
        if let Some(value) = value.filter(|value| !value.is_null()) {
            fields.push((name, value));
        }
    };
    push("title", string("title"));
    push("description", string("summary"));
    let tags: Vec<Value> = product
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|tag| tag.is_string())
        .cloned()
        .collect();
    push("tags", (!tags.is_empty()).then(|| Value::Array(tags)));
    push(
        "duration",
        Some(super::streamable::int_or_none(product.get("duration"))),
    );
    push(
        "season_number",
        Some(super::streamable::int_or_none(product.get("seasonNumber"))),
    );
    push(
        "episode_number",
        Some(super::streamable::int_or_none(product.get("episodeNumber"))),
    );
    push(
        "series",
        product
            .get("collection")
            .and_then(|collection| collection.get("title"))
            .and_then(Value::as_str)
            .map(|title| Value::from(title.to_owned())),
    );
    push(
        "age_limit",
        Some(super::videopress::parse_age_limit(
            product.get("rating").and_then(|rating| rating.get("name")),
        )),
    );

    let mut info = dict_info(page, (name, key), fields, formats.formats);
    info.subtitles = formats.subtitles;
    Ok(info)
}

impl Extractor for TV5UnisVideo {
    fn key(&self) -> &'static str {
        "TV5UnisVideo"
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&GEO_COUNTRIES)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor("tv5unis:video");
        crate::screen::screen(&format!(
            "[tv5unis:video] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        extract_product(
            client,
            url,
            "TV5UnisVideo",
            "tv5unis:video",
            "productById",
            &format!("id: {id}"),
            &id,
        )
        .map(Some)
    }
}

impl Extractor for TV5Unis {
    fn key(&self) -> &'static str {
        "TV5Unis"
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&GEO_COUNTRIES)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let slug = found["id"].to_owned();
        crate::screen::set_extractor("tv5unis");
        crate::screen::screen(&format!(
            "[tv5unis] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_gql_args`: the season and episode only when both are in the URL.
        let mut args = format!("rootProductSlug: \"{slug}\"");
        if let (Some(season), Some(episode)) =
            (found.name("season_number"), found.name("episode_number"))
        {
            args.push_str(&format!(
                ", seasonNumber: {}, episodeNumber: {}",
                season.as_str(),
                episode.as_str()
            ));
        }
        extract_product(
            client,
            url,
            "TV5Unis",
            "tv5unis",
            "productByRootProductSlug",
            &args,
            &slug,
        )
        .map(Some)
    }
}

/// URL matching for `--list-extractors`.
pub fn video_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video_suitable(url: &str) -> bool {
        VIDEO_URL.is_match(url)
    }

    fn series_suitable(url: &str) -> bool {
        SERIES_URL.is_match(url)
    }

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(video_suitable(
            "https://www.tv5unis.ca/videos/bande-annonces/144041"
        ));
        assert!(!video_suitable(
            "https://www.tv5unis.ca/videos/boite-a-savon"
        ));
        assert!(series_suitable(
            "https://www.tv5unis.ca/videos/boite-a-savon"
        ));
        assert!(series_suitable(
            "https://www.tv5unis.ca/videos/watatatow/saisons/11/episodes/1"
        ));
    }
}
