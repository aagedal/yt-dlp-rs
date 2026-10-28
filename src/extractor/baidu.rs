//! Upstream `BaiduVideoIE` (`yt_dlp/extractor/baidu.py`): Baidu video
//! series as playlists of their episodes' `url` results from the app API,
//! with the series title and unescaped intro.
use super::{Extractor, download_json_note};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BaiduVideo;

const NAME: &str = "BaiduVideo";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://v\.baidu\.com/(?P<type>[a-z]+)/(?P<id>\d+)\.htm").unwrap()
});

/// `_match_valid_url`.
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
    Error::UnsupportedHtml5(format!("Baidu {what} are not supported natively"))
}

/// `_call_api(path, category, playlist_id, note)`.
fn call_api(
    client: &Client,
    path: &str,
    category: &str,
    playlist_id: &str,
    note: &str,
) -> Result<Value> {
    let api = Url::parse(&format!(
        "http://app.video.baidu.com/{path}/?worktype=adnative{category}&id={playlist_id}"
    ))?;
    download_json_note(client, &api, NAME, playlist_id, note)
}

/// `url_result(episode['url'], video_title=episode['title'])`: no `ie_key`.
fn episode_entry(target: &str, title: Value) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = String::new();
    // `video_title=None` adds no `title`.
    let mut entry = serde_json::Map::new();
    if !title.is_null() {
        entry.insert("title".into(), title);
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": null,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for BaiduVideo {
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
        let category = match &found["type"] {
            "show" => "tvshow",
            "tv" => "tvplay",
            other => other,
        }
        .to_owned();
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let detail = call_api(
            client,
            "xqinfo",
            &category,
            &playlist_id,
            "Download playlist JSON metadata",
        )?;
        let title = match detail.get("title") {
            Some(Value::String(title)) => Some(title.clone()).filter(|title| !title.is_empty()),
            Some(Value::Null) => None,
            Some(_) => return Err(unsupported("titles that are not strings")),
            None => return Err(key_error(&playlist_id, "title")),
        };
        let description = match detail.get("intro") {
            None | Some(Value::Null) => None,
            Some(Value::String(intro)) => Some(Value::from(crate::html::unescape_html(intro))),
            Some(_) => return Err(unsupported("intros that are not strings")),
        };
        let episodes = call_api(
            client,
            "xqsingle",
            &category,
            &playlist_id,
            "Download episodes JSON metadata",
        )?;
        let videos = match episodes.get("videos") {
            Some(Value::Array(videos)) => videos,
            Some(_) => return Err(unsupported("episode lists that are not lists")),
            None => return Err(key_error(&playlist_id, "videos")),
        };
        let mut entries = Vec::new();
        for episode in videos {
            let target = match episode.get("url") {
                Some(Value::String(target)) => target.clone(),
                Some(_) => return Err(unsupported("episode URLs that are not strings")),
                None => return Err(key_error(&playlist_id, "url")),
            };
            let title = episode
                .get("title")
                .cloned()
                .ok_or_else(|| key_error(&playlist_id, "title"))?;
            entries.push(episode_entry(&target, title)?);
        }
        Ok(Some(super::nrk::playlist_result(
            url,
            NAME,
            playlist_id,
            title,
            description,
            entries,
            false,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://v.baidu.com/comic/1069.htm?frp=bdbrand&q=中华小当家"
        ));
        assert!(suitable("http://v.baidu.com/show/11595.htm?frp=bdbrand"));
        assert!(!suitable("http://v.baidu.com/comic/x.htm"));
    }
}
