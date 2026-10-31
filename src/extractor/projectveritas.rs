//! Upstream `ProjectVeritasIE` (`yt_dlp/extractor/projectveritas.py`, marked
//! broken): the Gatsby `page-data.json` video/post, its Mux asset's HLS
//! playlist and the post's title, date and OpenGraph image.
use super::{Extractor, broken_warning, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct ProjectVeritas;

const NAME: &str = "ProjectVeritas";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?projectveritas\.com/(?P<type>news|video)/(?P<id>[^/?#]+)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `value[key]`'s `KeyError`.
fn key<'a>(value: &'a Value, name: &str, id: &str) -> Result<&'a Value> {
    match value {
        Value::Object(object) => object.get(name).ok_or_else(|| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('{name}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        }),
        _ => Err(Error::UnsupportedHtml5(
            "ProjectVeritas values that are not objects are not supported natively".into(),
        )),
    }
}

/// `traverse_obj`'s non-`None` value at `path`.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .try_fold(value, |value, key| value.get(key))
        .filter(|value| !value.is_null())
}

impl Extractor for ProjectVeritas {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let slug = found["id"].to_owned();
        let kind = found["type"].to_owned();
        broken_warning();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "https://www.projectveritas.com/page-data/{kind}/{slug}/page-data.json"
        ))?;
        let response = download_json(client, &api, NAME, &slug)?;
        let data = key(key(&response, "result", &slug)?, "data", &slug)?;
        // `traverse_obj(data_json, 'video', 'post')`.
        let main = path(data, &["video"])
            .or_else(|| path(data, &["post"]))
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "ProjectVeritas pages without a video or post are not supported natively"
                        .into(),
                )
            })?;
        let id = match key(main, "id", &slug)? {
            Value::String(id) => id.clone(),
            other => other.to_string(),
        };
        let thumbnail = path(main, &["image", "ogImage", "src"]).cloned();
        // `traverse_obj(main_data, 'muxAsset', ('body', 'json', 'content',
        // ..., 'data', 'target', 'fields', 'muxAsset'), get_all=False,
        // expected_type=dict)`.
        let mux = path(main, &["muxAsset"])
            .filter(|value| value.is_object())
            .or_else(|| {
                path(main, &["body", "json", "content"])
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|item| path(item, &["data", "target", "fields", "muxAsset"]))
                    .find(|value| value.is_object())
            })
            .ok_or_else(|| {
                Error::Reported(format!("[{NAME}] {slug}: No video on the provided url."))
            })?;
        let playback = path(mux, &["playbackId"])
            .or_else(|| path(mux, &["en-US", "playbackId"]))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "ProjectVeritas Mux assets without a playback ID are not supported natively"
                        .into(),
                )
            })?;
        let mut info = super::extract_m3u8_fatal(
            client,
            &Url::parse(&format!("https://stream.mux.com/{playback}.m3u8"))?,
            NAME,
            &id,
            None,
            false,
        )?;
        super::hls::without_ext(&mut info);
        let title = key(main, "title", &slug)?.clone();
        let upload_date = main
            .get("date")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_strdate(date, true));
        let thumbnail = match thumbnail {
            Some(Value::String(thumbnail)) => thumbnail.replace("//", ""),
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "ProjectVeritas posts without an OpenGraph image are not supported natively"
                        .into(),
                ));
            }
        };
        info.id = id;
        match title {
            Value::String(title) => info.title = title,
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "ProjectVeritas titles that are not strings are not supported natively".into(),
                ));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        match upload_date {
            Some(date) => info.metadata.upload_date = Some(date),
            None => {
                info.extra.insert("upload_date".into(), Value::Null);
            }
        }
        info.metadata.thumbnail = Some(thumbnail);
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,title,upload_date,thumbnail,formats",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.projectveritas.com/news/exclusive-inside-the-new-york-and-new-jersey-hospitals-battling-coronavirus/"
        ));
        assert!(suitable(
            "https://www.projectveritas.com/video/ilhan-omar-connected-ballot-harvester-in-cash-for-ballots-scheme-car-is-full/"
        ));
        assert!(!suitable("https://www.projectveritas.com/about/"));
    }
}
