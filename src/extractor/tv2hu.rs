//! Upstream `TV2HuIE` and `TV2HuSeriesIE` (`yt_dlp/extractor/tv2hu.py`):
//! TV2 Play (Hungary) videos from the search API's video (its player's
//! streaming JSON as HLS, with the series metadata), show pages as playlists
//! of their ribbons, and ribbons as playlists of their cards.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Tv2Hu;
pub struct Tv2HuSeries;

const NAME: &str = "tv2play.hu";
const KEY: &str = "TV2Hu";
const SERIES_NAME: &str = "tv2playseries.hu";
const SERIES_KEY: &str = "TV2HuSeries";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?tv2play\.hu/(?P<id>[^#&?]+)").unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?tv2play\.hu/szalag/(?P<id>[^#&?]+)").unwrap()
});

/// `TV2HuIE._match_id` (`(?!szalag/)`).
pub fn suitable(url: &str) -> bool {
    VALID_URL
        .captures(url)
        .is_some_and(|found| !found["id"].starts_with("szalag/"))
}

/// `TV2HuSeriesIE._match_id`.
pub fn series_suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
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

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("TV2 Play {what} are not supported natively"))
}

/// The playlist's `extractor_key`, which differs from its `IE_NAME`.
fn with_key(mut result: ExtractResult, key: &str) -> ExtractResult {
    if let ExtractResult::Playlist(playlist) = &mut result {
        playlist.extractor_key = key.into();
    }
    result
}

fn id_text(value: &Value) -> Result<String> {
    match value {
        Value::String(id) => Ok(id.clone()),
        Value::Number(id) => Ok(id.to_string()),
        _ => Err(unsupported("IDs that are not strings")),
    }
}

impl Extractor for Tv2Hu {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        let found = VALID_URL.captures(url.as_str()).expect("suitable");
        let page_id = found["id"].to_owned();
        start(NAME, url);
        let api = Url::parse(&format!("https://tv2play.hu/api/search/{page_id}"))?;
        let json_data = download_json(client, &api, NAME, &page_id)?;
        let content_type = json_data
            .get("contentType")
            .ok_or_else(|| key_error(&page_id, "contentType"))?;
        if content_type.as_str() == Some("showpage") {
            // `('pages', ..., 'tabs', ..., 'ribbonIds')`, the first list.
            let pages: Vec<&Value> = match json_data.get("pages") {
                Some(Value::Array(pages)) => pages.iter().collect(),
                Some(Value::Object(pages)) => pages.values().collect(),
                _ => Vec::new(),
            };
            let ribbon_ids = pages
                .into_iter()
                .filter_map(|page| page.get("tabs"))
                .flat_map(|tabs| match tabs {
                    Value::Array(tabs) => tabs.iter().collect::<Vec<_>>(),
                    Value::Object(tabs) => tabs.values().collect(),
                    _ => Vec::new(),
                })
                .filter_map(|tab| tab.get("ribbonIds"))
                .find_map(Value::as_array)
                .ok_or_else(|| Error::Reported("'NoneType' object is not iterable".into()))?;
            let mut entries = Vec::new();
            for ribbon_id in ribbon_ids {
                let ribbon = id_text(ribbon_id)?;
                let mut entry = super::nrk::url_entry(
                    &format!("https://tv2play.hu/szalag/{ribbon}"),
                    SERIES_KEY,
                    Some(&ribbon),
                    None,
                    None,
                )?;
                if let Some(marker) = entry.extra.get_mut(crate::TRANSPARENT) {
                    marker["entry"]["id"] = ribbon_id.clone();
                    marker["extractor"] = SERIES_NAME.into();
                }
                entries.push(entry);
            }
            return Ok(Some(with_key(
                super::nrk::playlist_result(url, NAME, page_id, None, None, entries, false),
                KEY,
            )));
        }
        if content_type.as_str() != Some("video") {
            return Err(Error::Reported(format!("Unsupported URL: {url}")));
        }
        let video_id = id_text(
            json_data
                .get("id")
                .ok_or_else(|| key_error(&page_id, "id"))?,
        )?;
        let player_id = json_data
            .get("playerId")
            .map(|id| match id {
                Value::Null => Ok("None".to_owned()),
                other => id_text(other),
            })
            .transpose()?
            .unwrap_or_else(|| "None".into());
        let streaming = download_json(
            client,
            &Url::parse(&format!(
                "https://tv2play.hu/api/streaming-url?playerId={player_id}"
            ))?,
            NAME,
            &video_id,
        )?;
        let video_json_url = match streaming.get("url") {
            Some(Value::String(url)) => {
                Url::parse(url).map_err(|_| unsupported("streaming URLs that are not URLs"))?
            }
            Some(_) => return Err(unsupported("streaming URLs that are not strings")),
            None => return Err(key_error(&page_id, "url")),
        };
        let video_json = download_json(client, &video_json_url, NAME, &video_id)?;
        let manifest = video_json
            .pointer("/bitrates/hls")
            .and_then(Value::as_str)
            .map(super::proto_relative)
            .and_then(|hls| Url::parse(&hls).ok())
            .ok_or_else(|| unsupported("streams without an HLS playlist"))?;
        let mut stream =
            super::extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, true)?;
        super::hls::without_ext(&mut stream);
        let title = json_data
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&page_id, "title"))?;
        let thumbnail = match json_data.get("thumbnailUrl") {
            Some(Value::String(path)) => format!("https://tv2play.hu{path}"),
            _ => return Err(unsupported("videos without a thumbnail path")),
        };
        let release_date = match json_data.get("uploadedAt") {
            Some(Value::String(date)) => date.replace('.', ""),
            _ => return Err(unsupported("videos without an upload date")),
        };
        let get = |key: &str| json_data.get(key).cloned().unwrap_or(Value::Null);
        let series = |key: &str| {
            json_data
                .get("seriesInfo")
                .and_then(|info| info.get(key))
                .cloned()
                .unwrap_or(Value::Null)
        };
        let mut info = super::dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("series", get("seriesTitle")),
                ("duration", get("length")),
                ("description", get("description")),
                ("thumbnail", thumbnail.into()),
                ("release_date", release_date.into()),
                ("season_number", series("seasonNr")),
                ("episode_number", series("episodeNr")),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            stream.formats,
        );
        info.subtitles = stream.subtitles;
        Ok(Some(ExtractResult::Video(info)))
    }
}

impl Extractor for Tv2HuSeries {
    fn key(&self) -> &'static str {
        SERIES_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        start(SERIES_NAME, url);
        let api = Url::parse(&format!(
            "https://tv2play.hu/api/ribbons/{playlist_id}/0?size=100000"
        ))?;
        let json_data = download_json(client, &api, SERIES_NAME, &playlist_id)?;
        let cards = match json_data.get("cards") {
            None => Vec::new(),
            Some(Value::Array(cards)) => cards.clone(),
            Some(_) => return Err(unsupported("card lists that are not lists")),
        };
        let mut entries = Vec::new();
        for card in &cards {
            let Some(slug) = card.get("slug").filter(|slug| super::nuxt::truthy(slug)) else {
                continue;
            };
            let slug_text = id_text(slug)?;
            let mut entry = super::nrk::url_entry(
                &format!("https://tv2play.hu/{slug_text}"),
                KEY,
                Some(&slug_text),
                None,
                None,
            )?;
            if let Some(marker) = entry.extra.get_mut(crate::TRANSPARENT) {
                marker["entry"]["id"] = slug.clone();
                marker["extractor"] = NAME.into();
            }
            entries.push(entry);
        }
        Ok(Some(with_key(
            super::nrk::playlist_result(url, SERIES_NAME, playlist_id, None, None, entries, false),
            SERIES_KEY,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://tv2play.hu/mintaapak/mintaapak_213_epizod_resz"
        ));
        assert!(!suitable("https://tv2play.hu/szalag/59"));
        assert!(series_suitable(
            "https://tv2play.hu/szalag/59?rendezes=nepszeruseg"
        ));
    }
}
