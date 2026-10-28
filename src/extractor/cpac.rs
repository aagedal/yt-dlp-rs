//! Upstream `CPACIE` and `CPACPlaylistIE` (`yt_dlp/extractor/cpac.py`):
//! CPAC episodes from the Crafter content model (an HLS playlist whose
//! renditions prefer the URL's language) and programs or searches as
//! playlists of their pages' episode URLs.
use super::{Extractor, dict_info, download_json, download_json_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Cpac;
pub struct CpacPlaylist;

const NAME: &str = "cpac";
const KEY: &str = "CPAC";
const PLAYLIST_NAME: &str = "cpac:playlist";
const PLAYLIST_KEY: &str = "CPACPlaylist";
const API: &str = "https://www.cpac.ca/api/1/services/contentModel.json?url=/site/website/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?cpac\.ca/(?P<fr>l-)?episode\?id=(?P<id>[\da-f]{8}(?:-[\da-f]{4}){3}-[\da-f]{12})",
    )
    .unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^https?://(?:www\.)?cpac\.ca/(?:program|search|(?P<fr>emission|rechercher))\?(?:[^&]+&)*?(?P<id>(?:id=\d+|programId=\d+|key=[^&]+))",
    )
    .unwrap()
});

/// `CPACIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `CPACPlaylistIE._match_id`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
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

/// `str_or_none(value)`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.into()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

impl Extractor for Cpac {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);
        let lang = if url.as_str().contains("/l-episode?") {
            "fr"
        } else {
            "en"
        };
        let api = Url::parse(&format!(
            "{API}episode/index.xml&crafterSite=cpacca&id={video_id}"
        ))?;
        let content = download_json(client, &api, NAME, &video_id)?;
        let video_url = content
            .pointer("/page/details/videoUrl")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(video_url) = video_url else {
            // `content['details']` of the whole response.
            return Err(key_error(&video_id, "details"));
        };
        let page = &content["page"];
        let details = &page["details"];
        let title = details
            .get(format!("title_{lang}_t"))
            .ok_or_else(|| key_error(&video_id, &format!("title_{lang}_t")))?;
        let title = str_or_none(Some(title)).map_or(Value::Null, Value::from);
        let stream = super::extract_m3u8_fatal_master(
            client,
            &Url::parse(&video_url).map_err(|_| {
                Error::UnsupportedHtml5(
                    "CPAC video URLs that are not URLs are not supported natively".into(),
                )
            })?,
            NAME,
            &video_id,
            Some("hls"),
            false,
        )?;
        let mut formats = stream.formats;
        for format in &mut formats {
            // Prefer the URL's language.
            let preference = match format.language.as_deref() {
                Some(language) if language == lang => 10,
                None | Some("") => -1,
                Some(_) => -10,
            };
            format
                .extra
                .insert("language_preference".into(), Value::from(preference));
        }
        let category = details
            .get(format!("category_{lang}_t"))
            .ok_or_else(|| key_error(&video_id, &format!("category_{lang}_t")))?;
        let category = str_or_none(Some(category)).filter(|category| !category.is_empty());
        let thumbnail = str_or_none(details.get(format!("image_{lang}_s")))
            .and_then(|image| super::videopress::urljoin(Some(url.as_str()), &image));
        let is_live = match details.get("type") {
            None | Some(Value::Null) => Value::Null,
            Some(kind) => Value::Bool(kind.as_str() == Some("live")),
        };
        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("formats", Value::Null),
                ("title", title),
                (
                    "description",
                    str_or_none(details.get(format!("description_{lang}_t")))
                        .map_or(Value::Null, Value::from),
                ),
                (
                    "timestamp",
                    details
                        .get("liveDateTime")
                        .and_then(Value::as_str)
                        .and_then(|date| crate::dates::unified_timestamp(date, false))
                        .map_or(Value::Null, Value::from),
                ),
                (
                    "categories",
                    category.map_or(Value::Null, |category| json!([category])),
                ),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
                ("is_live", is_live),
            ],
            formats,
        )))
    }
}

/// `url_result(url)`.
fn url_entry(target: &str) -> Result<VideoInfo> {
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
            "ie_key": null,
            "entry": {"_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for CpacPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(PLAYLIST_NAME, url);
        let text = url.as_str();
        let lang = if ["/emission?", "/rechercher?"]
            .iter()
            .any(|part| text.contains(part))
        {
            "fr"
        } else {
            "en"
        };
        let (pl_type, list_type) = if ["/program?", "/emission?"]
            .iter()
            .any(|part| text.contains(part))
        {
            ("program", "itemList")
        } else {
            ("search", "searchResult")
        };
        let api_url = Url::parse(&format!(
            "{API}{pl_type}/index.xml&crafterSite=cpacca&{video_id}"
        ))?;
        let mut content = download_json(client, &api_url, PLAYLIST_NAME, &video_id)?;
        let total_pages = match content.pointer(&format!("/page/{list_type}/totalPages")) {
            Some(Value::Number(number)) => number.as_i64().unwrap_or(1),
            Some(Value::String(text)) => text.trim().parse().unwrap_or(1),
            _ => 1,
        };
        let mut entries = Vec::new();
        for page in 1..=total_pages {
            if page > 1 {
                let page_text = page.to_string();
                let next = super::update_url_query(&api_url, &[("page", page_text.as_str())]);
                // `_download_json(..., fatal=False)`: `False` on failure.
                content = match download_json_note(
                    client,
                    &next,
                    PLAYLIST_NAME,
                    &video_id,
                    &format!("Downloading continuation - {page}"),
                ) {
                    Ok(content) => content,
                    Err(Error::ExtractorRequest { source, .. }) => {
                        crate::screen::warning(&format!(
                            "[{PLAYLIST_NAME}] {video_id}: Unable to download JSON metadata: {}",
                            crate::upstream_error_text(&source)
                        ));
                        Value::Null
                    }
                    Err(error) => return Err(error),
                };
            }
            if let Some(Value::Array(items)) = content.pointer(&format!("/page/{list_type}/item")) {
                for item in items {
                    if let Some(episode) = item
                        .get(format!("url_{lang}_s"))
                        .and_then(Value::as_str)
                        .and_then(|path| super::videopress::urljoin(Some(url.as_str()), path))
                    {
                        entries.push(url_entry(&episode)?);
                    }
                }
            }
        }
        let title = content
            .pointer(&format!("/page/program/title_{lang}_t"))
            .filter(|value| super::nuxt::truthy(value))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| video_id.rsplit('=').next().unwrap_or_default().to_owned());
        let description = content
            .pointer(&format!("/page/program/description_{lang}_t"))
            .filter(|value| !value.is_null())
            .cloned();
        let mut keys = vec!["id"];
        if !title.is_empty() {
            keys.push("title");
        }
        let mut extra = Map::new();
        if let Some(description) = description {
            keys.push("description");
            extra.insert("description".into(), description);
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;{}{}",
                keys.join(","),
                crate::info_order::GENERATOR
            )
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: video_id,
            title,
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_KEY.into(),
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
            "https://www.cpac.ca/episode?id=fc7edcae-4660-47e1-ba61-5b7f29a9db0f"
        ));
        assert!(suitable(
            "https://www.cpac.ca/l-episode?id=fc7edcae-4660-47e1-ba61-5b7f29a9db0f"
        ));
        assert!(playlist_suitable("https://www.cpac.ca/program?id=6"));
        assert!(playlist_suitable(
            "https://www.cpac.ca/search?key=hudson&type=all&order=desc"
        ));
        assert!(playlist_suitable(
            "https://www.cpac.ca/rechercher?key=hudson"
        ));
        assert!(!suitable("https://www.cpac.ca/program?id=6"));
    }
}
