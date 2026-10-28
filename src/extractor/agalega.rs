//! Upstream `AGalegaIE` (`yt_dlp/extractor/agalega.py`): A Galega (CRTVG)
//! videos from the Interactvty content API (a cached anonymous JWT access
//! token): the content data's name, description and image (non-fatal) and
//! the content resources' HLS playlists.
use super::{Extractor, dict_info, extract_m3u8_fatal_master};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::{LazyLock, Mutex};
use url::Url;

pub struct AGalega;

const NAME: &str = "agalega:videos";
const KEY: &str = "AGalega";
const TOKEN_URL: &str = "https://www.agalega.gal/api/fetch-api/jwt/token";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?agalega\.gal/videos/(?:detail/)?(?P<id>[0-9]+)").unwrap()
});

/// `AGalegaBaseIE._access_token`: shared by every extraction of the run.
static ACCESS_TOKEN: Mutex<Option<String>> = Mutex::new(None);

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
    Error::UnsupportedHtml5(format!("A Galega {what} are not supported natively"))
}

/// `_jwt_is_expired(token)`: `jwt_decode_hs256(token)['exp'] - time.time()
/// < 120`.
fn jwt_is_expired(token: &str, video_id: &str) -> Result<bool> {
    let payload = super::podbayfm::jwt_payload(token)
        .ok_or_else(|| unsupported("access tokens that are not decodable JWTs"))?;
    let exp = match payload.get("exp") {
        Some(Value::Number(exp)) => exp.as_f64().unwrap_or_default(),
        None if payload.is_object() => return Err(key_error(video_id, "exp")),
        _ => return Err(unsupported("access tokens without a numeric expiry")),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    Ok(exp - now < 120.0)
}

/// `_refresh_access_token(endpoint)`: the token POST (noted with the
/// endpoint as its ID).
fn refresh_access_token(client: &Client, endpoint: &str, video_id: &str) -> Result<String> {
    // `json.dumps({...}).encode()` without a `Content-Type`: urllib's form
    // default.
    let body =
        r#"{"username": null, "password": null, "client": "crtvg", "checkExistsCookies": false}"#;
    let response = super::post_json_note_headers(
        client,
        &Url::parse(TOKEN_URL)?,
        NAME,
        endpoint,
        "Downloading access token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body.to_owned(),
    )?;
    match response.get("access") {
        Some(Value::String(token)) => Ok(token.clone()),
        Some(_) => Err(unsupported("access tokens that are not strings")),
        None => Err(key_error(video_id, "access")),
    }
}

/// `_call_api(endpoint, display_id, note, fatal, query)`: `None` after the
/// upstream warning for a failed non-fatal request.
fn call_api(
    client: &Client,
    (endpoint, display_id): (&str, &str),
    note: &str,
    fatal: bool,
    query: &[(&str, &str)],
) -> Result<Option<Value>> {
    let cached = ACCESS_TOKEN.lock().unwrap().clone();
    let token = match cached {
        Some(token) if !jwt_is_expired(&token, display_id)? => token,
        _ => {
            let token = refresh_access_token(client, endpoint, display_id)?;
            *ACCESS_TOKEN.lock().unwrap() = Some(token.clone());
            token
        }
    };
    let api = super::update_url_query(
        &Url::parse(&format!(
            "https://api-agalega.interactvty.com/api/2.0/contents/{endpoint}"
        ))?,
        query,
    );
    let authorization = format!("jwtok {token}");
    let headers = [("Authorization", authorization.as_str())];
    match super::download_json_note_headers(client, &api, NAME, display_id, note, &headers) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) if !fatal => {
            crate::screen::warning(&format!(
                "[{NAME}] {display_id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// `traverse_obj(obj, (KEY, ...))` branching over a list or a dict's values.
fn branch(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(map)) => map.values().collect(),
        _ => Vec::new(),
    }
}

impl Extractor for AGalega {
    fn key(&self) -> &'static str {
        KEY
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
        let content_data = call_api(
            client,
            (&format!("content/{video_id}/"), &video_id),
            "Downloading content data",
            false,
            &[(
                "optional_fields",
                "image,is_premium,short_description,has_subtitle",
            )],
        )?;
        let resource_data = call_api(
            client,
            (&format!("content_resources/{video_id}/"), &video_id),
            "Downloading resource data",
            true,
            &[("optional_fields", "media_url")],
        )?
        .unwrap_or(Value::Null);
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        // `('results', ..., 'media_url', {url_or_none})`.
        for result in branch(resource_data.get("results")) {
            let Some(manifest) = result
                .get("media_url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let manifest = Url::parse(&manifest)?;
            let stream =
                extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, Some("hls"), true)?;
            formats.extend(stream.formats);
            super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
        }
        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        // `**traverse_obj(content_data, {...})`: only the found keys.
        let content = content_data.as_ref();
        if let Some(title) = content
            .and_then(|content| content.get("name"))
            .filter(|title| title.is_string())
        {
            fields.push(("title", title.clone()));
        }
        // `(('description', 'short_description'), {str}, any)`.
        if let Some(description) = ["description", "short_description"]
            .iter()
            .filter_map(|key| content.and_then(|content| content.get(*key)))
            .find(|description| description.is_string())
        {
            fields.push(("description", description.clone()));
        }
        if let Some(thumbnail) = content
            .and_then(|content| content.get("image"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            fields.push(("thumbnail", thumbnail.into()));
        }
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.agalega.gal/videos/288664-lr-ninguencheconta"
        ));
        assert!(suitable(
            "https://www.agalega.gal/videos/detail/296152-pulso-activo-7"
        ));
        assert!(!suitable("https://www.agalega.gal/programas/123"));
    }
}
