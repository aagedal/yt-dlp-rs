//! Upstream `EUScreenIE` (`yt_dlp/extractor/euscreen.py`): the item page
//! servlet's two POSTs (the screen ID request, then the same reply with
//! `screenid` renamed) and the `setVideo`/`setData` JSON in the second reply.
use super::{Extractor, dict_format, dict_info};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub struct EuScreen;

const NAME: &str = "EUScreen";

const SERVLET: &str =
    "https://euscreen.eu/lou/LouServlet/domain/euscreenxl/html5application/euscreenxlitem";

const PAYLOAD: &str = "<fsxml><screen><properties><screenId>-1</screenId></properties><capabilities id=\"1\"><properties><platform>Win32</platform><appcodename>Mozilla</appcodename><appname>Netscape</appname><appversion>5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/94.0.4606.71 Safari/537.36</appversion><useragent>Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/94.0.4606.71 Safari/537.36</useragent><cookiesenabled>true</cookiesenabled><screenwidth>784</screenwidth><screenheight>758</screenheight><orientation>undefined</orientation><smt_browserid>Sat, 07 Oct 2021 08:56:50 GMT</smt_browserid><smt_sessionid>1633769810758</smt_sessionid></properties></capabilities></screen></fsxml>";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?euscreen\.eu/item.html\?id=(?P<id>[^&?$/]+)")
        .unwrap()
});
static VIDEO_JSON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"setVideo\((\{.+\})\)\(\$end\$\)put").unwrap());
static METADATA_JSON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"setData\((\{.+\})\)\(\$end\$\)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_download_webpage(url, video_id, data=...)`: a POST of `body`.
fn post_webpage(client: &Client, url: &Url, id: &str, body: String) -> Result<String> {
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading webpage"));
    let mut response = super::direct::post(
        client,
        url,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body,
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: NAME,
            id: id.to_owned(),
            what: "webpage",
            source: Box::new(error),
        },
        error => error,
    })?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    Ok(super::decode_webpage(&body, response.headers()))
}

/// `_parse_json(_search_regex(pattern, info_js, name), video_id,
/// transform_source=js_to_json)` (both fatal).
fn search_json(regex: &Regex, text: &str, name: &str, id: &str) -> Result<Value> {
    let Some(found) = regex.captures(text) else {
        return Err(Error::Reported(format!(
            "[{NAME}] {id}: Unable to extract {name}; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let text = crate::js_to_json::js_to_json(&found[1]);
    crate::json::from_str(&text).map_err(|_| {
        let cause =
            crate::json::python_decode_error_text(&text).map_or_else(String::new, |error| {
                format!(
                    " (caused by JSONDecodeError({}))",
                    crate::batch::py_repr(&error)
                )
            });
        Error::Reported(format!(
            "[{NAME}] {id}: Failed to parse JSON{cause}; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })
}

/// `dict.get(key)` of a JSON object (`None` for other values).
fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object().and_then(|object| object.get(key))
}

/// Python truthiness of an optional JSON value.
fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| super::lnk::truthy(Some(value)))
}

/// `'{}'.format(value)` of a string (other values are not supported).
fn py_format(value: Option<&Value>, default: &str) -> Result<String> {
    match value {
        None => Ok(default.to_owned()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(Error::UnsupportedHtml5(
            "EUScreen summaries that are not strings are not supported natively".into(),
        )),
    }
}

impl Extractor for EuScreen {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let servlet = Url::parse(SERVLET)?;
        let mut itempage = servlet.clone();
        itempage
            .query_pairs_mut()
            .append_pair("actionlist", "itempage")
            .append_pair("id", &video_id);
        let args_for_js_request = post_webpage(client, &itempage, &video_id, PAYLOAD.into())?;
        let info_js = post_webpage(
            client,
            &servlet,
            &video_id,
            args_for_js_request.replace("screenid", "screenId"),
        )?;
        let video_json = search_json(&VIDEO_JSON, &info_js, "Video JSON", &video_id)?;
        let meta_json = search_json(&METADATA_JSON, &info_js, "Metadata JSON", &video_id)?;
        let mut formats = Vec::new();
        if let Some(sources) = get(&video_json, "sources") {
            let Some(sources) = sources.as_array() else {
                return Err(Error::UnsupportedHtml5(
                    "EUScreen sources that are not a list are not supported natively".into(),
                ));
            };
            for source in sources {
                let Some(src) = get(source, "src") else {
                    return Err(Error::Reported(format!(
                        "{video_id}: An extractor error has occurred. (caused by KeyError('src')); {}",
                        crate::BUG_REPORTS_MESSAGE
                    )));
                };
                formats.push(dict_format(vec![("url", src.clone())], NAME, &video_id)?);
            }
        }
        let field = |key: &str| get(&meta_json, key).cloned().unwrap_or(Value::Null);
        let duration = get(&meta_json, "duration")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
            .and_then(|duration| serde_json::to_value(duration).ok())
            .unwrap_or(Value::Null);
        let description = format!(
            "{}\n{}",
            py_format(get(&meta_json, "summaryOriginal"), "")?,
            py_format(get(&meta_json, "summaryEnglish"), "")?
        );
        let or = |first: &str, second: Option<&Value>| {
            truthy(get(&meta_json, first))
                .or(second)
                .cloned()
                .unwrap_or(Value::Null)
        };
        let series = or("series", get(&meta_json, "seriesEnglish"));
        let thumbnail = or("screenshot", get(&video_json, "screenshot"));
        let info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", field("originalTitle")),
                ("alt_title", field("title")),
                ("duration", duration),
                ("description", description.into()),
                ("series", series),
                ("episode", field("episodeNumber")),
                ("uploader", field("provider")),
                ("thumbnail", thumbnail),
                ("formats", Value::Null),
            ],
            formats,
        );
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://euscreen.eu/item.html?id=EUS_0EBCBF356BFC4E12A014023BA41BD98C"
        ));
        assert!(!suitable("https://euscreen.eu/item.html"));
    }
}
