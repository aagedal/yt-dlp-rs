//! Upstream `RCTIPlusIE` (`yt_dlp/extractor/rcti.py`): an episode, clip,
//! extra, live or missed event of RCTI+ from its visitor token, `/url` and
//! metadata APIs and the HLS master playlist. `RCTIPlusSeriesIE` (program
//! pages) and `RCTIPlusTVIE` (TV and live-channel pages) are not ported and
//! their URLs are not claimed here.
use super::{
    Extractor, dict_info, download_json_note, download_json_note_headers,
    post_json_errnote_optional,
};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::LazyLock;
use url::Url;

pub struct RctiPlus;

const NAME: &str = "RCTIPlus";
const REFERER: &str = "https://www.rctiplus.com/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://www\.rctiplus\.com/(?:programs/\d+?/.*?/)?(?P<type>episode|clip|extra|live-event|missed-event)/(?P<id>\d+)/(?P<display_id>[^/?#&]+)",
    )
    .unwrap()
});

/// `_call_api(url, video_id, note)`: the bearer key's JSON `data` and `meta`.
fn call_api(
    client: &Client,
    url: &Url,
    auth: &str,
    video_id: &str,
    note: &str,
) -> Result<(Value, Value)> {
    let json = download_json_note_headers(
        client,
        url,
        NAME,
        video_id,
        note,
        &[("Authorization", auth)],
    )?;
    let code = json
        .get("status")
        .and_then(|status| status.get("code"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if code != 0 {
        let message = json
            .get("status")
            .and_then(|status| status.get("message_client"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(Error::Reported(format!(
            "[{NAME}] {video_id}: {NAME} said: {message}"
        )));
    }
    Ok((
        json.get("data").cloned().unwrap_or(Value::Null),
        json.get("meta").cloned().unwrap_or(Value::Null),
    ))
}

/// `try_get(video_json, lambda x: x[a] < x[b])`: `None` when either field is
/// missing or the two are not comparable.
fn is_less(json: &Value, a: &str, b: &str) -> Option<bool> {
    match (json.get(a)?, json.get(b)?) {
        (Value::Number(x), Value::Number(y)) => Some(x.as_f64()? < y.as_f64()?),
        (Value::String(x), Value::String(y)) => Some(x < y),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| !value.is_null()).cloned()
}

/// A non-null string field.
fn string_of(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// `dict_get(data, keys)`: the first key with a non-`None` value.
fn first_of(data: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter().find_map(|key| text(data.get(*key)))
}

/// `str_or_none(value)`.
fn str_or_none(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Null => None,
        Value::String(text) => Some(Value::from(text.clone())),
        other => Some(Value::from(other.to_string())),
    }
}

/// `appierid`: upstream's `'23984824_' + str(random.randint(0, 10000000000))`.
fn appier_id() -> String {
    let random = RandomState::new().build_hasher().finish() % 10_000_000_001;
    format!("23984824_{random}")
}

fn conviva_session(client: &Client, display_id: &str, video_url: &str) -> Result<()> {
    let body = json!({
        "t": "CwsSessionHb",
        "cid": "ff84ae928c3b33064b76dec08f12500465e59a6f",
        "clid": "0",
        "sid": 0,
        "seq": 0,
        "caps": 0,
        "sf": 7,
        "sdk": true,
        "url": video_url,
        "sst": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default(),
    });
    let response = post_json_errnote_optional(
        client,
        &Url::parse("https://ff84ae928c3b33064b76dec08f12500465e59a6f.cws.conviva.com/0/wsg")?,
        (NAME, display_id),
        (
            "Creating Conviva session",
            "Failed to create Conviva session",
        ),
        &[],
        body.to_string(),
    )?;
    if let Some(response) = response
        && response.get("err").and_then(Value::as_str) != Some("ok")
    {
        let err = response.get("err").map(|err| match err {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        });
        crate::screen::warning(&format!(
            "[{NAME}] Conviva said: {}",
            err.unwrap_or_else(|| "None".into())
        ));
    }
    Ok(())
}

impl Extractor for RctiPlus {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_type = found["type"].to_owned();
        let video_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_real_initialize`: the visitor's access token.
        let visitor = download_json_note(
            client,
            &Url::parse("https://api.rctiplus.com/api/v1/visitor?platform=web")?,
            NAME,
            &video_id,
            "Fetching authorization key",
        )?;
        let auth = visitor
            .get("data")
            .and_then(|data| data.get("access_token"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();

        let url_version = if video_type == "missed-event" {
            "v2"
        } else {
            "v1"
        };
        let (video_json, _) = call_api(
            client,
            &Url::parse(&format!(
                "https://api.rctiplus.com/api/{url_version}/{video_type}/{video_id}/url?appierid={}",
                appier_id()
            ))?,
            &auth,
            &display_id,
            "Downloading video URL JSON",
        )?;
        let Some(video_url) = string_of(video_json.get("url")) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: An extractor error has occurred. (caused by KeyError('url')); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };

        let mut is_upcoming = is_less(&video_json, "current_date", "live_at");
        if is_upcoming.is_none() {
            is_upcoming = is_less(&video_json, "current_date", "start_date");
        }
        if is_upcoming == Some(true) {
            let message = match video_json.get("live_label").and_then(Value::as_str) {
                Some(label) if !label.is_empty() => {
                    format!("This event will start at {label}.")
                }
                _ => "This event has not started yet.".to_owned(),
            };
            crate::no_formats::raise_expected(NAME, None, Some(video_id.as_str()), &message)?;
        }
        if video_url.contains("akamaized") {
            // For some Akamai-hosted videos a Conviva session must be created first.
            conviva_session(client, &display_id, &video_url)?;
        }

        let (video_meta, meta_paths) = call_api(
            client,
            &Url::parse(&format!(
                "https://api.rctiplus.com/api/v1/{video_type}/{video_id}"
            ))?,
            &auth,
            &display_id,
            "Downloading video metadata",
        )?;

        let image_path = meta_paths
            .get("image_path")
            .and_then(Value::as_str)
            .unwrap_or("https://rstatic.akamaized.net/media/")
            .to_owned();
        let mut thumbnails = Vec::new();
        // 2000px seems to be the highest resolution that can be given.
        for (id, key) in [
            ("portrait_image", "portrait_image"),
            ("landscape_image", "landscape_image"),
        ] {
            if let Some(image) = string_of(video_meta.get(key)) {
                thumbnails.push(json!({"id": id, "url": format!("{image_path}2000{image}")}));
            }
        }

        let manifest = Url::parse(&video_url)?;
        let formats = match super::direct::request_with_headers(
            client,
            &manifest,
            None,
            &[("Referer", REFERER)],
        ) {
            Ok(response) => {
                super::hls::from_response_with(client, response, &manifest, None, true, None, true)?
                    .map(|info| info.formats)
                    .unwrap_or_default()
            }
            Err(Error::Http(error)) if error.status() == Some(StatusCode::FORBIDDEN) => {
                crate::geo::restricted_metadata_available(
                    NAME,
                    &format!("[{NAME}] {display_id}: "),
                    "This video is not available from your location due to geo restriction",
                    &["ID"],
                )?;
                Vec::new()
            }
            Err(error) => return Err(error),
        };
        let mut formats = formats;
        for format in &mut formats {
            let url = format.url.as_str();
            if url.contains("akamaized") || url.contains("cloudfront") {
                // Referer header is required for akamai/cloudfront CDNs.
                format
                    .http_headers
                    .get_or_insert_with(Map::new)
                    .insert("Referer".into(), REFERER.into());
            }
        }

        let is_live = video_type == "live-event" && is_upcoming != Some(true);
        let live_status = if is_upcoming == Some(true) {
            "is_upcoming"
        } else if is_live {
            "is_live"
        } else if video_type == "missed-event" {
            "was_live"
        } else {
            "not_live"
        };
        let fields = vec![
            (
                "id",
                first_of(&video_meta, &["product_id"])
                    .or_else(|| text(video_json.get("product_id")))
                    .unwrap_or(Value::Null),
            ),
            (
                "title",
                first_of(&video_meta, &["title", "name"])
                    .or_else(|| first_of(&video_json, &["content_name", "assets_name"]))
                    .unwrap_or(Value::Null),
            ),
            ("display_id", json!(display_id)),
            (
                "description",
                video_meta.get("summary").cloned().unwrap_or(Value::Null),
            ),
            (
                "timestamp",
                first_of(&video_meta, &["release_date"])
                    .or_else(|| text(video_json.get("start_date")))
                    .unwrap_or(Value::Null),
            ),
            (
                "duration",
                video_meta.get("duration").cloned().unwrap_or(Value::Null),
            ),
            (
                "categories",
                match text(video_meta.get("genre")) {
                    Some(genre) => json!([genre]),
                    None => Value::Null,
                },
            ),
            (
                "average_rating",
                video_meta
                    .get("star_rating")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "series",
                first_of(&video_meta, &["program_title"])
                    .or_else(|| text(video_json.get("program_title")))
                    .unwrap_or(Value::Null),
            ),
            (
                "season_number",
                video_meta.get("season").cloned().unwrap_or(Value::Null),
            ),
            (
                "episode_number",
                video_meta.get("episode").cloned().unwrap_or(Value::Null),
            ),
            (
                "channel",
                video_json.get("tv_name").cloned().unwrap_or(Value::Null),
            ),
            (
                "channel_id",
                str_or_none(video_json.get("tv_id")).unwrap_or(Value::Null),
            ),
            ("formats", Value::Null),
            ("thumbnails", Value::Array(thumbnails)),
            ("is_live", json!(is_live)),
            ("was_live", json!(video_type == "missed-event")),
            // `_fill_common_fields`'s status from the flags, made explicit.
            ("live_status", json!(live_status)),
            (
                "release_timestamp",
                video_json.get("live_at").cloned().unwrap_or(Value::Null),
            ),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        // The thumbnail dicts are `{id, url}` in upstream's order.
        if let Some(Value::String(shape)) = info.extra.get(crate::info_order::INFO_SHAPE).cloned() {
            info.extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                format!("{shape};thumbnail=id,url").into(),
            );
        }
        Ok(Some(ExtractResult::Video(info)))
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.rctiplus.com/programs/616/inews-malam/extra/9438/diungkapkan-melalui-surat-terbuka-ceo-ruangguru-belva-devara-mundur-dari-staf-khusus-presiden"
        ));
        assert!(suitable(
            "https://www.rctiplus.com/missed-event/2507/mou-signing-ceremony-27-juli-2021-1400-wib"
        ));
        assert!(suitable("https://www.rctiplus.com/live-event/1/rcti"));
        assert!(!suitable(
            "https://www.rctiplus.com/programs/829/putri-untuk-pangeran"
        ));
    }

    #[test]
    fn upcoming_comparison_needs_comparable_fields() {
        let json = json!({"current_date": 5, "live_at": 9});
        assert_eq!(is_less(&json, "current_date", "live_at"), Some(true));
        assert_eq!(is_less(&json, "current_date", "start_date"), None);
    }
}
