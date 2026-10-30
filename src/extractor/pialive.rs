//! Upstream `PiaLiveIE` (`yt_dlp/extractor/pialive.py`): PIA LIVE STREAM
//! player pages: the page's program and article codes, ended and upcoming
//! streams, and the player tag list's (a multipart POST) player as a
//! `url_transparent` result; with `getcomments`, the chat room's history.
use super::Extractor;
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct PiaLive;

const NAME: &str = "PiaLive";

/// The transparent result's stand-in for upstream's `__post_extractor`.
const COMMENTS: &str = "__pialive_comments";

const PLAYER_ROOT_URL: &str = "https://player.pia-live.jp/";
const PIA_LIVE_API_URL: &str = "https://api.pia-live.jp";
const API_KEY: &str = "kfds)FKFps-dms9e";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://player\.pia-live\.jp/stream/(?P<id>[\w-]+)").unwrap()
});
static START_INFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<date>\d{4}/\d{1,2}/\d{1,2})\([月火水木金土日]\)(?P<time>\d{2}:\d{2})").unwrap()
});
static HISTORY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"var\s+_history\s*=").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("PIA LIVE {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_extract_var(variable, html)` (fatal): a quoted `var`/`const`/`let`.
fn extract_var(variable: &str, html: &str, video_key: &str) -> Result<String> {
    let pattern = Regex::new(&format!(
        r#"(?:var|const|let)\s+{variable}\s*=\s*(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')"#
    ))
    .map_err(|_| unsupported("variable names"))?;
    pattern
        .captures(html)
        .and_then(|found| found.name("dq").or_else(|| found.name("sq")))
        .map(|value| value.as_str().to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_key}: Unable to extract variable {variable}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

/// `get_element_by_class` / `get_element_html_by_class`: the first
/// element's text and HTML.
fn element_by_class(class: &str, html: &str) -> Result<Option<(String, String)>> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .map(|elements| elements.into_iter().next())
        .map_err(|error| unsupported(&format!("pages whose HTML does not parse ({error})")))
}

/// `multipart_encode({'play_url': ..., 'api_key': ...})` with a random
/// boundary: the body and its `Content-Type`.
fn multipart_encode(fields: &[(&str, &str)]) -> (String, String) {
    use std::hash::{BuildHasher, Hasher};
    loop {
        // `'-' * 15 + str(random.randrange(0x0fffffff, 0xffffffff))`.
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(0);
        let number = 0x0fff_ffff + hasher.finish() % (0xffff_ffff - 0x0fff_ffff);
        let boundary = format!("---------------{number}");
        let mut out = String::new();
        let mut overlaps = false;
        for (name, value) in fields {
            let content =
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n");
            overlaps |= content.contains(&boundary);
            out.push_str(&format!("--{boundary}\r\n{content}"));
        }
        if overlaps {
            continue;
        }
        out.push_str(&format!("--{boundary}--\r\n"));
        return (out, format!("multipart/form-data; boundary={boundary}"));
    }
}

impl Extractor for PiaLive {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_key = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = super::download_webpage(client, url, NAME, &video_key)?;
        let program_code = extract_var("programCode", &webpage, &video_key)?;
        let article_code = extract_var("articleCode", &webpage, &video_key)?;
        // `_html_extract_title` (non-fatal).
        let title = crate::html::html_extract_title(&webpage);
        if title.is_none() {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract title; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        if element_by_class("play-end", &webpage)?.is_some() {
            return Err(Error::Reported(format!(
                "[{NAME}] {program_code}: The video is no longer available"
            )));
        }
        let start_info = element_by_class("play-waiting__date", &webpage)?
            .map(|(text, _)| crate::html::clean_html(&text))
            .filter(|text| !text.is_empty());
        if let Some(start_info) = start_info {
            let Some(found) = START_INFO.captures(&start_info) else {
                crate::screen::warning(&format!(
                    "[{NAME}] unable to extract start_info; {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
                // `date, time = None`.
                return Err(Error::Reported(
                    "cannot unpack non-iterable NoneType object".into(),
                ));
            };
            let release_timestamp_str = format!("{} {} +09:00", &found["date"], &found["time"]);
            let release_timestamp = crate::dates::unified_timestamp(&release_timestamp_str, true);
            crate::no_formats::raise_expected(
                NAME,
                None,
                Some(&video_key),
                &format!("The video will be available after {release_timestamp_str}"),
            )?;
            let info = super::dict_info(
                url,
                (NAME, NAME),
                vec![
                    ("id", program_code.into()),
                    ("title", title.map_or(Value::Null, Value::from)),
                    ("live_status", "is_upcoming".into()),
                    (
                        "release_timestamp",
                        release_timestamp.map_or(Value::Null, Value::from),
                    ),
                ],
                Vec::new(),
            );
            return Ok(Some(info));
        }
        let (payload, content_type) =
            multipart_encode(&[("play_url", &video_key), ("api_key", API_KEY)]);
        let player_tag_list = super::post_json_errnote(
            client,
            &Url::parse(&format!(
                "{PIA_LIVE_API_URL}/perf/player-tag-list/{program_code}"
            ))?,
            (NAME, &program_code),
            (
                "Fetching player tag list",
                "Unable to fetch player tag list",
            ),
            &[
                ("Content-Type", &content_type),
                ("Referer", PLAYER_ROOT_URL),
            ],
            payload.clone(),
        )?;
        let data = player_tag_list
            .get("data")
            .ok_or_else(|| key_error(&video_key, "data"))?;
        let tag = data
            .get("movie_one_tag")
            .ok_or_else(|| key_error(&video_key, "movie_one_tag"))?
            .as_str()
            .ok_or_else(|| unsupported("player tags that are not strings"))?;
        let target = match crate::html_elements::extract_attributes(tag).get("src") {
            Some(Value::String(src)) => src.clone(),
            Some(_) => return Err(unsupported("player tags without a source value")),
            None => return Err(key_error(&video_key, "src")),
        };
        let mut fields = Map::new();
        fields.insert("title".into(), title.map_or(Value::Null, Value::from));
        fields.insert("display_id".into(), program_code.clone().into());
        if super::get_comments() {
            fields.insert(
                COMMENTS.into(),
                json!({
                    "program_code": program_code,
                    "article_code": article_code,
                    "payload": payload,
                    "content_type": content_type,
                }),
            );
        }
        // `url_result(src, url_transparent=True, ...)`.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": Value::Object(fields)}),
        );
        Ok(Some(follow))
    }
}

/// `YoutubeDL.post_extract` of a result carrying the transparent comments
/// marker: `_get_comments(program_code, article_code, ...)` (each request
/// non-fatal) as `comments` and `comment_count`.
pub(crate) fn post_extract(client: &Client, info: &mut VideoInfo) -> Result<()> {
    let Some(marker) = info.extra.shift_remove(COMMENTS) else {
        return Ok(());
    };
    let text = |key: &str| marker[key].as_str().unwrap_or_default().to_owned();
    let (program_code, article_code) = (text("program_code"), text("article_code"));
    let content_type = text("content_type");
    let headers = [
        ("Content-Type", content_type.as_str()),
        ("Referer", PLAYER_ROOT_URL),
    ];
    let mut comments = Vec::new();
    let chat_info = super::post_json_errnote_optional(
        client,
        &Url::parse(&format!(
            "{PIA_LIVE_API_URL}/perf/chat-tag-list/{program_code}/{article_code}"
        ))?,
        (NAME, &program_code),
        ("Fetching chat info", "Unable to fetch chat info"),
        &headers,
        text("payload"),
    )?;
    // `('data', 'chat_one_tag', {extract_attributes}, 'src', {url_or_none})`.
    let chat_room_url = chat_info
        .as_ref()
        .and_then(|chat_info| chat_info.pointer("/data/chat_one_tag"))
        .and_then(Value::as_str)
        .and_then(|tag| {
            crate::html_elements::extract_attributes(tag)
                .get("src")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
        });
    if let Some(chat_room_url) = chat_room_url {
        let comment_page = match super::download_webpage_note_headers(
            client,
            &Url::parse(&chat_room_url)?,
            NAME,
            &program_code,
            "Fetching comment page",
            &[("Referer", PLAYER_ROOT_URL)],
        ) {
            Ok(page) => page,
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{NAME}] {program_code}: Unable to fetch comment page: {}",
                    crate::upstream_error_text(&source)
                ));
                String::new()
            }
            Err(error) => return Err(error),
        };
        if !comment_page.is_empty() {
            comments = history(&comment_page, &program_code)?;
        }
    }
    crate::screen::screen(&format!("[{NAME}] Extracted {} comments", comments.len()));
    let count = comments.len();
    crate::metadata_parser::set_field(info, "comments", Value::Array(comments));
    crate::metadata_parser::set_field(info, "comment_count", count.into());
    Ok(())
}

/// The comment page's `var _history = [...]` (`_search_json`, non-fatal)
/// rows as comments.
fn history(page: &str, program_code: &str) -> Result<Vec<Value>> {
    let rows = match HISTORY.find(page).and_then(|found| {
        // `contains_pattern=r'\[(?s:.+)\]'`: greedy to the last `]`.
        let rest = page[found.end()..].trim_start();
        let end = rest.rfind(']')?;
        rest.starts_with('[').then(|| &rest[..=end])
    }) {
        None => {
            crate::screen::warning(&format!(
                "[{NAME}] {program_code}: unable to extract comment list; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
            return Ok(Vec::new());
        }
        Some(candidate) => serde_json::Deserializer::from_str(candidate)
            .into_iter::<Value>()
            .next()
            .and_then(std::result::Result::ok)
            .ok_or_else(|| unsupported("comment lists that do not parse"))?,
    };
    let rows: Vec<&Value> = match &rows {
        Value::Array(rows) => rows.iter().collect(),
        Value::Object(rows) => rows.values().collect(),
        _ => Vec::new(),
    };
    let mut comments = Vec::new();
    for row in rows {
        // `traverse_obj(row, INDEX)`: list items (dict keys never match).
        let at = |index: usize| match row {
            Value::Array(items) => items.get(index).filter(|value| !value.is_null()),
            _ => None,
        };
        let mut comment = Map::new();
        if let Some(timestamp) =
            at(0).filter(|value| value.is_i64() || value.is_u64() || value.is_boolean())
        {
            comment.insert("timestamp".into(), timestamp.clone());
        }
        // `{lambda x: x == 2}` also runs on a missing item.
        let is_uploader = at(1).is_some_and(|value| value.as_f64() == Some(2.0));
        comment.insert("author_is_uploader".into(), is_uploader.into());
        if let Some(author) = at(2).filter(|value| value.is_string()) {
            comment.insert("author".into(), author.clone());
        }
        if let Some(text) = at(3).filter(|value| value.is_string()) {
            comment.insert("text".into(), text.clone());
        }
        match at(4) {
            None => {}
            Some(Value::String(id)) => {
                comment.insert("id".into(), id.clone().into());
            }
            Some(Value::Number(id)) if id.is_i64() || id.is_u64() => {
                comment.insert("id".into(), id.to_string().into());
            }
            Some(_) => return Err(unsupported("comment IDs that are not strings or integers")),
        }
        comments.push(Value::Object(comment));
    }
    Ok(comments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://player.pia-live.jp/stream/4JagFBEIM14s_hK9aXHKf3k3"
        ));
        assert!(!suitable("https://player.pia-live.jp/"));
    }

    #[test]
    fn multipart_bodies_follow_upstream() {
        let (body, content_type) = multipart_encode(&[("a", "1")]);
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        assert_eq!(
            body,
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n1\r\n--{boundary}--\r\n"
            )
        );
    }
}
