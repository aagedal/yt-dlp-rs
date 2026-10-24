//! Upstream `YoutubeLiveChatFD` (`yt_dlp/downloader/youtube_live_chat.py`):
//! the `live_chat` subtitle of a live stream or its replay, written as one
//! JSON line per chat action. The watch page names the chat's continuation;
//! the chat page and then the InnerTube `live_chat` API return each page of
//! actions and the next continuation.
use super::watch;
use crate::download::{DownloadOptions, DownloadPhase, DownloadProgress};
use crate::{Error, Result};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::io::Write;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

/// Upstream `params['skip_download']`.
pub(crate) static SKIP_DOWNLOAD: AtomicBool = AtomicBool::new(false);

const FD_NAME: &str = "youtube_live_chat";

/// `_YT_INITIAL_DATA_RE`.
static INITIAL_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:window\s*\[\s*["']ytInitialData["']\s*\]|ytInitialData)\s*="#).unwrap()
});

/// A continuation to request: its token, the player offset and its click
/// tracking parameters.
#[derive(Debug, Default, PartialEq)]
struct Next {
    continuation: Option<String>,
    offset: i64,
    click_tracking: Option<String>,
}

fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, key| match value {
        Value::Array(items) => items.get(key.parse::<usize>().ok()?),
        _ => value.get(*key),
    })
}

fn text_at(value: &Value, path: &[&str]) -> Option<String> {
    at(value, path).and_then(Value::as_str).map(str::to_owned)
}

/// One action as `json.dumps(action, ensure_ascii=False)` plus a newline.
fn json_line(value: &Value, output: &mut Vec<u8>) -> Result<()> {
    output.extend(crate::info_json::to_python_json(value)?);
    output.push(b'\n');
    Ok(())
}

/// `parse_actions_replay`: every action, the last replayed offset and, when
/// there was one, the replay continuation.
fn parse_actions_replay(chat: &Value, output: &mut Vec<u8>) -> Result<Next> {
    let mut offset = None;
    for action in chat
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(replay) = action.get("replayChatItemAction") {
            offset = Some(
                replay
                    .get("videoOffsetTimeMsec")
                    .and_then(|value| match value {
                        Value::String(text) => text.trim().parse::<i64>().ok(),
                        Value::Number(number) => number.as_i64(),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        Error::Reported(format!(
                            "[{FD_NAME}] invalid videoOffsetTimeMsec in live chat replay"
                        ))
                    })?,
            );
        }
        json_line(action, output)?;
    }
    let mut next = Next {
        offset: offset.unwrap_or_default(),
        ..Next::default()
    };
    if offset.is_some()
        && let Some(data) = at(
            chat,
            &["continuations", "0", "liveChatReplayContinuationData"],
        )
        .filter(|data| data.is_object())
    {
        next.continuation = text_at(data, &["continuation"]);
        next.click_tracking = text_at(data, &["clickTrackingParams"]);
    }
    Ok(next)
}

/// `try_refresh_replay_beginning`: the unfiltered chat replay (the second
/// view), else the page's own actions.
fn try_refresh_replay_beginning(chat: &Value, output: &mut Vec<u8>) -> Result<Next> {
    let refresh = at(
        chat,
        &[
            "header",
            "liveChatHeaderRenderer",
            "viewSelector",
            "sortFilterSubMenuRenderer",
            "subMenuItems",
            "1",
            "continuation",
            "reloadContinuationData",
        ],
    )
    .filter(|data| data.is_object());
    match refresh {
        Some(data) => Ok(Next {
            continuation: text_at(data, &["continuation"]),
            offset: 0,
            click_tracking: text_at(data, &["trackingParams"]),
        }),
        None => parse_actions_replay(chat, output),
    }
}

/// `dict_get`: the first key's value that is truthy.
fn dict_get<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find(|value| truthy(value))
}

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

const MESSAGE_RENDERERS: [&str; 4] = [
    "liveChatTextMessageRenderer",
    "liveChatPaidMessageRenderer",
    "liveChatMembershipItemRenderer",
    "liveChatPaidStickerRenderer",
];

/// `parse_live_timestamp`: a chat item's `timestampUsec` in milliseconds.
fn parse_live_timestamp(action: &Value) -> Option<i64> {
    let content = dict_get(
        action,
        &[
            "addChatItemAction",
            "addLiveChatTickerItemAction",
            "addBannerToLiveChatCommand",
        ],
    )
    .filter(|value| value.is_object())?;
    let item = dict_get(content, &["item", "bannerRenderer"]).filter(|value| value.is_object())?;
    let mut renderer = dict_get(
        item,
        &[
            MESSAGE_RENDERERS[0],
            MESSAGE_RENDERERS[1],
            MESSAGE_RENDERERS[2],
            MESSAGE_RENDERERS[3],
            "liveChatTickerPaidMessageItemRenderer",
            "liveChatTickerSponsorItemRenderer",
            "liveChatBannerRenderer",
        ],
    )
    .filter(|value| value.is_object())?;
    let parent = at(
        renderer,
        &["showItemEndpoint", "showLiveChatItemEndpoint", "renderer"],
    )
    .filter(|value| value.is_object())
    .or_else(|| renderer.get("contents").filter(|value| value.is_object()));
    if let Some(parent) = parent {
        renderer = dict_get(parent, &MESSAGE_RENDERERS).filter(|value| value.is_object())?;
    }
    // `int_or_none(timestampUsec, 1000)`: integer division of an integer.
    match renderer.get("timestampUsec")? {
        Value::String(text) => text.trim().parse::<i64>().ok(),
        Value::Number(number) => number.as_i64(),
        _ => None,
    }
    .map(|usec| usec.div_euclid(1000))
}

/// `parse_actions_live`: each action wrapped like a replayed one, with its
/// offset from the download's start, then the next continuation after its
/// `timeoutMs`.
fn parse_actions_live(
    chat: &Value,
    start_ms: i64,
    live_offset: &mut i64,
    output: &mut Vec<u8>,
) -> Result<(Next, Option<Duration>)> {
    for action in chat
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(timestamp) = parse_live_timestamp(action) {
            *live_offset = timestamp - start_ms;
        }
        let pseudo = json!({
            "replayChatItemAction": {"actions": [action]},
            "videoOffsetTimeMsec": live_offset.to_string(),
            "isLive": true,
        });
        json_line(&pseudo, output)?;
    }
    let mut next = Next {
        offset: *live_offset,
        ..Next::default()
    };
    let mut wait = None;
    let data = at(
        chat,
        &["continuations", "0", "invalidationContinuationData"],
    )
    .filter(|data| data.is_object())
    .or_else(|| {
        at(chat, &["continuations", "0", "timedContinuationData"]).filter(|data| data.is_object())
    });
    if let Some(data) = data {
        next.continuation = text_at(data, &["continuation"]);
        next.click_tracking = text_at(data, &["clickTrackingParams"]);
        wait = match data.get("timeoutMs") {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse::<i64>().ok().map(|n| n as f64),
            _ => None,
        }
        .map(|ms| Duration::from_secs_f64((ms / 1000.0).max(0.0)));
    }
    Ok((next, wait))
}

/// `extract_yt_initial_data`, else the body as JSON.
fn page_data(body: &str) -> Result<Value> {
    if let Some(data) = super::initial_data_script(body)
        .or_else(|| super::super::search_json(&INITIAL_DATA, body, false).flatten())
        && truthy(&data)
    {
        return Ok(data);
    }
    serde_json::from_str(body)
        .map_err(|error| Error::Reported(format!("[{FD_NAME}] Failed to parse JSON: {error}")))
}

/// One fragment request, retried with `--fragment-retries` on HTTP errors.
fn fetch(
    client: &Client,
    url: &Url,
    body: Option<(&[(&str, String)], &str)>,
    frag_index: u32,
    options: &DownloadOptions,
) -> Result<String> {
    let mut attempt = 0;
    loop {
        let response = match body {
            Some((headers, body)) => {
                let mut request = client
                    .post(url.clone())
                    .header(reqwest::header::ACCEPT_ENCODING, "identity")
                    .body(body.to_owned());
                for (name, value) in headers {
                    request = request.header(*name, value);
                }
                send(request)
            }
            None => send(
                client
                    .get(url.clone())
                    .header(reqwest::header::ACCEPT_ENCODING, "identity"),
            ),
        };
        match response.and_then(|response| response.text().map_err(Error::Http)) {
            Ok(text) => return Ok(text),
            Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. }))
                if attempt < options.fragment_retries =>
            {
                let delay = crate::retry::announce_retry(
                    &error,
                    attempt,
                    options.fragment_retries,
                    Some(frag_index),
                    options.retry_sleep.fragment,
                );
                crate::retry::backoff(attempt, delay.unwrap_or_default());
                attempt += 1;
            }
            Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) if attempt > 0 => {
                return Err(crate::retry::exhausted(error, options.fragment_retries));
            }
            Err(error) => return Err(error),
        }
    }
}

fn send(request: reqwest::blocking::RequestBuilder) -> Result<reqwest::blocking::Response> {
    crate::screen::load_cookies()?;
    let response = request.send();
    crate::screen::flush_traffic();
    crate::retry::check_response(response?)
}

/// `YoutubeLiveChatFD.real_download` into `destination` (published without
/// overwriting); `live` is the `youtube_live_chat` protocol.
pub(crate) fn download(
    client: &Client,
    live: bool,
    subtitle: &crate::SubtitleFormat,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    crate::screen::screen(&format!("[{FD_NAME}] Downloading live chat"));
    if live && !SKIP_DOWNLOAD.load(Ordering::Relaxed) {
        crate::screen::warning(
            "Live chat download runs until the livestream ends. If you wish to download the video simultaneously, run a separate yt-dlp instance",
        );
    }
    crate::screen::screen(&format!("[{FD_NAME}] Total fragments: unknown (live)"));
    let start_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64);
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut output = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-subtitle-")
        .suffix(".part")
        .tempfile_in(parent)?;
    let mut written = 0_u64;
    let mut report = |written: u64, fragment: u32| {
        callback(DownloadProgress {
            attempt: 1,
            downloaded_bytes: written,
            total_bytes: None,
            total_bytes_estimate: None,
            phase: DownloadPhase::Downloading,
            stream_index: None,
            stream_count: None,
            fragment_index: Some(fragment),
            fragment_count: None,
            fragment: None,
        })
    };
    let failed = || Error::Subtitle("live chat download failed".into());

    // Fragment 0: the watch page, naming the chat and the InnerTube config.
    let webpage = fetch(client, &subtitle.url, None, 0, &options)?;
    let Some(data) = super::initial_data_script(&webpage)
        .or_else(|| super::super::search_json(&INITIAL_DATA, &webpage, false).flatten())
    else {
        return Err(failed());
    };
    let mut continuation = text_at(
        &data,
        &[
            "contents",
            "twoColumnWatchNextResults",
            "conversationBar",
            "liveChatRenderer",
            "continuations",
            "0",
            "reloadContinuationData",
            "continuation",
        ],
    );
    if report(0, 0).is_break() {
        return Err(Error::DownloadCancelled);
    }
    let ytcfg = watch::extract_ytcfg(&webpage);
    let (Some(api_key), Some(context)) = (
        ytcfg.get("INNERTUBE_API_KEY").filter(|key| truthy(key)),
        ytcfg
            .get("INNERTUBE_CONTEXT")
            .filter(|context| truthy(context)),
    ) else {
        return Err(failed());
    };
    let api_key = match api_key {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let mut context = context.clone();
    let visitor = text_at(&context, &["client", "visitorData"]);
    let (endpoint, page) = if live {
        ("get_live_chat", "live_chat")
    } else {
        ("get_live_chat_replay", "live_chat_replay")
    };
    let api_url = Url::parse(&format!(
        "https://www.youtube.com/youtubei/v1/live_chat/{endpoint}?key={api_key}"
    ))?;
    let mut headers = watch::web_api_headers(&ytcfg, visitor.as_deref());
    headers.push(("content-type", "application/json".into()));

    let mut frag_index = 0_u32;
    let mut offset = 0_i64;
    let mut live_offset = 0_i64;
    let mut click_tracking: Option<String> = None;
    while let Some(token) = continuation.take() {
        frag_index += 1;
        let body = if frag_index > 1 {
            if let Some(params) = &click_tracking
                && let Value::Object(context) = &mut context
            {
                context.insert(
                    "clickTracking".into(),
                    json!({"clickTrackingParams": params}),
                );
            }
            let mut request = Map::new();
            request.insert("context".into(), context.clone());
            request.insert("continuation".into(), token.clone().into());
            request.insert(
                "currentPlayerState".into(),
                json!({"playerOffsetMs": (offset - 5000).max(0).to_string()}),
            );
            let mut body = crate::info_json::to_python_json(&Value::Object(request))?;
            body.push(b'\n');
            Some(String::from_utf8(body).map_err(|error| Error::Reported(error.to_string()))?)
        } else {
            None
        };
        let text = match &body {
            Some(body) => fetch(
                client,
                &api_url,
                Some((&headers, body)),
                frag_index,
                &options,
            )?,
            None => {
                let mut url = Url::parse(&format!("https://www.youtube.com/{page}"))?;
                url.query_pairs_mut().append_pair("continuation", &token);
                fetch(client, &url, None, frag_index, &options)?
            }
        };
        let data = page_data(&text)?;
        let empty = Value::Object(Map::new());
        let chat = at(&data, &["continuationContents", "liveChatContinuation"])
            .filter(|chat| chat.is_object())
            .unwrap_or(&empty);
        let mut fragment = Vec::new();
        let (next, wait) = if live {
            parse_actions_live(chat, start_ms, &mut live_offset, &mut fragment)?
        } else if frag_index == 1 {
            (try_refresh_replay_beginning(chat, &mut fragment)?, None)
        } else {
            (parse_actions_replay(chat, &mut fragment)?, None)
        };
        if let Some(wait) = wait {
            std::thread::sleep(wait);
        }
        output.write_all(&fragment)?;
        written += fragment.len() as u64;
        if report(written, frag_index).is_break() {
            return Err(Error::DownloadCancelled);
        }
        continuation = next.continuation;
        offset = next.offset;
        click_tracking = next.click_tracking;
    }
    output.flush()?;
    if callback(DownloadProgress {
        attempt: 1,
        downloaded_bytes: written,
        total_bytes: Some(written),
        total_bytes_estimate: None,
        phase: DownloadPhase::Publishing,
        stream_index: None,
        stream_count: None,
        fragment_index: None,
        fragment_count: None,
        fragment: None,
    })
    .is_break()
    {
        return Err(Error::DownloadCancelled);
    }
    crate::download::persist(output, destination, options.overwrite)?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_actions_are_written_with_the_next_continuation() {
        let chat = json!({
            "actions": [
                {"replayChatItemAction": {"actions": [], "videoOffsetTimeMsec": "1500"}},
                {"addLiveChatTickerItemAction": {"item": {}}},
            ],
            "continuations": [{"liveChatReplayContinuationData": {
                "continuation": "NEXT", "clickTrackingParams": "CTP"}}],
        });
        let mut output = Vec::new();
        let next = parse_actions_replay(&chat, &mut output).unwrap();
        assert_eq!(
            next,
            Next {
                continuation: Some("NEXT".into()),
                offset: 1500,
                click_tracking: Some("CTP".into())
            }
        );
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "{\"replayChatItemAction\": {\"actions\": [], \"videoOffsetTimeMsec\": \"1500\"}}\n\
             {\"addLiveChatTickerItemAction\": {\"item\": {}}}\n"
        );
        // Without a replayed offset, the chat ends.
        let chat = json!({"actions": [], "continuations": [{"liveChatReplayContinuationData": {"continuation": "X"}}]});
        assert_eq!(
            parse_actions_replay(&chat, &mut Vec::new())
                .unwrap()
                .continuation,
            None
        );
    }

    #[test]
    fn live_timestamps_come_from_the_message_renderer() {
        let text = json!({"addChatItemAction": {"item": {"liveChatTextMessageRenderer": {"timestampUsec": "1700000000123456"}}}});
        assert_eq!(parse_live_timestamp(&text), Some(1_700_000_000_123));
        let ticker = json!({"addLiveChatTickerItemAction": {"item": {"liveChatTickerPaidMessageItemRenderer": {
            "showItemEndpoint": {"showLiveChatItemEndpoint": {"renderer": {
                "liveChatPaidMessageRenderer": {"timestampUsec": "5000"}}}}}}}});
        assert_eq!(parse_live_timestamp(&ticker), Some(5));
        let banner = json!({"addBannerToLiveChatCommand": {"bannerRenderer": {"liveChatBannerRenderer": {
            "contents": {"liveChatTextMessageRenderer": {"id": "B"}}}}}});
        assert_eq!(parse_live_timestamp(&banner), None);
        assert_eq!(
            parse_live_timestamp(&json!({"markChatItemAsDeletedAction": {}})),
            None
        );
    }

    #[test]
    fn live_actions_are_wrapped_like_replays() {
        let chat = json!({
            "actions": [{"addChatItemAction": {"item": {"liveChatTextMessageRenderer": {"timestampUsec": "3000000"}}}}],
            "continuations": [{"timedContinuationData": {"continuation": "T", "timeoutMs": 250}}],
        });
        let mut offset = 0;
        let mut output = Vec::new();
        let (next, wait) = parse_actions_live(&chat, 1000, &mut offset, &mut output).unwrap();
        assert_eq!(offset, 2000);
        assert_eq!(next.continuation.as_deref(), Some("T"));
        assert_eq!(wait, Some(Duration::from_millis(250)));
        assert!(
            String::from_utf8(output)
                .unwrap()
                .ends_with("\"videoOffsetTimeMsec\": \"2000\", \"isLive\": true}\n")
        );
    }
}
