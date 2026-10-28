//! Upstream `DeuxMIE` and `DeuxMNewsIE` (`yt_dlp/extractor/deuxm.py`): 2M
//! replays from the watch-detail API and news articles from the
//! articles-by-URL API, each a single media URL with its metadata.
use super::{Extractor, dict_info, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct DeuxM;
pub struct DeuxMNews;

const NAME: &str = "DeuxM";
const NEWS_NAME: &str = "DeuxMNews";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?2m\.ma/[^/]+/replay/single/(?P<id>([\w.]{1,24})+)").unwrap()
});
static NEWS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?2m\.ma/(?P<lang>\w+)/news/(?P<id>[^/#?]+)").unwrap()
});

/// `DeuxMIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `DeuxMNewsIE._match_valid_url`.
pub fn news_suitable(url: &str) -> bool {
    NEWS_URL.is_match(url)
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

fn get<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| key_error(id, key))
}

/// A `{'id', 'title', 'url', 'description', 'thumbnail'}` result: its URL
/// is the only format.
fn media_result(
    page: &Url,
    name: &'static str,
    id: String,
    title: Value,
    media: &Value,
    description: Value,
    thumbnail: Option<&Value>,
) -> Result<VideoInfo> {
    let Value::String(media) = media else {
        return Err(Error::UnsupportedHtml5(
            "2M media URLs that are not strings are not supported natively".into(),
        ));
    };
    let ext = super::direct::determine_ext(media)
        .unwrap_or_else(|| "unknown_video".into())
        .to_lowercase();
    let mut format: crate::VideoFormat =
        serde_json::from_value(json!({"format_id": "", "url": media, "ext": ext}))
            .map_err(|error| Error::Reported(format!("[{name}] {id}: {error}")))?;
    crate::info_order::mark_format(&mut format, "url-only");
    let thumbnail = thumbnail
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map_or(Value::Null, Value::from);
    let fields = vec![
        ("id", Value::from(id)),
        ("title", title),
        ("url", media.clone().into()),
        ("description", description),
        ("thumbnail", thumbnail),
    ];
    let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
    let shape = format!("untyped-dict;{}", keys.join(","));
    Ok(crate::info_order::mark_info(
        dict_info(page, (name, name), fields, vec![format]),
        &shape,
    ))
}

impl Extractor for DeuxM {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);
        let api = Url::parse(&format!("https://2m.ma/api/watchDetail/{video_id}"))?;
        let data = download_json(client, &api, NAME, &video_id)?;
        let video = get(get(&data, "response", &video_id)?, "News", &video_id)?;
        let opt = |key: &str| video.get(key).cloned().unwrap_or(Value::Null);
        let media = get(video, "url", &video_id)?;
        media_result(
            url,
            NAME,
            video_id.clone(),
            opt("titre"),
            media,
            opt("description"),
            video.get("image"),
        )
        .map(Some)
    }
}

impl Extractor for DeuxMNews {
    fn key(&self) -> &'static str {
        NEWS_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = NEWS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let article_name = found["id"].to_owned();
        let lang = found["lang"].to_owned();
        start(NEWS_NAME, url);
        let api = Url::parse(&format!(
            "https://2m.ma/api/articlesByUrl?lang={lang}&url=/news/{article_name}"
        ))?;
        let data = download_json(client, &api, NEWS_NAME, &article_name)?;
        let articles = get(
            get(&data, "response", &article_name)?,
            "article",
            &article_name,
        )?;
        // An uncaught `IndexError` is reported as its message.
        let index_error = || Error::Reported("list index out of range".into());
        let video = match articles {
            Value::Array(articles) => articles.first().ok_or_else(index_error)?,
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "2M article lists that are not lists are not supported natively".into(),
                ));
            }
        };
        let id = match get(video, "id", &article_name)? {
            Value::String(id) => id.clone(),
            Value::Number(id) => id.to_string(),
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "2M article IDs that are not strings are not supported natively".into(),
                ));
            }
        };
        let media = match get(video, "image", &article_name)? {
            Value::Array(images) => images.first().ok_or_else(index_error)?,
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "2M article images that are not lists are not supported natively".into(),
                ));
            }
        };
        let opt = |key: &str| video.get(key).cloned().unwrap_or(Value::Null);
        media_result(
            url,
            NEWS_NAME,
            id,
            opt("title"),
            media,
            opt("content"),
            video.get("cover"),
        )
        .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://2m.ma/fr/replay/single/6351d439b15e1a613b3debe8"
        ));
        assert!(news_suitable(
            "https://2m.ma/fr/news/Kan-Ya-Mkan-d%C3%A9poussi%C3%A8re-l-histoire-20221028"
        ));
        assert!(!suitable("https://2m.ma/fr/news/x"));
    }
}
