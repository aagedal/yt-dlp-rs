//! Upstream `RedGifsIE` (`yt_dlp/extractor/redgifs.py`): a GIF's
//! `api.redgifs.com` metadata (behind a temporary bearer token) and its
//! `gif`/`sd`/`hd` MP4 renditions. The search and user listings
//! (`RedGifsSearchIE`, `RedGifsUserIE`) are not ported.
use super::{Extractor, dict_format, dict_info, download_json_note, download_json_note_headers};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct RedGifs;

const NAME: &str = "RedGifs";
const TOKEN_URL: &str = "https://api.redgifs.com/v2/auth/temporary";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:www\.)?redgifs\.com/(?:watch|ifr)/|thumbs2\.redgifs\.com/)(?P<id>[^-/?#.]+)",
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_FORMATS` in order: format id and its height (`None` for the source).
const FORMATS: [(&str, Option<i64>); 3] = [("gif", Some(250)), ("sd", Some(480)), ("hd", None)];

/// `_fetch_oauth_token`.
fn fetch_token(client: &Client, video_id: &str) -> Result<String> {
    let url = Url::parse(TOKEN_URL)?;
    let auth = download_json_note(client, &url, NAME, video_id, "Fetching temporary token")?;
    match auth.get("token").and_then(Value::as_str) {
        Some(token) if !token.is_empty() => Ok(token.to_owned()),
        _ => Err(Error::Reported(format!(
            "Unable to get temporary token; {}",
            crate::BUG_REPORTS_MESSAGE
        ))),
    }
}

/// `_call_api(ep, video_id, ...)` for `gifs/{id}?views=yes`. Headers follow
/// `_API_HEADERS` plus the per-video `x-customheader`.
fn call_api(client: &Client, token: &str, endpoint: &str, video_id: &str) -> Result<Value> {
    let url = Url::parse(&format!("https://api.redgifs.com/v2/{endpoint}"))?;
    let watch = format!("https://www.redgifs.com/watch/{video_id}");
    let bearer = format!("Bearer {token}");
    let data = download_json_note_headers(
        client,
        &url,
        NAME,
        video_id,
        "Downloading video info",
        &[
            ("referer", "https://www.redgifs.com/"),
            ("origin", "https://www.redgifs.com"),
            ("content-type", "application/json"),
            ("authorization", bearer.as_str()),
            ("x-customheader", watch.as_str()),
        ],
    )?;
    if let Some(error) = data.get("error") {
        return Err(Error::Reported(format!(
            "[{NAME}] {video_id}: RedGifs said: {}",
            error_text(error)
        )));
    }
    Ok(data)
}

/// `f'{data["error"]}'`.
fn error_text(error: &Value) -> String {
    match error {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `int_or_none(value)`.
fn int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `_parse_gif_data`.
fn parse_gif(gif: &Value) -> Result<VideoInfo> {
    let video_id = gif.get("id").and_then(Value::as_str).unwrap_or_default();
    let orig_height = int(gif.get("height"));
    let aspect_ratio = match (gif.get("width").and_then(Value::as_f64), orig_height) {
        (Some(width), Some(height)) if height != 0 => Some(width / height as f64),
        _ => None,
    };

    let mut formats = Vec::new();
    for (rank, (format_id, height)) in FORMATS.into_iter().enumerate() {
        let Some(video_url) = gif
            .get("urls")
            .and_then(|urls| urls.get(format_id))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let Some(orig_height) = orig_height else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: unsupported GIF without a height"
            )));
        };
        let height = orig_height.min(height.unwrap_or(orig_height));
        let width = aspect_ratio
            .filter(|ratio| *ratio != 0.0)
            .map(|ratio| Value::from((height as f64 * ratio) as i64))
            .unwrap_or(Value::Null);
        formats.push(dict_format(
            vec![
                ("url", video_url.into()),
                ("format_id", format_id.into()),
                ("width", width),
                ("height", height.into()),
                ("quality", (rank as i64).into()),
            ],
            NAME,
            video_id,
        )?);
    }

    let tags: Vec<Value> = gif
        .get("tags")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let title = tags
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    let title = if title.is_empty() {
        "RedGifs".to_owned()
    } else {
        title
    };
    let text = |key: &str| gif.get(key).and_then(Value::as_str).map(Value::from);
    let number = |key: &str| int(gif.get(key)).map_or(Value::Null, Value::from);
    let fields = vec![
        ("id", video_id.into()),
        (
            "webpage_url",
            format!("https://redgifs.com/watch/{video_id}").into(),
        ),
        ("extractor_key", NAME.into()),
        ("extractor", NAME.into()),
        ("title", title.into()),
        ("timestamp", number("createDate")),
        ("uploader", text("userName").unwrap_or(Value::Null)),
        ("duration", number("duration")),
        ("view_count", number("views")),
        ("like_count", number("likes")),
        ("categories", Value::Array(tags.clone())),
        ("tags", gif.get("tags").cloned().unwrap_or(Value::Null)),
        ("age_limit", 18.into()),
        ("formats", Value::Null),
    ];
    let page = Url::parse(&format!("https://redgifs.com/watch/{video_id}"))?;
    Ok(dict_info(&page, (NAME, NAME), fields, formats))
}

impl Extractor for RedGifs {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_lowercase();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let token = fetch_token(client, &video_id)?;
        let info = call_api(
            client,
            &token,
            &format!("gifs/{video_id}?views=yes"),
            &video_id,
        )?;
        let gif = info
            .get("gif")
            .ok_or_else(|| Error::Reported(format!("[{NAME}] {video_id}: no GIF data")))?;
        Ok(Some(parse_gif(gif)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.redgifs.com/watch/squeakyhelplesswisent"
        ));
        assert!(suitable(
            "https://thumbs2.redgifs.com/SqueakyHelplessWisent-mobile.mp4#t=0"
        ));
        assert!(suitable(
            "https://www.redgifs.com/ifr/squeakyhelplesswisent"
        ));
        assert!(!suitable("https://www.redgifs.com/browse"));
    }
}
