//! Upstream `WykopBaseIE` and its `wykop:dig`, `wykop:dig:comment`,
//! `wykop:post` and `wykop:post:comment` classes (`yt_dlp/extractor/wykop.py`):
//! a link, entry or comment from the v3 API as a `url_transparent` result for
//! its embedded video. The anonymous token is requested on every run (upstream
//! caches it on disk and refreshes it after a `403`; not ported).
use super::{Extractor, direct, download_json_headers, post_json_errnote};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct WykopDig;
pub struct WykopDigComment;
pub struct WykopPost;
pub struct WykopPostComment;

const DIG_NAME: &str = "wykop:dig";
const DIG_COMMENT_NAME: &str = "wykop:dig:comment";
const POST_NAME: &str = "wykop:post";
const POST_COMMENT_NAME: &str = "wykop:post:comment";

static DIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?wykop\.pl/link/(?P<id>\d+)").unwrap());
static DIG_COMMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?wykop\.pl/link/(?P<dig_id>\d+)/[^/]+/komentarz/(?P<id>\d+)")
        .unwrap()
});
static POST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?wykop\.pl/wpis/(?P<id>\d+)").unwrap());
static POST_COMMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?wykop\.pl/wpis/(?P<post_id>\d+)/[^/#]+#(?P<id>\d+)").unwrap()
});

/// `WykopDigIE.suitable`: a link URL that is not a comment's.
pub fn dig_suitable(url: &str) -> bool {
    DIG.is_match(url) && !DIG_COMMENT.is_match(url)
}

/// `WykopPostIE.suitable`: an entry URL that is not a comment's.
pub fn post_suitable(url: &str) -> bool {
    POST.is_match(url) && !POST_COMMENT.is_match(url)
}

/// `_get_token`: the frontend's hardcoded anonymous key pair.
fn token(client: &Client, name: &'static str) -> Result<String> {
    let url = Url::parse("https://wykop.pl/api/v3/auth")?;
    let body =
        json!({"data": {"key": "w53947240748", "secret": "d537d9e0a7adc1510842059ae5316419"}});
    let data = post_json_errnote(
        client,
        &url,
        (name, ""),
        (
            "Downloading anonymous auth token",
            "Unable to download JSON metadata",
        ),
        &[("Content-Type", "application/json")],
        body.to_string(),
    )?;
    data.get("data")
        .and_then(|data| data.get("token"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::UnsupportedHtml5(
                "Wykop auth responses without a token are not supported natively".into(),
            )
        })
}

/// `_call_api(path, id)['data']`.
fn call_api(client: &Client, name: &'static str, path: &str, id: &str) -> Result<Value> {
    let bearer = format!("Bearer {}", token(client, name)?);
    let url = Url::parse(&format!("https://wykop.pl/api/v3/{path}"))?;
    let response = download_json_headers(client, &url, name, id, &[("Authorization", &bearer)])?;
    response.get("data").cloned().ok_or_else(|| {
        Error::UnsupportedHtml5(
            "Wykop API responses without data are not supported natively".into(),
        )
    })
}

/// `traverse_obj(data, path, expected_type=...)` as a string.
fn string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |value, key| value.get(*key))
        .and_then(Value::as_str)
}

fn int_at(value: &Value, path: &[&str]) -> Option<i64> {
    path.iter()
        .try_fold(value, |value, key| value.get(*key))
        .and_then(Value::as_i64)
}

/// `_common_data_extract`: the outer `url_transparent` fields and its target.
fn common(data: &Value) -> Result<(String, Vec<(&'static str, Value)>)> {
    let author = string_at(data, &["author", "username"]).map(str::to_owned);
    let target = [&["media", "embed", "url"][..], &["source", "url"][..]]
        .iter()
        .find_map(|path| string_at(data, path).filter(|url| Url::parse(url).is_ok()))
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::UnsupportedHtml5(
                "Wykop entries without an embedded video URL are not supported natively".into(),
            )
        })?;
    let thumbnail = [
        &["media", "photo", "url"][..],
        &["media", "embed", "thumbnail"][..],
    ]
    .iter()
    .find_map(|path| string_at(data, path).filter(|url| Url::parse(url).is_ok()));
    let mut fields: Vec<(&'static str, Value)> = vec![
        (
            "display_id",
            data.get("slug").cloned().unwrap_or(Value::Null),
        ),
        ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
        ("uploader", author.clone().map_or(Value::Null, Value::from)),
        (
            "uploader_id",
            author.clone().map_or(Value::Null, Value::from),
        ),
        (
            "uploader_url",
            author.as_ref().map_or(Value::Null, |author| {
                format!("https://wykop.pl/ludzie/{author}").into()
            }),
        ),
        (
            "timestamp",
            string_at(data, &["created_at"])
                .and_then(crate::dates::parse_iso8601_space)
                .map_or(Value::Null, Value::from),
        ),
        (
            "like_count",
            int_at(data, &["votes", "up"]).map_or(Value::Null, Value::from),
        ),
        (
            "dislike_count",
            int_at(data, &["votes", "down"]).map_or(Value::Null, Value::from),
        ),
        (
            "comment_count",
            int_at(data, &["comments", "count"]).map_or(Value::Null, Value::from),
        ),
        (
            "age_limit",
            Value::from(if data.get("adult").is_some_and(super::nuxt::truthy) {
                18
            } else {
                0
            }),
        ),
        ("tags", data.get("tags").cloned().unwrap_or(Value::Null)),
    ];
    fields.retain(|(_, value)| !value.is_null());
    Ok((target, fields))
}

/// `"{username} - {content}"`, with Python's `None`/`''` for missing values.
fn comment_title(data: &Value) -> String {
    let username = string_at(data, &["author", "username"]).unwrap_or("None");
    let content = data
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("{username} - {content}")
}

/// A `url_transparent` result: `target`'s info with these outer `fields`.
fn transparent(url: &Url, target: String, fields: Vec<(&'static str, Value)>) -> ExtractResult {
    let mut outer = serde_json::Map::new();
    for (key, value) in fields {
        if !value.is_null() {
            outer.insert(key.into(), value);
        }
    }
    let mut follow = direct::build_at(url.clone(), url, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow.extra.insert(
        crate::FOLLOW_TRANSPARENT.into(),
        json!({"url": target, "fields": Value::Object(outer)}),
    );
    ExtractResult::Video(follow)
}

/// The ID-keyed `url_transparent` result for `path`, plus `extra` outer fields.
fn entry(
    client: &Client,
    name: &'static str,
    url: &Url,
    path: &str,
    id: &str,
    extra: impl FnOnce(&Value) -> Vec<(&'static str, Value)>,
) -> Result<ExtractResult> {
    let data = call_api(client, name, path, id)?;
    let (target, mut fields) = common(&data)?;
    fields.extend(extra(&data));
    Ok(transparent(url, target, fields))
}

macro_rules! wykop_extractor {
    ($ty:ident, $name:expr, $suitable:expr) => {
        impl Extractor for $ty {
            fn key(&self) -> &'static str {
                $name
            }

            fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
                Ok(None)
            }

            fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
                if !$suitable(url.as_str()) {
                    return Ok(None);
                }
                self.run(client, url).map(Some)
            }
        }
    };
}

wykop_extractor!(WykopDig, DIG_NAME, dig_suitable);
wykop_extractor!(WykopDigComment, DIG_COMMENT_NAME, |url: &str| DIG_COMMENT
    .is_match(url));
wykop_extractor!(WykopPost, POST_NAME, post_suitable);
wykop_extractor!(WykopPostComment, POST_COMMENT_NAME, |url: &str| {
    POST_COMMENT.is_match(url)
});

impl WykopDig {
    fn run(&self, client: &Client, url: &Url) -> Result<ExtractResult> {
        let found = DIG.captures(url.as_str()).ok_or_else(unmatched)?;
        let id = found["id"].to_owned();
        entry(client, DIG_NAME, url, &format!("links/{id}"), &id, |data| {
            let mut extra = vec![("id", Value::from(id.clone()))];
            extra.push(("title", data.get("title").cloned().unwrap_or(Value::Null)));
            extra.push((
                "description",
                data.get("description").cloned().unwrap_or(Value::Null),
            ));
            extra.push((
                "release_timestamp",
                string_at(data, &["published_at"])
                    .and_then(crate::dates::parse_iso8601_space)
                    .map_or(Value::Null, Value::from),
            ));
            extra
        })
    }
}

impl WykopDigComment {
    fn run(&self, client: &Client, url: &Url) -> Result<ExtractResult> {
        let found = DIG_COMMENT.captures(url.as_str()).ok_or_else(unmatched)?;
        let (dig_id, comment_id) = (found["dig_id"].to_owned(), found["id"].to_owned());
        entry(
            client,
            DIG_COMMENT_NAME,
            url,
            &format!("links/{dig_id}/comments/{comment_id}"),
            &comment_id,
            |data| comment_fields(&comment_id, data),
        )
    }
}

impl WykopPost {
    fn run(&self, client: &Client, url: &Url) -> Result<ExtractResult> {
        let found = POST.captures(url.as_str()).ok_or_else(unmatched)?;
        let id = found["id"].to_owned();
        entry(
            client,
            POST_NAME,
            url,
            &format!("entries/{id}"),
            &id,
            |data| comment_fields(&id, data),
        )
    }
}

impl WykopPostComment {
    fn run(&self, client: &Client, url: &Url) -> Result<ExtractResult> {
        let found = POST_COMMENT.captures(url.as_str()).ok_or_else(unmatched)?;
        let (post_id, comment_id) = (found["post_id"].to_owned(), found["id"].to_owned());
        entry(
            client,
            POST_COMMENT_NAME,
            url,
            &format!("entries/{post_id}/comments/{comment_id}"),
            &comment_id,
            |data| comment_fields(&comment_id, data),
        )
    }
}

/// The `id`, `title` and `description` of a comment or entry.
fn comment_fields(id: &str, data: &Value) -> Vec<(&'static str, Value)> {
    vec![
        ("id", id.into()),
        ("title", comment_title(data).into()),
        (
            "description",
            data.get("content").cloned().unwrap_or(Value::Null),
        ),
    ]
}

fn unmatched() -> Error {
    Error::Reported(format!(
        "[wykop] Unable to match URL; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// URL matching for `--list-extractors`.
pub fn dig_comment_suitable(url: &str) -> bool {
    DIG_COMMENT.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn post_comment_suitable(url: &str) -> bool {
    POST_COMMENT.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        let link = "https://wykop.pl/link/6912923/najbardziej-zrzedliwy-kot";
        let comment = "https://wykop.pl/link/6992589/strollowal/komentarz/114540527/podobna";
        assert!(dig_suitable(link));
        assert!(!dig_suitable(comment));
        assert!(DIG_COMMENT.is_match(comment));
        let entry = "https://wykop.pl/wpis/68893343/kot-koty-smiesznykotek";
        let post_comment = "https://wykop.pl/wpis/70084873/test-test-test#249303979";
        assert!(post_suitable(entry));
        assert!(!post_suitable(post_comment));
        assert!(POST_COMMENT.is_match(post_comment));
    }
}
