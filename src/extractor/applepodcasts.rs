//! Upstream `ApplePodcastsIE` (`yt_dlp/extractor/applepodcasts.py`): an
//! episode page (also when it answers 500) and its `serialized-server-data`
//! share item, else the amp-api catalog episode, authorized with the JSON Web
//! Token found in the page's JS asset (`AppleBaseIE._get_token`).
use super::{Extractor, download_json_headers};
use crate::{Error, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct ApplePodcasts;

const NAME: &str = "apple:podcasts";
const KEY: &str = "ApplePodcasts";
const BASE_URL: &str = "https://podcasts.apple.com";
const JWT_KEY_ID: &str = "C4J7GBP74H";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://podcasts\.apple\.com/(?P<country>[^/?#]+/)?podcast(?:/[^/?#]+){1,2}/?\?(?:[^#]+&)?i=(?P<id>\d+)",
    )
    .unwrap()
});
static JS_ASSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<script [^>]*\bsrc="(/assets/index~[0-9a-f]+\.js)">"#).unwrap());
static SERVER_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<script [^>]*\bid=["']serialized-server-data["'][^>]*>"#).unwrap()
});
/// `AppleBaseIE._jwt_cache`: the token per `_BASE_URL`, for the process.
static JWT_CACHE: LazyLock<Mutex<HashMap<&'static str, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Apple Podcasts {what} are not supported natively"))
}

/// A Python exception escaping `_real_extract`.
fn extractor_error(id: &str, cause: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by {cause}); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `jwt_encode({}, '', headers={'typ': 'JWT', 'alg': 'ES256', 'kid':
/// KEY_ID}).split('.')[0]`.
fn jwt_header() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
        r#"{{"typ":"JWT","alg":"ES256","kid":"{JWT_KEY_ID}"}}"#
    ))
}

/// `_jwt_is_expired`: `jwt_decode_hs256(token)['exp'] - time.time() < 120`.
fn jwt_is_expired(token: &str, id: &str) -> Result<bool> {
    let payload = token.split('.').nth(1).unwrap_or_default();
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let payload: Value = engine
        .decode(payload.trim_end_matches('='))
        .ok()
        .and_then(|decoded| serde_json::from_slice(&decoded).ok())
        .ok_or_else(|| unsupported("tokens whose payload does not decode"))?;
    let exp = match payload.get("exp") {
        Some(Value::Number(exp)) => exp.as_f64().unwrap_or_default(),
        None if payload.is_object() => return Err(extractor_error(id, "KeyError('exp')")),
        _ => return Err(unsupported("tokens without a numeric expiry")),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    Ok(exp - now < 120.0)
}

/// `_get_token(webpage, episode_id)`.
fn get_token(client: &Client, webpage: &str, id: &str) -> Result<String> {
    if let Some(token) = JWT_CACHE.lock().unwrap().get(BASE_URL).cloned()
        && !jwt_is_expired(&token, id)?
    {
        return Ok(token);
    }
    let path = JS_ASSET
        .captures(webpage)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract JS asset path; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    let asset = Url::parse(BASE_URL)?.join(&path)?;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JS asset"));
    let js_code = download_text(client, &asset, &[], &[], id)
        .map_err(|error| errnote(error, id, "Unable to download JS asset"))?;
    // `(["\'])(?P<jwt>HEADER(?:\.[\w-]+){2})\1`.
    let header = regex::escape(&jwt_header());
    let pattern = Regex::new(&format!(
        r#""(?P<double>{header}(?:\.[\w-]+){{2}})"|'(?P<single>{header}(?:\.[\w-]+){{2}})'"#
    ))
    .expect("escaped pattern");
    let token = pattern
        .captures(&js_code)
        .and_then(|found| found.name("double").or_else(|| found.name("single")))
        .map(|token| token.as_str().to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract JSON Web Token; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    JWT_CACHE.lock().unwrap().insert(BASE_URL, token.clone());
    if jwt_is_expired(&token, id)? {
        return Err(Error::Reported(format!(
            "[{NAME}] {id}: The fetched token is already expired; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    Ok(token)
}

/// An HTTP error as upstream's `errnote` text.
fn errnote(error: Error, id: &str, note: &str) -> Error {
    match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => {
            Error::Reported(super::request_errnote_text(NAME, id, note, &error))
        }
        error => error,
    }
}

/// A request's body as text (`expected_status=`; `--dump-pages` like other
/// pages).
fn download_text(
    client: &Client,
    url: &Url,
    headers: &[(&str, &str)],
    expected: &[u16],
    id: &str,
) -> Result<String> {
    use std::io::Read;
    let mut response = super::direct::request_expecting(client, url, headers, expected)?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    Ok(super::decode_webpage(&body, response.headers()))
}

/// `traverse_obj(obj, (*path, {str}))`.
fn text(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| value.is_string()).cloned()
}

/// `{clean_html}` over a string.
fn cleaned(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .map(|text| crate::html::clean_html(text).into())
}

/// `{int_or_none}`.
fn int(value: Option<&Value>) -> Option<Value> {
    Some(super::streamable::int_or_none(value)).filter(|value| !value.is_null())
}

/// `{parse_iso8601}` over a string.
fn iso8601(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map(Value::from)
}

/// `{clean_podcast_url}` over a string.
fn podcast_url(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(super::playerfm::clean_podcast_url)
}

/// `update_url(url, scheme='https')`: `urlparse` and `urlunparse` with the
/// scheme replaced.
fn https(url: &str) -> String {
    let rest = url
        .find(':')
        .filter(|&colon| {
            colon > 0
                && url.as_bytes()[0].is_ascii_alphabetic()
                && url[..colon]
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte))
        })
        .map_or(url, |colon| &url[colon + 1..]);
    format!("https:{rest}")
}

/// `artwork['url'].format(f='jpg', h=..., w=...)`: `None` when a field is
/// missing (`try_call`).
fn artwork(template: &str, fields: &[(&str, String)]) -> Result<Option<String>> {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(char) = chars.next() {
        match char {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            // `ValueError: Single '}' encountered`.
            '}' => return Ok(None),
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(char) => name.push(char),
                        None => return Ok(None),
                    }
                }
                if name.contains([':', '!', '.', '[', '{']) {
                    return Err(unsupported("artwork URLs with format specifications"));
                }
                match fields.iter().find(|(key, _)| *key == name) {
                    Some((_, value)) => out.push_str(value),
                    None => return Ok(None),
                }
            }
            char => out.push(char),
        }
    }
    Ok(Some(out))
}

/// `_extract_podcast_from_webpage`: the share item's `EpisodeLockup` model.
fn from_webpage(webpage: &str, id: &str) -> Option<Vec<(&'static str, Value)>> {
    let server_data = super::search_json(&SERVER_DATA, webpage, false).flatten()?;
    let items = server_data
        .get("data")
        .and_then(|data| data.get(0))
        .and_then(|data| data.get("data"))
        .and_then(|data| data.get("headerButtonItems"))?;
    let items: Vec<&Value> = match items {
        Value::Array(items) => items.iter().collect(),
        Value::Object(items) => items.values().collect(),
        _ => Vec::new(),
    };
    let model = items
        .into_iter()
        .filter(|item| {
            item.get("$kind") == Some(&Value::from("share"))
                && item.get("modelType") == Some(&Value::from("EpisodeLockup"))
        })
        .filter_map(|item| item.get("model"))
        .find(|model| model.is_object())
        .filter(|model| model.as_object().is_some_and(|model| !model.is_empty()))?;
    let stream = model
        .get("playAction")
        .and_then(|action| action.get("episodeOffer"))
        .and_then(|offer| offer.get("streamUrl"));
    let mut fields: Vec<(&str, Value)> = vec![("id", id.into())];
    fields.extend(
        [
            ("title", text(model.get("title"))),
            ("description", cleaned(model.get("summary"))),
            ("url", podcast_url(stream).map(Value::from)),
            ("timestamp", iso8601(model.get("releaseDate"))),
            ("duration", int(model.get("duration"))),
            ("episode", text(model.get("title"))),
            ("episode_number", int(model.get("episodeNumber"))),
            ("series", text(model.get("showTitle"))),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value))),
    );
    // `_og_search_thumbnail(webpage)` (non-fatal).
    let thumbnail = crate::html::og_search_property(webpage, "image");
    if thumbnail.is_none() {
        super::search_warning(NAME, "thumbnail URL");
    }
    fields.push(("thumbnail", thumbnail.map_or(Value::Null, Value::from)));
    fields.push(("vcodec", "none".into()));
    Some(fields)
}

/// `_extract_podcast_from_api`: the catalog episode with its podcast.
fn from_api(
    client: &Client,
    webpage: &str,
    id: &str,
    country: Option<&str>,
) -> Result<Vec<(&'static str, Value)>> {
    let token = get_token(client, webpage, id)?;
    let api = Url::parse(&format!(
        "https://amp-api.podcasts.apple.com/v1/catalog/{}/podcast-episodes/{id}?extend=fullDescription&include=podcast&l=en-US",
        country.unwrap_or("us")
    ))?;
    let authorization = format!("Bearer {token}");
    let response = download_json_headers(
        client,
        &api,
        NAME,
        id,
        &[("Authorization", &authorization), ("Origin", BASE_URL)],
    )?;
    // `['data'][0]`.
    let data = match response.get("data") {
        Some(Value::Array(data)) => data
            .first()
            .ok_or_else(|| extractor_error(id, "IndexError('list index out of range')"))?,
        Some(_) => return Err(unsupported("API data that are not lists")),
        None if response.is_object() => return Err(extractor_error(id, "KeyError('data')")),
        None => return Err(unsupported("API responses that are not objects")),
    };
    let attributes = data.get("attributes");
    let attribute = |key: &str| attributes.and_then(|attributes| attributes.get(key));
    let series = data
        .get("relationships")
        .and_then(|relationships| relationships.get("podcast"))
        .and_then(|podcast| podcast.get("data"))
        .and_then(|data| data.get(0))
        .and_then(|podcast| podcast.get("attributes"))
        .and_then(|attributes| attributes.get("name"));
    let duration = match super::streamable::int_or_none(attribute("durationInMilliseconds")) {
        Value::Number(number) => number
            .as_i64()
            .map(|milliseconds| Value::from(milliseconds.div_euclid(1000))),
        _ => None,
    };
    let mut fields: Vec<(&str, Value)> = vec![("id", id.into())];
    fields.extend(
        [
            ("title", text(attribute("name"))),
            ("description", cleaned(attribute("fullDescription"))),
            (
                "url",
                podcast_url(attribute("assetUrl")).map(|url| https(&url).into()),
            ),
            ("timestamp", iso8601(attribute("releaseDateTime"))),
            ("duration", duration),
            ("episode", text(attribute("name"))),
            ("episode_number", int(attribute("episodeNumber"))),
            ("series", text(series)),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value))),
    );
    // `thumb_info.pop('url').format(f='jpg', **thumb_info)`.
    let artwork_info = attribute("artwork");
    let artwork_get = |key: &str| artwork_info.and_then(|artwork| artwork.get(key));
    let mut format_fields = vec![("f", "jpg".to_owned())];
    for key in ["h", "w"] {
        let source = if key == "h" { "height" } else { "width" };
        if let Some(Value::Number(value)) = int(artwork_get(source)) {
            format_fields.push((key, value.to_string()));
        }
    }
    let thumbnail = match artwork_get("url")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
    {
        Some(template) => artwork(&template, &format_fields)?,
        None => None,
    };
    fields.push(("thumbnail", thumbnail.map_or(Value::Null, Value::from)));
    fields.push(("vcodec", "none".into()));
    Ok(fields)
}

/// The episode dict: its own `url` is its only format.
fn result(page: &Url, id: &str, fields: Vec<(&'static str, Value)>) -> Result<VideoInfo> {
    let Some(media) = fields
        .iter()
        .find(|(key, _)| *key == "url")
        .and_then(|(_, url)| url.as_str())
    else {
        return Err(Error::Reported(format!(
            "[{NAME}] {id}: No video formats found!; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let media = Url::parse(media).map_err(|_| unsupported("episode URLs that are not absolute"))?;
    let ext = super::direct::determine_ext(media.as_str())
        .unwrap_or_else(|| "unknown_video".into())
        .to_lowercase();
    let mut info = super::direct::with_format(
        super::direct::build_at(media.clone(), &media, ext.clone()),
        "",
    );
    for format in &mut info.formats {
        format.ext = ext.clone();
        format.format_id = String::new();
        format.vcodec = Some("none".into());
    }
    info.format_id = None;
    info.webpage_url = page.clone();
    info.direct = None;
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
    let mut keys = Vec::new();
    let mut titled = false;
    for (key, value) in fields {
        keys.push(key);
        match key {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => {
                titled = true;
                info.title = value.as_str().unwrap_or_default().to_owned();
            }
            "url" => {}
            _ => crate::metadata_parser::set_field(&mut info, key, value),
        }
    }
    if !titled {
        info.title = String::new();
        info.extra
            .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!(
            "untyped-dict;{}{}",
            keys.join(","),
            if titled { "" } else { "-untitled" }
        ),
    ))
}

impl Extractor for ApplePodcasts {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        // The group keeps its slash, like upstream (`catalog/us//...`).
        let country = found.name("country").map(|country| country.as_str());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // The page may answer 500 (yt-dlp/yt-dlp#17266).
        crate::screen::screen(&format!("[{NAME}] {id}: Downloading webpage"));
        let webpage =
            download_text(client, url, &[], &[500], &id).map_err(|error| match error {
                Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                    extractor: NAME,
                    id: id.clone(),
                    what: "webpage",
                    source: Box::new(error),
                },
                error => error,
            })?;
        let fields = match from_webpage(&webpage, &id) {
            Some(fields) => fields,
            None => from_api(client, &webpage, &id, country)?,
        };
        result(url, &id, fields).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "https://podcasts.apple.com/us/podcast/urbana-podcast-724-by-david-penn/id1531349107?i=1000748574256",
            "https://podcasts.apple.com/podcast/207-whitney-webb-returns/id1135137367?i=1000482637777",
            "https://podcasts.apple.com/podcast/207-whitney-webb-returns?i=1000482637777",
            "https://podcasts.apple.com/podcast/id1135137367?i=1000482637777",
        ] {
            assert!(suitable(url), "{url}");
        }
        assert!(!suitable(
            "https://podcasts.apple.com/us/podcast/the-tim-dillon-show/id1135137367"
        ));
        let found = VALID_URL
            .captures("https://podcasts.apple.com/us/podcast/x/id1?i=2")
            .unwrap();
        assert_eq!(&found["country"], "us/");
    }

    #[test]
    fn token_header_matches_upstream_jwt_encode() {
        assert_eq!(
            jwt_header(),
            "eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiIsImtpZCI6IkM0SjdHQlA3NEgifQ"
        );
    }

    #[test]
    fn artwork_templates_format_like_python() {
        let fields = [("f", "jpg".to_owned()), ("w", "300".to_owned())];
        assert_eq!(
            artwork("https://a/{w}x{w}bb.{f}?{{x}}", &fields).unwrap(),
            Some("https://a/300x300bb.jpg?{x}".into())
        );
        assert_eq!(artwork("https://a/{h}.{f}", &fields).unwrap(), None);
        assert_eq!(artwork("https://a/}", &fields).unwrap(), None);
        assert!(artwork("https://a/{w:d}", &fields).is_err());
        assert_eq!(https("http://a/b.mp3"), "https://a/b.mp3");
        assert_eq!(https("//a/b.mp3"), "https://a/b.mp3");
    }
}
