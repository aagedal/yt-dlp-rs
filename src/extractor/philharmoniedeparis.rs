//! Upstream `PhilharmonieDeParisIE` (`yt_dlp/extractor/philharmoniedeparis.py`):
//! the otoplayer config JSON as one video (a titled config with files) or a
//! playlist of its titled chapters, each with the `mobile`/`desktop` HLS
//! manifests' formats.
use super::{Extractor, download_json};
use crate::{ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct PhilharmonieDeParis;

const NAME: &str = "PhilharmonieDeParis";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:live\.philharmoniedeparis\.fr/(?:[Cc]oncert/|embed(?:app)?/|misc/Playlist\.ashx\?id=)|pad\.philharmoniedeparis\.fr/(?:doc/CIMU/|player\.aspx\?id=)|philharmoniedeparis\.fr/fr/live/concert/|otoplayer\.philharmoniedeparis\.fr/fr/embed/)(?P<id>\d+)",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// `extract_entry(source)` without its ID: the title, the formats and the
/// files' thumbnail, `None` for untitled sources, sources without a
/// `files` dict and sources without formats.
fn extract_entry(
    client: &Client,
    source: &Value,
    video_id: &str,
    page: &Url,
) -> Result<Option<VideoInfo>> {
    let Some(source) = source.as_object() else {
        return Ok(None);
    };
    let Some(title) = source.get("title").filter(|title| truthy(title)) else {
        return Ok(None);
    };
    let Some(files) = source.get("files").and_then(Value::as_object) else {
        return Ok(None);
    };
    let mut seen: Vec<&str> = Vec::new();
    let mut formats = Vec::new();
    for format_id in ["mobile", "desktop"] {
        // `try_get(files, lambda x: x[format_id]['file'], str)`.
        let Some(format_url) = files
            .get(format_id)
            .and_then(|file| file.get("file"))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        if seen.contains(&format_url) {
            continue;
        }
        seen.push(format_url);
        let manifest = Url::parse(format_url).map_err(|_| {
            crate::Error::UnsupportedHtml5(
                "PhilharmonieDeParis manifest URLs that do not parse".into(),
            )
        })?;
        if let Some(info) = super::extract_m3u8_master_optional(
            client,
            &manifest,
            NAME,
            video_id,
            Some("hls"),
            false,
            false,
        ) {
            for mut format in info.formats {
                if format.ext.is_empty() || format.ext == "unknown_video" {
                    format.ext = "mp4".into();
                }
                formats.push(format);
            }
        }
    }
    // `self.get_param('ignore_no_formats')` is not an option upstream sets
    // (it is `ignore_no_formats_error`), so this always applies.
    if formats.is_empty() {
        return Ok(None);
    }
    let first = formats
        .first()
        .map_or_else(|| page.clone(), |format| format.url.clone());
    let mut info = super::direct::build_at(first.clone(), &first, String::new());
    info.formats = formats;
    info.direct = None;
    match title {
        Value::String(title) => info.title = title.clone(),
        other => {
            return Err(crate::Error::UnsupportedHtml5(format!(
                "PhilharmonieDeParis titles that are not strings ({other})"
            )));
        }
    }
    match files.get("thumbnail") {
        Some(Value::String(thumbnail)) => info.metadata.thumbnail = Some(thumbnail.clone()),
        None | Some(Value::Null) => {
            info.extra.insert("thumbnail".into(), Value::Null);
        }
        Some(other) => {
            return Err(crate::Error::UnsupportedHtml5(format!(
                "PhilharmonieDeParis thumbnails that are not strings ({other})"
            )));
        }
    }
    info.webpage_url = page.clone();
    info.extractor = NAME.into();
    info.extractor_key = NAME.into();
    Ok(Some(info))
}

impl Extractor for PhilharmonieDeParis {
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
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let config_url = Url::parse_with_params(
            &format!("https://otoplayer.philharmoniedeparis.fr/fr/config/{video_id}.json"),
            [("id", video_id.as_str()), ("lang", "fr-FR")],
        )
        .expect("valid config URL");
        let config = download_json(client, &config_url, NAME, &video_id)?;
        if let Some(mut info) = extract_entry(client, &config, &video_id, url)? {
            info.id = video_id;
            return Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
                info,
                "dict;title,formats,thumbnail,id",
            ))));
        }
        // `config['chapters']`: an uncaught `KeyError`/`TypeError`.
        let chapters = match config.get("chapters") {
            Some(Value::Array(chapters)) => chapters.clone(),
            None if config.is_object() => {
                return Err(crate::Error::Reported(format!(
                    "{video_id}: An extractor error has occurred. (caused by KeyError('chapters')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            _ => {
                return Err(crate::Error::UnsupportedHtml5(
                    "PhilharmonieDeParis chapters that are not a list".into(),
                ));
            }
        };
        let mut entries = Vec::new();
        for (num, chapter) in chapters.iter().enumerate() {
            let Some(mut entry) = extract_entry(client, chapter, &video_id, url)? else {
                continue;
            };
            entry.id = format!("{video_id}-{}", num + 1);
            entries.push(crate::info_order::mark_info(
                entry,
                "dict;title,formats,thumbnail,id",
            ));
        }
        // `playlist_result(entries, video_id, config.get('title'))`: the
        // title only when truthy.
        let title = config
            .get("title")
            .filter(|title| truthy(title))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            if title.is_some() {
                "playlist-dict;id,title"
            } else {
                "playlist-dict;id"
            }
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: video_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
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
            "https://philharmoniedeparis.fr/fr/live/concert/1129666-danses-symphoniques"
        ));
        assert!(suitable(
            "http://live.philharmoniedeparis.fr/misc/Playlist.ashx?id=1030324&track=&lang=fr"
        ));
        assert!(suitable(
            "https://live.philharmoniedeparis.fr/embedapp/1098406/berlioz.html?lang=fr-FR"
        ));
        assert!(suitable(
            "https://otoplayer.philharmoniedeparis.fr/fr/embed/1098406?lang=fr-FR"
        ));
        assert!(!suitable("https://philharmoniedeparis.fr/fr/live/"));
    }
}
