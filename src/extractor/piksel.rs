//! Upstream `PikselIE` (`yt_dlp/extractor/piksel.py`): a player page's app
//! token, the program API's asset files, HLS manifest and SMIL renditions.
//! Caption tracks are unsupported (upstream lists them as subtitles without
//! an extension).
use super::{
    Extractor, dict_format, dict_info, direct, download_json, download_webpage, update_url_query,
};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Piksel;

const NAME: &str = "Piksel";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:player\.(?:olympusattelecom|vibebyvista)|(?:api|player)\.multicastmedia|(?:api-ovp|player)\.piksel)\.(?:com|tech)|(?:mz-edge\.stream\.co|movie-s\.nhk\.or)\.jp|vidego\.baltimorecity\.gov)/v/(?:refid/(?P<refid>[^/]+)/prefid/)?(?P<id>[\w-]+)",
    )
    .unwrap()
});
static APP_TOKEN: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"clientAPI\s*:\s*"([^"]+)""#).unwrap(),
        Regex::new(r#"data-de-api-key\s*=\s*"([^"]+)""#).unwrap(),
    ]
});
static OD_SEGMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/od/[^/]+/").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `value[key]` when `value` is an object.
fn field<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| key_error(id, key))
}

/// Python's truthiness as `dict_get` uses it: `None`, empty and zero are skipped.
fn dict_get<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find(|value| super::nuxt::truthy(value))
}

/// `int_or_none(value, scale)`.
fn int_or_none(value: Option<&Value>, scale: i64) -> Option<i64> {
    let number = match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64))?,
        Value::String(text) => text.trim().parse::<i64>().ok()?,
        _ => return None,
    };
    Some(number / scale)
}

/// `str(value)` of a JSON id (a string or an integer).
fn id_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `_search_regex([...], webpage, 'app token')`.
fn app_token(webpage: &str, id: &str) -> Result<String> {
    APP_TOKEN
        .iter()
        .find_map(|pattern| pattern.captures(webpage).map(|found| found[1].to_owned()))
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract app token; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

/// `_call_api(app_token, resource, display_id, query, host, fatal)`: the API
/// response object (its `failure` is an error when `fatal`, else a warning).
fn call_api(
    client: &Client,
    host: &Url,
    token: &str,
    resource: &str,
    id: &str,
    query: &[(&str, &str)],
    fatal: bool,
) -> Result<Map<String, Value>> {
    let api = host.join(&format!("/ws/ws_{resource}/api/{token}/mode/json/apiv/5"))?;
    let api = update_url_query(&api, query);
    let body = match download_json(client, &api, NAME, id) {
        Ok(body) => body,
        Err(error) if fatal => return Err(error),
        Err(error) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&error)
            ));
            Value::Null
        }
    };
    let response = match body.get("response") {
        Some(Value::Object(response)) => response.clone(),
        _ => Map::new(),
    };
    let failure = if response.is_empty() {
        Some("Empty response from API".to_owned())
    } else {
        response
            .get("failure")
            .and_then(|failure| failure.get("reason"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    if let Some(failure) = failure {
        if fatal {
            return Err(Error::Reported(format!("[{NAME}] {id}: {failure}")));
        }
        crate::screen::warning(&format!("[{NAME}] {failure}"));
    }
    Ok(response)
}

/// `_process_asset_file`: one `http_url` rendition.
fn asset_format(
    asset_file: &Value,
    asset_type: Option<&str>,
    id: &str,
) -> Result<Option<VideoFormat>> {
    let Some(http_url) = asset_file
        .get("http_url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
    else {
        return Ok(None);
    };
    let vbr = int_or_none(asset_file.get("videoBitrate"), 1024);
    let abr = int_or_none(asset_file.get("audioBitrate"), 1024);
    let tbr = match asset_type {
        Some("video") => vbr.zip(abr).map(|(vbr, abr)| vbr + abr),
        Some("audio") => abr,
        _ => None,
    };
    let format_id = match tbr {
        Some(tbr) => format!("http-{tbr}"),
        None => "http".to_owned(),
    };
    let url = crate::html::unescape_html(http_url);
    let mut format = dict_format(
        vec![
            ("format_id", format_id.into()),
            ("url", url.into()),
            ("vbr", vbr.map_or(Value::Null, Value::from)),
            ("abr", abr.map_or(Value::Null, Value::from)),
            (
                "width",
                int_or_none(asset_file.get("videoWidth"), 1).map_or(Value::Null, Value::from),
            ),
            (
                "height",
                int_or_none(asset_file.get("videoHeight"), 1).map_or(Value::Null, Value::from),
            ),
            (
                "filesize",
                int_or_none(asset_file.get("filesize"), 1).map_or(Value::Null, Value::from),
            ),
            ("tbr", tbr.map_or(Value::Null, Value::from)),
        ],
        NAME,
        id,
    )?;
    crate::info_order::mark_int_rates(&mut format);
    Ok(Some(format))
}

/// `_process_asset_files` over a list (or nothing).
fn asset_formats(
    assets: Option<&Value>,
    asset_type: Option<&str>,
    id: &str,
) -> Result<Vec<VideoFormat>> {
    let Some(Value::Array(assets)) = assets else {
        return Ok(Vec::new());
    };
    let mut formats = Vec::new();
    for asset in assets.iter().filter(|asset| super::nuxt::truthy(asset)) {
        formats.extend(asset_format(asset, asset_type, id)?);
    }
    Ok(formats)
}

/// `_extract_smil_formats(url, video_id, transform_source=..., fatal=False)`.
fn smil_formats(
    client: &Client,
    smil_url: &str,
    transform: &dyn Fn(&str) -> String,
    id: &str,
) -> Result<Vec<VideoFormat>> {
    use std::io::Read;
    let Ok(smil_url) = Url::parse(smil_url) else {
        return Ok(Vec::new());
    };
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading SMIL file"));
    let mut response = match direct::request(client, &smil_url, None) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download SMIL file: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
    };
    let final_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = transform(&super::decode_webpage(&body, response.headers()));
    let smil = super::smil::parse(&text)
        .ok_or_else(|| Error::UnsupportedHtml5("Piksel SMIL files that are not SMIL".into()))?;
    // Upstream's `_extract_smil_formats` returns the formats only.
    let (formats, _subtitles) =
        super::html5::smil_formats_and_subtitles(client, &smil, &final_url, id, NAME)?;
    Ok(formats)
}

impl Extractor for Piksel {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let ref_id = found.name("refid").map(|found| found.as_str().to_owned());
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let token = app_token(&webpage, &display_id)?;
        let query: Vec<(&str, &str)> = match &ref_id {
            Some(ref_id) => vec![("refid", ref_id.as_str()), ("prefid", display_id.as_str())],
            None => vec![("v", display_id.as_str())],
        };
        let response = call_api(client, url, &token, "program", &display_id, &query, true)?;
        let response = Value::Object(response);
        let program = field(
            field(&response, "WsProgramResponse", &display_id)?,
            "program",
            &display_id,
        )?;
        let video_id = field(program, "uuid", &display_id)?
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let video_data = field(program, "asset", &display_id)?;
        let title = field(video_data, "title", &display_id)?.clone();
        let asset_type = dict_get(video_data, &["assetType", "asset_type"]).and_then(Value::as_str);

        let mut formats = asset_formats(video_data.get("assetFiles"), asset_type, &video_id)?;
        if let Some(reference) = video_data.get("referenceFile") {
            formats.extend(asset_format(reference, asset_type, &video_id)?);
        }
        if formats.is_empty()
            && let Some(asset_id) = [video_data, program]
                .into_iter()
                .find_map(|object| {
                    object
                        .get("assetid")
                        .filter(|value| super::nuxt::truthy(value))
                })
                .and_then(id_text)
        {
            // `try_get(_call_api(..., 'asset_file', fatal=False), ...)`.
            let asset_response = call_api(
                client,
                url,
                &token,
                "asset_file",
                &display_id,
                &[("assetid", asset_id.as_str())],
                false,
            )?;
            let assets = asset_response
                .get("WsAssetFileResponse")
                .and_then(|response| response.get("AssetFiles"));
            formats.extend(asset_formats(assets, asset_type, &video_id)?);
        }

        if let Some(m3u8) = dict_get(
            video_data,
            &[
                "m3u8iPadURL",
                "ipadM3u8Url",
                "m3u8AndroidURL",
                "m3u8iPhoneURL",
                "iphoneM3u8Url",
            ],
        )
        .and_then(Value::as_str)
            && let Ok(m3u8) = Url::parse(m3u8)
            && let Some(info) = super::extract_m3u8_master_optional(
                client,
                &m3u8,
                NAME,
                &video_id,
                Some("hls"),
                false,
                false,
            )
        {
            formats.extend(info.formats);
        }

        if let Some(smil) =
            dict_get(video_data, &["httpSmil", "hdSmil", "rtmpSmil"]).and_then(Value::as_str)
        {
            let smil = OD_SEGMENT.replace(smil, "/od/http/").into_owned();
            let nhkworld = ref_id.as_deref() == Some("nhkworld");
            let transform = move |text: &str| {
                let text = text.replace("src=\"/", "src=\"");
                if nhkworld {
                    text.replace("/media\"", "/media/\"")
                } else {
                    text
                }
            };
            formats.extend(smil_formats(client, &smil, &transform, &video_id)?);
        }

        if video_data
            .get("captions")
            .and_then(Value::as_array)
            .is_some_and(|captions| captions.iter().any(|caption| caption.get("url").is_some()))
        {
            return Err(Error::UnsupportedHtml5(
                "Piksel caption tracks are not supported natively".into(),
            ));
        }

        let timestamp = video_data
            .get("dateadd")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", title),
                (
                    "description",
                    video_data
                        .get("description")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                (
                    "thumbnail",
                    video_data
                        .get("thumbnailUrl")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                ("timestamp", timestamp),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("_format_sort_fields", json!(["tbr"])),
            ],
            formats,
        );
        info.subtitles = crate::Subtitles::default();
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://player.piksel.tech/v/ums2867l"));
        assert!(suitable(
            "http://player.piksel.com/v/refid/nhkworld/prefid/nw_vod_v_en_2019_240_20190823233000_02_1566873477"
        ));
        assert!(suitable("https://mz-edge.stream.co.jp/v/abc"));
        assert!(!suitable("https://player.piksel.tech/"));
        assert!(!suitable("https://example.com/v/abc"));
    }

    #[test]
    fn bitrates_scale_like_int_or_none() {
        assert_eq!(int_or_none(Some(&json!(510976)), 1024), Some(499));
        assert_eq!(int_or_none(Some(&json!("49152")), 1024), Some(48));
        assert_eq!(int_or_none(Some(&json!(null)), 1024), None);
    }
}
