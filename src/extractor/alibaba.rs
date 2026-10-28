//! Upstream `AlibabaIE` (`yt_dlp/extractor/alibaba.py`): Alibaba product
//! videos from the page's `window.detailData` product: its first video media
//! item's ID, duration, cover and `videoUrl` formats, with the product's
//! subject as the title.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Alibaba;

const NAME: &str = "Alibaba";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?alibaba\.com/product-detail/[\w-]+_(?P<id>\d+)\.html").unwrap()
});
static DETAIL_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.detailData\s*=\s*").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `{int_or_none}` of a JSON value.
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

fn values(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    }
}

fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

impl Extractor for Alibaba {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
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
        let detail = super::search_json(&DETAIL_DATA, &webpage, false)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract detail data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Alibaba detail data that do not parse are not supported natively".into(),
                )
            })?;
        let product = detail
            .get("globalData")
            .ok_or_else(|| key_error(&display_id, "globalData"))?
            .get("product")
            .ok_or_else(|| key_error(&display_id, "product"))?;
        // `('mediaItems', lambda _, v: v['type'] == 'video' and v['videoId'], any)`.
        let media = values(product.get("mediaItems")).into_iter().find(|item| {
            item.get("type").and_then(Value::as_str) == Some("video")
                && item.get("videoId").is_some_and(super::nuxt::truthy)
        });
        let mut fields: Vec<(&str, Value)> = Vec::new();
        let mut formats = Vec::new();
        if let Some(media) = media {
            // `('videoId', {int}, {str_or_none})`: integer IDs only.
            if let Some(Value::Number(id)) = media.get("videoId")
                && (id.is_i64() || id.is_u64())
            {
                fields.push(("id", id.to_string().into()));
            }
            if let Some(duration) = int_or_none(media.get("duration")) {
                fields.push(("duration", duration.into()));
            }
            if let Some(thumbnail) = url_or_none(media.get("videoCoverUrl")) {
                fields.push(("thumbnail", thumbnail.into()));
            }
            let id = fields
                .first()
                .and_then(|(_, id)| id.as_str())
                .unwrap_or(&display_id)
                .to_owned();
            for source in values(media.get("videoUrl")) {
                let Some(video_url) = url_or_none(source.get("videoUrl")) else {
                    continue;
                };
                let mut format = vec![("url", Value::from(video_url))];
                match source.get("definition") {
                    None | Some(Value::Null) => {}
                    Some(Value::String(definition)) => {
                        format.push(("format_id", definition.clone().into()));
                    }
                    Some(Value::Number(definition)) => {
                        format.push(("format_id", definition.to_string().into()));
                    }
                    Some(_) => {
                        return Err(Error::UnsupportedHtml5(
                            "Alibaba definitions that are not scalars are not supported natively"
                                .into(),
                        ));
                    }
                }
                for (key, source_key) in [
                    ("tbr", "bitrate"),
                    ("width", "width"),
                    ("height", "height"),
                    ("filesize", "length"),
                ] {
                    if let Some(value) = int_or_none(source.get(source_key)) {
                        format.push((key, value.into()));
                    }
                }
                let mut format = dict_format(format, NAME, &id)?;
                crate::info_order::mark_int_rates(&mut format);
                formats.push(format);
            }
            fields.push(("formats", Value::Null));
        }
        if fields.first().is_none_or(|(key, _)| *key != "id") {
            return Err(Error::Reported(format!(
                "[{NAME}] Missing \"id\" field in extractor result; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let title = product.get("subject").filter(|title| title.is_string());
        fields.push(("title", title.cloned().unwrap_or(Value::Null)));
        fields.push(("display_id", display_id.into()));
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.alibaba.com/product-detail/Kids-Water-Slide_1601271126969.html"
        ));
        assert!(!suitable("https://www.alibaba.com/product-detail/x.html"));
    }
}
