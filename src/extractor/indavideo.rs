//! Upstream `IndavideoEmbedIE` (`yt_dlp/extractor/indavideo.py`):
//! Indavideo players from the `getVideoData` JSON (each video file with its
//! height's token), with the page's metadata; player iframes on other pages
//! are followed.
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct IndavideoEmbed;

pub(crate) const NAME: &str = "IndavideoEmbed";
pub(crate) const KEY: &str = "IndavideoEmbed";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:embed\.)?indavideo\.hu/player/video/|assets\.indavideo\.hu/swf/player\.swf\?.*\b(?:v(?:ID|id))=)(?P<id>[\da-f]+)",
    )
    .unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+\bsrc=["'](?P<url>(?:https?:)//embed\.indavideo\.hu/player/video/[\da-f]+)"#,
    )
    .unwrap()
});
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.(\d{3,4})\.mp4(?:\?|$)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Ok(embed) = url.join(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Indavideo {what} are not supported natively"))
}

fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

impl Extractor for IndavideoEmbed {
    fn key(&self) -> &'static str {
        KEY
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
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs_f64())
            .unwrap_or_default();
        let mut api = Url::parse(&format!(
            "https://amfphp.indavideo.hu/SYm0json.php/player.playerHandler.getVideoData/{video_id}/"
        ))?;
        api.query_pairs_mut().append_pair("_", &now.to_string());
        let response = download_json(client, &api, NAME, &video_id)?;
        let video = response
            .get("data")
            .ok_or_else(|| key_error(&video_id, "data"))?;
        // `list(set(video_urls))`: distinct URLs (upstream's order is a
        // set's; formats are sorted by height afterwards).
        let mut video_urls: Vec<String> = Vec::new();
        let files: Vec<&Value> = match video.get("video_files") {
            Some(Value::Array(files)) => files.iter().collect(),
            Some(Value::Object(files)) => files.values().collect(),
            _ => Vec::new(),
        };
        for file in files {
            let Some(file) = file.as_str() else {
                return Err(unsupported("video files that are not strings"));
            };
            if !video_urls.iter().any(|known| known == file) {
                video_urls.push(file.to_owned());
            }
        }
        let filesh: Map<String, Value> = match video.get("filesh") {
            Some(Value::Object(filesh)) => filesh.clone(),
            _ => Map::new(),
        };
        let mut formats = Vec::new();
        for video_url in &video_urls {
            let mut height = HEIGHT
                .captures(video_url)
                .and_then(|found| found[1].parse::<i64>().ok());
            if height.is_none_or(|height| height == 0) && filesh.len() == 1 {
                height = filesh.keys().next().and_then(|key| key.trim().parse().ok());
            }
            let key = height.map_or_else(|| "None".to_owned(), |height| height.to_string());
            let Some(token) = filesh.get(&key).filter(|token| !token.is_null()) else {
                continue;
            };
            let token = match token {
                Value::String(token) => token.clone(),
                Value::Number(number) => number.to_string(),
                _ => return Err(unsupported("tokens that are not strings")),
            };
            let target = super::update_url_query(
                &Url::parse(video_url).map_err(|_| unsupported("video files that are not URLs"))?,
                &[("token", token.as_str())],
            );
            formats.push(dict_format(
                vec![
                    ("url", target.as_str().into()),
                    ("height", height.map_or(Value::Null, Value::from)),
                ],
                NAME,
                &video_id,
            )?);
        }
        let timestamp = match video.get("date") {
            Some(Value::String(date)) if !date.is_empty() => {
                crate::dates::parse_iso8601_space(&format!("{date} +0200"))
                    .map_or(Value::Null, Value::from)
            }
            Some(value) if super::nuxt::truthy(value) => {
                return Err(unsupported("dates that are not strings"));
            }
            _ => Value::Null,
        };
        let thumbnails: Vec<Value> = match video.get("thumbnails") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(|thumbnail| {
                            serde_json::json!({"url": super::proto_relative(thumbnail)})
                        })
                        .ok_or_else(|| unsupported("thumbnails that are not strings"))
                })
                .collect::<Result<_>>()?,
            None => Vec::new(),
            Some(_) => return Err(unsupported("thumbnail lists that are not lists")),
        };
        let mut tags = Vec::new();
        if let Some(Value::Array(items)) = video.get("tags") {
            for tag in items {
                tags.push(
                    tag.get("title")
                        .cloned()
                        .ok_or_else(|| key_error(&video_id, "title"))?,
                );
            }
        }
        let get = |key: &str| video.get(key).cloned().unwrap_or(Value::Null);
        let id = video
            .get("id")
            .filter(|id| super::nuxt::truthy(id))
            .cloned()
            .unwrap_or_else(|| Value::from(video_id.clone()));
        let forced_string = id.is_number();
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", id),
                ("title", get("title")),
                ("description", get("description")),
                ("thumbnails", Value::Array(thumbnails)),
                ("uploader", get("user_name")),
                ("uploader_id", get("user_id")),
                ("timestamp", timestamp),
                (
                    "duration",
                    int_or_none(video.get("length")).map_or(Value::Null, Value::from),
                ),
                (
                    "age_limit",
                    super::videopress::parse_age_limit(video.get("age_limit")),
                ),
                ("tags", Value::Array(tags)),
                ("formats", Value::Null),
            ],
            formats,
        );
        if forced_string {
            info.id = match video.get("id") {
                Some(Value::Number(number)) => number.to_string(),
                _ => info.id,
            };
            crate::screen::warning(
                "\"id\" field is not a string - forcing string conversion, there is an error in extractor",
            );
        }
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=url");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://embed.indavideo.hu/player/video/1bdc3c6d80/"
        ));
        assert!(suitable(
            "https://assets.indavideo.hu/swf/player.swf?v=fe25e500&vID=1bdc3c6d80&autostart=1"
        ));
        assert!(!suitable(
            "https://index.indavideo.hu/video/Hod_Nemetorszagban"
        ));
    }
}
