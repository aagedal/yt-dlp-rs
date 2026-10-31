//! Upstream `USATodayIE` (`yt_dlp/extractor/usatoday.py`): the page's
//! (`?ajax=true`) `ui-video-data` JSON as a `url_transparent` result of its
//! Brightcove player, overlaid with the page's ID, title, thumbnail,
//! description and duration.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct UsaToday;

const NAME: &str = "USAToday";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?usatoday\.com/(?:[^/]+/)*(?P<id>[^?/#]+)")
        .unwrap()
});

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

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("USA Today {what} are not supported natively"))
}

/// Python `str(value)` (`'%s' % value`) of a JSON scalar.
fn py_str(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(true) => Ok("True".into()),
        Value::Bool(false) => Ok("False".into()),
        Value::Null => Ok("None".into()),
        _ => Err(unsupported("video data values that are not scalars")),
    }
}

impl Extractor for UsaToday {
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
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = super::update_url_query(url, &[("ajax", "true")]);
        let webpage = download_webpage(client, &page, NAME, &display_id)?;
        // `get_element_by_attribute('class', 'ui-video-data', webpage)`.
        let ui_video_data = crate::html_elements::elements_text_and_html_by_attribute(
            "class",
            "ui-video-data",
            &webpage,
        )
        .map_err(|_| unsupported("pages whose video data element does not parse"))?
        .into_iter()
        .next()
        .map(|(text, _)| text)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            Error::Reported(format!("[{NAME}] {display_id}: no video on the webpage"))
        })?;
        let video_data: Value = crate::json::from_str(&ui_video_data)
            .map_err(|_| unsupported("video data that is not JSON"))?;
        let Value::Object(video_data) = video_data else {
            return Err(unsupported("video data that is not an object"));
        };
        // `try_get(video_data, lambda x: x['asset_metadata']['items'], dict)`.
        let item = match video_data
            .get("asset_metadata")
            .and_then(|metadata| metadata.get("items"))
        {
            Some(Value::Object(item)) => item.clone(),
            _ => Map::new(),
        };
        let account = match item.get("brightcoveaccount") {
            Some(account) => py_str(account)?,
            None => "29906170001".into(),
        };
        let brightcove_id = match item
            .get("brightcoveid")
            .filter(|value| super::nuxt::truthy(value))
        {
            Some(brightcove_id) => py_str(brightcove_id)?,
            None => py_str(
                video_data
                    .get("brightcove_id")
                    .ok_or_else(|| key_error(&display_id, "brightcove_id"))?,
            )?,
        };
        let target = format!(
            "http://players.brightcove.net/{account}/default_default/index.html?videoId={brightcove_id}"
        );
        // `str(video_data['id'])`.
        py_str(
            video_data
                .get("id")
                .ok_or_else(|| key_error(&display_id, "id"))?,
        )?;
        let title = video_data
            .get("title")
            .ok_or_else(|| key_error(&display_id, "title"))?;
        if !title.is_string() && !title.is_null() {
            return Err(unsupported("titles that are not strings"));
        }
        let duration = video_data
            .get("length")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
            .map_or(Value::Null, |duration| json!(duration));
        let field = |key: &str| video_data.get(key).cloned().unwrap_or(Value::Null);
        // `{'_type': 'url_transparent', 'url': ..., 'ie_key': 'BrightcoveNew'}`.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({
                "url": target,
                // `process_ie_result` drops a `url_transparent` result's `id`.
                "fields": {
                    "title": title,
                    "thumbnail": field("thumbnail"),
                    "description": field("description"),
                    "duration": duration,
                },
            }),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.usatoday.com/media/cinematic/video/81729424/us-france-warn-syrian-regime-ahead-of-new-peace-talks/"
        ));
        assert!(suitable(
            "https://www.usatoday.com/story/tech/science/2018/08/21/yellowstone-supervolcano-eruption-stop-worrying-its-blow/973633002/"
        ));
        assert!(!suitable("https://www.usatoday.com/"));
    }
}
