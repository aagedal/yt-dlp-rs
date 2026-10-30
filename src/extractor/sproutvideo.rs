//! Upstream `SproutVideoIE` and `VidsIoIE` (`yt_dlp/extractor/sproutvideo.py`):
//! a SproutVideo embed's base64 player info (`window.videoInfo`), its signed
//! HLS master (query from the policy signatures, applied to the variants),
//! the `downloads` links and the subtitle tracks. vids.io pages hand off to
//! the SproutVideo embed they contain; password-protected pages are reported.
use super::{Extractor, dict_format, dict_info, download_webpage_note_headers, follow_url};
use crate::{Error, ExtractResult, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub struct SproutVideo;
pub struct VidsIo;

const NAME: &str = "SproutVideo";
const KEY: &str = "SproutVideo";
const VIDS_NAME: &str = "vids.io";
const VIDS_KEY: &str = "VidsIo";
const M3U8_URL_TEMPLATE: &str =
    "https://{base}.videos.sproutvideo.com/{s3_user_hash}/{s3_video_hash}/video/index.m3u8";
const QUALITIES: [&str; 3] = ["hd", "uhd", "source"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://videos\.sproutvideo\.com/embed/(?P<id>[\da-f]+)/[\da-f]+").unwrap()
});

static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe [^>]*\bsrc=["'](?P<url>(?:https?:)?//videos\.sproutvideo\.com/embed/[\da-f]+/[\da-f]+[^"']*)["']"#,
    )
    .unwrap()
});

static PLAYER_INFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:window\.|(?:var|const|let)\s+)(?:dat|(?:player|video)Info|)\s*=\s*["']\s*(?P<json>[A-Za-z0-9+/=]+)\s*["'];"#,
    )
    .unwrap()
});

static VIDS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://[\w-]+\.vids\.io/videos/(?P<id>[\da-f]+)/(?P<display_id>[\w-]+)")
        .unwrap()
});

/// `_match_valid_url` of `SproutVideoIE`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_valid_url` of `VidsIoIE`.
pub fn vids_suitable(url: &str) -> bool {
    VIDS_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("SproutVideo {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `ExtractorError(message)` raised by an extractor (`[IE] ID: message`, with
/// the bug report note because upstream does not expect it).
fn fatal(extractor: &str, video_id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{extractor}] {video_id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(items)) => !items.is_empty(),
    }
}

/// `int_or_none` of a JSON value.
fn int_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64))
            .map_or(Value::Null, Value::from),
        Some(Value::String(text)) => text.trim().parse::<i64>().map_or(Value::Null, Value::from),
        _ => Value::Null,
    }
}

/// `url_or_none` of a JSON value.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    Url::parse(text)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(String::from)
}

/// The `str(value)` of a policy value (the signatures are strings).
fn policy_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `_policy_to_qs(policy, signature_key)`: the signature's parameters (without
/// `CloudFront-`) and the `sessionID`, in order.
fn policy_to_qs(data: &Map<String, Value>, key: &str) -> Result<Vec<(String, String)>> {
    let video_id = data
        .get("videoUid")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let signatures = data
        .get("signatures")
        .and_then(|signatures| signatures.get(key))
        .and_then(Value::as_object)
        .ok_or_else(|| key_error(&video_id, "signatures"))?;
    let mut query: Vec<(String, String)> = signatures
        .iter()
        .map(|(name, value)| {
            (
                name.strip_prefix("CloudFront-").unwrap_or(name).to_owned(),
                policy_text(value),
            )
        })
        .collect();
    let session = data
        .get("sessionID")
        .ok_or_else(|| key_error(&video_id, "sessionID"))?;
    query.push(("sessionID".into(), policy_text(session)));
    Ok(query)
}

/// `urllib.parse.urlencode(query, doseq=True)`.
fn urlencode(query: &[(String, String)]) -> String {
    query
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                super::html5::quote_plus(name),
                super::html5::quote_plus(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// `update_url_query(url, query)` for a URL with the encoded `query` added.
fn with_query(url: &str, query: &str) -> String {
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}{query}")
}

/// `_extract_embed_urls` for vids.io's page: the first embed, `//` made
/// `https:` and its page as the smuggled referer.
fn embed_url(page: &Url, webpage: &str) -> Option<String> {
    let found = EMBED.captures(webpage)?;
    let embed = found["url"].to_owned();
    let embed = match embed.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => embed,
    };
    Some(super::brightcove::smuggle(
        &embed,
        &json!({"referer": page.as_str()}),
    ))
}

/// The page for `display_id`, as `_download_webpage_handle(...,
/// expected_status=403)`: the body and whether the status was 403.
fn vids_page(client: &Client, url: &Url, display_id: &str) -> Result<(String, bool)> {
    crate::screen::screen(&format!("[{VIDS_NAME}] {display_id}: Downloading webpage"));
    let mut response = super::direct::request_expecting(client, url, &[], &[403])?;
    let forbidden = response.status().as_u16() == 403;
    let page_url = response.url().clone();
    let headers = response.headers().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(&headers, body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(display_id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    Ok((super::decode_webpage(&body, &headers), forbidden))
}

impl Extractor for SproutVideo {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let (raw, smuggled) = super::brightcove::unsmuggle(url.as_str());
        let Some(found) = VALID_URL.captures(&raw) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        // `webpage_url` keeps the smuggled input; the request uses the bare URL.
        let page = url;
        let url = Url::parse(&raw)?;
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let referer = smuggled.get("referer").and_then(Value::as_str);
        let webpage = match referer {
            Some(referer) => download_webpage_note_headers(
                client,
                &url,
                NAME,
                &video_id,
                "Downloading webpage",
                &[("Referer", referer)],
            )?,
            None => super::download_webpage(client, &url, NAME, &video_id)?,
        };
        let player = PLAYER_INFO.captures(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract player info; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        // `transform_source=lambda x: base64.b64decode(x).decode()`.
        let payload: String = player["json"]
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/')
            .collect();
        let decoded = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(payload.trim_end_matches('='))
            .map_err(|_| unsupported("player info that is not base64"))?;
        let text =
            String::from_utf8(decoded).map_err(|_| unsupported("player info that is not UTF-8"))?;
        let data: Value =
            serde_json::from_str(&text).map_err(|_| unsupported("player info that is not JSON"))?;
        let Value::Object(data) = data else {
            return Err(unsupported("player info that is not an object"));
        };

        // SproutVideo may send another video's player info (e.g. for an old
        // user-agent).
        let video_uid = data
            .get("videoUid")
            .ok_or_else(|| key_error(&video_id, "videoUid"))?;
        let video_uid_text = policy_text(video_uid);
        if video_id != video_uid_text {
            return Err(fatal(
                NAME,
                &video_id,
                &format!("{NAME} sent the wrong video data ({video_uid_text})"),
            ));
        }

        let headers = [
            ("Accept", "*/*"),
            ("Origin", "https://videos.sproutvideo.com"),
            ("Referer", url.as_str()),
        ];
        let mut formats = Vec::new();
        // HLS extraction is fatal; only attempted when the player info says
        // it is available.
        if truthy(data.get("hls")) {
            let manifest_query = policy_to_qs(&data, "m")?;
            let fragment_query = urlencode(&policy_to_qs(&data, "t")?);
            let key_query = urlencode(&policy_to_qs(&data, "k")?);
            let base = data
                .get("base")
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(&video_id, "base"))?;
            let user_hash = data
                .get("s3_user_hash")
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(&video_id, "s3_user_hash"))?;
            let video_hash = data
                .get("s3_video_hash")
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(&video_id, "s3_video_hash"))?;
            let manifest_url = M3U8_URL_TEMPLATE
                .replace("{base}", base)
                .replace("{s3_user_hash}", user_hash)
                .replace("{s3_video_hash}", video_hash);
            let manifest = Url::parse(&with_query(&manifest_url, &urlencode(&manifest_query)))?;
            let stream = super::extract_m3u8_master_optional_headers_id(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("hls"),
                &headers,
            )
            .ok_or_else(|| unsupported("HLS manifests that fail to download"))?;
            let manifest_encoded = urlencode(&manifest_query);
            for mut format in stream.formats {
                let variant = with_query(format.url.as_str(), &manifest_encoded);
                format.url = Url::parse(&variant)?;
                crate::info_order::mark_format_update_in_place(&mut format, "url");
                format.extra.insert(
                    "extra_param_to_segment_url".into(),
                    fragment_query.clone().into(),
                );
                crate::info_order::mark_format_update(&mut format, "extra_param_to_segment_url");
                format
                    .extra
                    .insert("extra_param_to_key_url".into(), key_query.clone().into());
                crate::info_order::mark_format_update(&mut format, "extra_param_to_key_url");
                formats.push(format);
            }
        }

        if let Some(Value::Object(downloads)) = data.get("downloads") {
            for (format_id, format_url) in downloads {
                let Some(format_url) = url_or_none(Some(format_url)) else {
                    continue;
                };
                let quality = QUALITIES
                    .iter()
                    .position(|name| *name == format_id.as_str())
                    .map_or(-1, |index| index as i64);
                let acodec = if data.get("has_audio") == Some(&Value::Bool(false)) {
                    Value::from("none")
                } else {
                    Value::Null
                };
                formats.push(dict_format(
                    vec![
                        ("format_id", format_id.as_str().into()),
                        ("url", format_url.into()),
                        ("ext", "mp4".into()),
                        ("quality", quality.into()),
                        ("acodec", acodec),
                    ],
                    NAME,
                    &video_id,
                )?);
            }
        }

        let mut subtitles = crate::Subtitles::default();
        if let Some(Value::Array(tracks)) = data.get("subtitleData") {
            for track in tracks {
                let Some(src) = url_or_none(track.get("src")) else {
                    continue;
                };
                let language = match track.get("srclang") {
                    Some(Value::String(language)) => Some(language.clone()),
                    Some(other) => Some(other.to_string()),
                    None => Some("en".to_owned()),
                };
                let ext =
                    super::direct::determine_ext(&src).unwrap_or_else(|| "unknown_video".into());
                let format: crate::SubtitleFormat =
                    serde_json::from_value(json!({"url": src, "ext": ext}))
                        .map_err(|error| Error::Reported(error.to_string()))?;
                match subtitles
                    .entries
                    .iter_mut()
                    .find(|(key, _)| *key == language)
                {
                    Some((_, tracks)) => tracks.push(format),
                    None => subtitles.entries.push((language, vec![format])),
                }
            }
        }

        let title = match data.get("title") {
            Some(Value::String(title)) => Value::from(title.as_str()),
            _ => Value::Null,
        };
        let mut info = dict_info(
            page,
            (NAME, KEY),
            vec![
                ("id", video_id.clone().into()),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                (
                    "http_headers",
                    json!({
                        "Accept": "*/*",
                        "Origin": "https://videos.sproutvideo.com",
                        "Referer": url.as_str(),
                    }),
                ),
                ("title", title),
                ("duration", int_or_none(data.get("duration"))),
                (
                    "thumbnail",
                    url_or_none(data.get("posterframe_url")).map_or(Value::Null, Value::from),
                ),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

impl Extractor for VidsIo {
    fn key(&self) -> &'static str {
        VIDS_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VIDS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(VIDS_NAME);
        crate::screen::screen(&format!(
            "[{VIDS_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let (webpage, forbidden) = vids_page(client, url, &display_id)?;
        if forbidden {
            if super::video_password().is_none() {
                return Err(Error::Reported(format!(
                    "[{VIDS_NAME}] {video_id}: This video is password-protected; use the --video-password option"
                )));
            }
            return Err(unsupported("password-protected vids.io pages"));
        }
        let Some(target) = embed_url(url, &webpage) else {
            return Err(fatal(
                VIDS_NAME,
                &video_id,
                "Unable to extract any SproutVideo embed url",
            ));
        };
        Ok(Some(follow_url(url, target)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://videos.sproutvideo.com/embed/4c9dddb01910e3c9c4/0fc24387c4f24ee3"
        ));
        assert!(suitable(
            "https://videos.sproutvideo.com/embed/119dd8ba121ee0cc98/4ee50c88a343215d?type=hd"
        ));
        assert!(!suitable("https://videos.sproutvideo.com/"));
        assert!(vids_suitable(
            "https://how-to-video.vids.io/videos/799cd8b11c10efc1f0/how-to-video-live-streaming"
        ));
    }

    #[test]
    fn policy_query_matches_upstream_encoding() {
        let data = json!({
            "videoUid": "x",
            "sessionID": "s-1",
            "signatures": {"m": {"CloudFront-Policy": "a~b", "Signature": "c+d/e="}},
        });
        let query = policy_to_qs(data.as_object().unwrap(), "m").unwrap();
        assert_eq!(
            urlencode(&query),
            "Policy=a~b&Signature=c%2Bd%2Fe%3D&sessionID=s-1"
        );
    }
}
