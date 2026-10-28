//! Upstream `LiTVIE` (`yt_dlp/extractor/litv.py`): LiTV pages' Next.js
//! `programInformation` and `seriesTree` (a playlist of the series'
//! episodes unless `--no-playlist`), the `get-urls` API (a JSON POST; its
//! region and other errors) and the asset's HLS formats.
use super::{Extractor, dict_info};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct LiTv;

const NAME: &str = "LiTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?litv\.tv/(?:[^/?#]+/watch/|vod/[^/?#]+/content\.do\?content_id=)(?P<id>[\w-]+)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("LiTV {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// Python `str()` of a scalar JSON value (`None` for containers).
fn py_str(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) if number.is_f64() => None,
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(true) => Some("True".into()),
        Value::Bool(false) => Some("False".into()),
        _ => None,
    }
}

/// `url_result(smuggle_url(URL, {'force_noplaylist': True}))`.
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

/// `_extract_playlist(playlist_data, content_type)`.
fn extract_playlist(
    url: &Url,
    playlist_data: &Value,
    content_type: Option<&Value>,
    video_id: &str,
) -> Result<ExtractResult> {
    let content_type = match content_type {
        None | Some(Value::Null) => "None".to_owned(),
        Some(value) => py_str(value).ok_or_else(|| unsupported("content types"))?,
    };
    let mut entries = Vec::new();
    // `('seasons', ..., 'episodes', lambda _, v: v['content_id'])`.
    let values = |value: Option<&Value>| -> Vec<Value> {
        match value {
            Some(Value::Array(items)) => items.clone(),
            Some(Value::Object(items)) => items.values().cloned().collect(),
            _ => Vec::new(),
        }
    };
    for season in values(playlist_data.get("seasons")) {
        for episode in values(season.get("episodes")) {
            let Some(content_id) = episode
                .get("content_id")
                .filter(|value| super::nuxt::truthy(value))
            else {
                continue;
            };
            let content_id =
                py_str(content_id).ok_or_else(|| unsupported("episode content IDs"))?;
            let target = super::brightcove::smuggle(
                &format!("https://www.litv.tv/{content_type}/watch/{content_id}"),
                &json!({"force_noplaylist": true}),
            );
            entries.push(url_entry(&target)?);
        }
    }
    let playlist_id = match playlist_data.get("content_id") {
        Some(value) => value,
        None if playlist_data.is_object() => return Err(key_error(video_id, "content_id")),
        None => return Err(unsupported("series trees that are not objects")),
    };
    let playlist_id = match playlist_id {
        Value::Null => String::new(),
        value => py_str(value).ok_or_else(|| unsupported("series IDs"))?,
    };
    let title = playlist_data
        .get("title")
        .filter(|value| super::nuxt::truthy(value))
        .map(|title| py_str(title).ok_or_else(|| unsupported("series titles")))
        .transpose()?
        .unwrap_or_default();
    let mut keys = Vec::new();
    if !playlist_id.is_empty() {
        keys.push("id");
    }
    if !title.is_empty() {
        keys.push("title");
    }
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    Ok(ExtractResult::Playlist(PlaylistInfo {
        id: playlist_id,
        title,
        webpage_url: url.clone(),
        extractor: NAME.into(),
        extractor_key: NAME.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    }))
}

impl Extractor for LiTv {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_GEO_COUNTRIES = ['TW']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["TW"])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !VALID_URL.is_match(url.as_str()) {
            return Ok(None);
        }
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let (bare, smuggled) = super::brightcove::unsmuggle(url.as_str());
        let Some(found) = VALID_URL.captures(&bare) else {
            return Err(unsupported("smuggled URLs that do not match"));
        };
        let video_id = found["id"].to_owned();
        let page = Url::parse(&bare)?;
        let webpage = super::download_webpage(client, &page, NAME, &video_id)?;
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        let vod_data = next_data
            .get("props")
            .ok_or_else(|| key_error(&video_id, "props"))?
            .get("pageProps")
            .ok_or_else(|| key_error(&video_id, "pageProps"))?;
        let empty = Value::Object(Map::new());
        let program_info = vod_data
            .get("programInformation")
            .filter(|value| value.is_object() && super::nuxt::truthy(value))
            .unwrap_or(&empty);
        if let Some(playlist_data) = vod_data
            .get("seriesTree")
            .filter(|value| super::nuxt::truthy(value))
        {
            // `_yes_playlist(program_info.get('series_id'), video_id,
            // smuggled_data)`.
            let series_id = program_info
                .get("series_id")
                .filter(|value| super::nuxt::truthy(value));
            let yes = match series_id {
                None => false,
                Some(series_id) => match smuggled.get("force_noplaylist") {
                    Some(Value::Null) | None => {
                        let series_id =
                            py_str(series_id).ok_or_else(|| unsupported("series IDs"))?;
                        if super::no_playlist() {
                            crate::screen::screen(&format!(
                                "[{NAME}] Downloading just the video {video_id} because of --no-playlist"
                            ));
                            false
                        } else {
                            crate::screen::screen(&format!(
                                "[{NAME}] Downloading playlist {series_id} - add --no-playlist to download just the video {video_id}"
                            ));
                            true
                        }
                    }
                    Some(no_playlist) => !super::nuxt::truthy(no_playlist),
                },
            };
            if yes {
                return extract_playlist(
                    url,
                    playlist_data,
                    program_info.get("content_type"),
                    &video_id,
                )
                .map(Some);
            }
        }
        let asset_id = program_info
            .pointer("/assets/0/asset_id")
            .and_then(Value::as_str);
        let (asset_id, media_type) = match asset_id {
            // A VOD.
            Some(asset_id) => (Value::from(asset_id), Value::from("vod")),
            // A live stream.
            None => (
                program_info
                    .get("content_id")
                    .cloned()
                    .ok_or_else(|| key_error(&video_id, "content_id"))?,
                program_info
                    .get("content_type")
                    .cloned()
                    .ok_or_else(|| key_error(&video_id, "content_type"))?,
            ),
        };
        let (puid, endpoint) = match crate::cookies::get_cookie("https://www.litv.tv/", "PUID")
            .filter(|puid| !puid.is_empty())
        {
            Some(puid) => (puid, "get-urls"),
            None => (super::drtv::uuid4(), "get-urls-no-auth"),
        };
        let body = crate::output::python_json_dumps(&json!({
            "AssetId": asset_id,
            "MediaType": media_type,
            "puid": puid,
        }));
        let video_data = super::post_json_note_headers(
            client,
            &Url::parse(&format!("https://www.litv.tv/api/{endpoint}"))?,
            NAME,
            &video_id,
            "Downloading JSON metadata",
            &[("Content-Type", "application/json")],
            body,
        )?;
        if let Some(error) = video_data.get("error").filter(|error| error.is_object()) {
            let error_msg = error
                .get("message")
                .and_then(Value::as_str)
                .filter(|message| !message.is_empty());
            if let Some(error_msg) = error_msg {
                if error_msg.contains("OutsideRegionError") {
                    return Err(crate::geo::restricted(
                        &format!("[{NAME}] {video_id}: This video is available in Taiwan only"),
                        &["TW"],
                    ));
                }
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: {NAME} said: {error_msg}"
                )));
            }
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unexpected error from {NAME}; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let asset_urls = video_data
            .get("result")
            .ok_or_else(|| key_error(&video_id, "result"))?
            .get("AssetURLs")
            .ok_or_else(|| key_error(&video_id, "AssetURLs"))?;
        let m3u8_url = match asset_urls {
            Value::Array(urls) => urls
                .first()
                .ok_or_else(|| Error::Reported("list index out of range".into()))?,
            _ => return Err(unsupported("asset URL lists that are not lists")),
        }
        .as_str()
        .ok_or_else(|| unsupported("asset URLs that are not strings"))?;
        let mut formats = super::extract_m3u8_fatal_master(
            client,
            &Url::parse(m3u8_url)?,
            NAME,
            &video_id,
            Some("hls"),
            false,
        )?
        .formats;
        for format in &mut formats {
            // LiTV HLS segments don't like compression.
            format
                .http_headers
                .get_or_insert_with(Map::new)
                .insert("Accept-Encoding".into(), "identity".into());
            crate::info_order::mark_format_update(format, "http_headers");
        }
        // `join_nonempty('title', 'secondary_mark', delim='', from_dict=...)`.
        let mut title = String::new();
        for key in ["title", "secondary_mark"] {
            if let Some(value) = program_info
                .get(key)
                .filter(|value| super::nuxt::truthy(value))
            {
                title.push_str(&py_str(value).ok_or_else(|| unsupported("title parts"))?);
            }
        }
        let categories: Vec<Value> = match program_info.get("genres") {
            Some(Value::Array(genres)) => genres
                .iter()
                .filter_map(|genre| genre.get("name").filter(|name| name.is_string()).cloned())
                .collect(),
            _ => Vec::new(),
        };
        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("formats", Value::Null),
            ("title", Value::from(title)),
        ];
        if let Some(description) = program_info
            .get("description")
            .filter(|value| value.is_string())
        {
            fields.push(("description", description.clone()));
        }
        if let Some(thumbnail) = program_info
            .get("picture")
            .and_then(Value::as_str)
            .and_then(|picture| {
                super::videopress::urljoin(Some("https://p-cdnstatic.svc.litv.tv/"), picture)
            })
        {
            fields.push(("thumbnail", thumbnail.into()));
        }
        if !categories.is_empty() {
            fields.push(("categories", Value::Array(categories)));
        }
        let episode_number = super::streamable::int_or_none(program_info.get("episode"));
        if !episode_number.is_null() {
            fields.push(("episode_number", episode_number));
        }
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (NAME, NAME),
            fields,
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.litv.tv/drama/watch/VOD00041610"));
        assert!(suitable(
            "https://www.litv.tv/vod/drama/content.do?content_id=VOD00044841"
        ));
        assert!(!suitable("https://www.litv.tv/drama/VOD00041610"));
    }
}
