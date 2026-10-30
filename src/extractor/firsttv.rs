//! Upstream `FirstTVIE` (`yt_dlp/extractor/firsttv.py`): a page's playlist URL
//! and its items' HLS, DASH and MP4 sources with chapters. `FirstTVLiveIE`
//! (a dynamic DASH manifest) is not ported.
use super::{
    Extractor, dict_format, dict_info, direct, download_json, download_webpage,
    extract_m3u8_master_optional, extract_mpd_optional,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct FirstTv;

const NAME: &str = "1tv";
const KEY: &str = "FirstTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?(?:sport)?1tv\.ru/(?:[^/?#]+/)+(?P<id>[^/?#]+)").unwrap()
});
/// `data-playlist-url=(["'])(?P<url>(?:(?!\1).)+)\1`, without a backreference.
static PLAYLIST_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-playlist-url=(?:"([^"\n]+)"|'([^'\n]+)')"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("1tv {what} are not supported natively"))
}

/// `str(value)` for an ID-like JSON value.
fn id_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `parse_qs(url)` values under the first of the keys that has any.
fn item_ids(playlist_url: &Url) -> Vec<String> {
    for key in ["video_id", "videos_ids[]", "news_ids[]"] {
        let values: Vec<String> = playlist_url
            .query_pairs()
            .filter(|(name, value)| name.as_ref() == key && !value.is_empty())
            .map(|(_, value)| value.into_owned())
            .collect();
        if !values.is_empty() {
            return values;
        }
    }
    Vec::new()
}

/// A playlist result keyed as upstream's `playlist_result(...)` with
/// `thumbnail` first.
fn playlist(
    page: &Url,
    id: String,
    title: Option<String>,
    thumbnail: Option<String>,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut extra = Map::new();
    extra.insert(
        "thumbnail".into(),
        thumbnail.map_or(Value::Null, Value::from),
    );
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "playlist-dict;thumbnail,id,title".into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title: title.unwrap_or_default(),
        webpage_url: page.clone(),
        extractor: NAME.into(),
        extractor_key: KEY.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

/// `self._entries(items)`'s dict for one playlist item.
fn entry(client: &Client, page: &Url, item: &Value) -> Result<VideoInfo> {
    let video_id = match item.get("id").filter(|id| super::nuxt::truthy(id)) {
        Some(id) => id_text(id),
        None => id_text(
            item.get("uid")
                .ok_or_else(|| super::clyp::key_error("", "uid"))?,
        ),
    };
    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    let sources = item
        .get("sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|source| {
            let src = source.get("src").and_then(Value::as_str)?;
            super::json_ld::url_or_none(src).map(|src| (source, src))
        });
    for (source, src) in sources {
        let mime = source
            .get("type")
            .and_then(Value::as_str)
            .filter(|mime| !mime.is_empty());
        let ext = match mime {
            Some(mime) => direct::mimetype2ext_or(mime, None),
            None => direct::determine_ext(&src).unwrap_or_default(),
        };
        let src_url = Url::parse(&src).map_err(|_| unsupported("sources that do not parse"))?;
        if ext == "m3u8" {
            if let Some(stream) = extract_m3u8_master_optional(
                client,
                &src_url,
                NAME,
                &video_id,
                Some("hls"),
                true,
                false,
            ) {
                formats.extend(stream.formats);
                super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else if ext == "mpd" {
            if let Some(stream) = extract_mpd_optional(client, &src_url, NAME, &video_id, true) {
                formats.extend(stream.formats);
                super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else {
            let tbr_pattern = Regex::new(&format!(r"_(\d{{3,}})\.{}", regex::escape(&ext)))
                .map_err(|_| unsupported("extensions that do not form a pattern"))?;
            let tbr = tbr_pattern
                .captures(&src)
                .and_then(|found| found[1].parse::<i64>().ok());
            let format_id = match tbr {
                Some(tbr) => format!("http-{ext}-{tbr}"),
                None => format!("http-{ext}"),
            };
            let mut format = dict_format(
                vec![
                    ("url", src.clone().into()),
                    ("ext", ext.clone().into()),
                    ("format_id", format_id.into()),
                    ("tbr", tbr.map_or(Value::Null, Value::from)),
                    // Quality metadata of HTTP formats may be incorrect.
                    ("quality", (-10).into()),
                ],
                NAME,
                &video_id,
            )?;
            // `int_or_none` tbr values print as integers.
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
    }

    let mut fields: Vec<(&str, Value)> = Vec::new();
    if let Some(title) = item.get("title").and_then(Value::as_str) {
        fields.push(("title", title.into()));
    }
    if let Some(thumbnail) = item
        .get("poster")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
    {
        fields.push(("thumbnail", thumbnail.into()));
    }
    let timestamp = super::streamable::int_or_none(item.get("dvr_begin_at"));
    if !timestamp.is_null() {
        fields.push(("timestamp", timestamp));
    }
    if let Some(upload_date) = item
        .get("date_air")
        .and_then(Value::as_str)
        .and_then(|date| crate::dates::unified_strdate(date, false))
    {
        fields.push(("upload_date", upload_date.into()));
    }
    let duration = super::streamable::int_or_none(item.get("duration"));
    if !duration.is_null() {
        fields.push(("duration", duration));
    }
    if let Some(episodes) = item.get("episodes").and_then(Value::as_array) {
        let chapters: Vec<Value> = episodes
            .iter()
            .filter(|episode| !super::clyp::float_or_none(episode.get("from")).is_null())
            .map(|episode| {
                let mut chapter = Map::new();
                let from = super::clyp::float_or_none(episode.get("from"));
                if !from.is_null() {
                    chapter.insert("start_time".into(), from);
                }
                if let Some(name) = episode.get("name").and_then(Value::as_str) {
                    chapter.insert("title".into(), crate::html::unescape_html(name).into());
                }
                let to = super::clyp::float_or_none(episode.get("to"));
                if !to.is_null() {
                    chapter.insert("end_time".into(), to);
                }
                Value::Object(chapter)
            })
            .collect();
        if !chapters.is_empty() {
            fields.push(("chapters", chapters.into()));
        }
    }
    fields.push(("id", video_id.into()));
    fields.push(("formats", Value::Null));
    fields.push(("subtitles", Value::Null));
    let mut info = dict_info(page, (NAME, KEY), fields, formats);
    info.subtitles = subtitles;
    Ok(info)
}

impl Extractor for FirstTv {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
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
        let Some(found) = PLAYLIST_URL.captures(&webpage) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract playlist url; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let relative = found
            .get(1)
            .or_else(|| found.get(2))
            .map_or("", |value| value.as_str());
        let playlist_url = url
            .join(&crate::html::unescape_html(relative))
            .map_err(|_| unsupported("playlist URLs that do not parse"))?;

        let ids = item_ids(&playlist_url);
        let listing = download_json(client, &playlist_url, NAME, &display_id)?;
        let items: Vec<&Value> = listing
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| {
                let Some(uid) = item.get("uid").filter(|uid| super::nuxt::truthy(uid)) else {
                    return false;
                };
                ids.is_empty() || ids.contains(&id_text(uid))
            })
            .collect();
        let mut entries = Vec::new();
        for item in items {
            entries.push(entry(client, url, item)?);
        }

        let title = crate::html::og_search_property(&webpage, "title");
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        Ok(Some(playlist(url, display_id, title, thumbnail, entries)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.1tv.ru/shows/naedine-so-vsemi/vypuski/gost-lyudmila-senchina-naedine-so-vsemi-vypusk-ot-12-02-2015"
        ));
        assert!(suitable("https://www.1tv.ru/news/issue/2016-12-01/14:00"));
        assert!(suitable(
            "https://www.sport1tv.ru/sport/chempionat-rossii-po-figurnomu-kataniyu-2025"
        ));
        assert!(!suitable("https://www.1tv.ru/"));
    }
}
