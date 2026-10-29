//! Upstream `GaiaIE` (`yt_dlp/extractor/gaia.py`): a video's feature or
//! preview media through the `brooklyn.gaia.com` pathinfo, node and media
//! APIs (the HLS master's formats and the text tracks), with the `auth`
//! cookie's JWT when the browser has a session. Username and password logins
//! are not supported natively.
use super::clyp::key_error;
use super::{
    Extractor, dict_info, download_json, download_json_headers, extract_m3u8_fatal_master,
    ignore_subtitles,
};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Gaia;

const NAME: &str = "Gaia";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?gaia\.com/video/(?P<id>[^/?]+).*?\bfullplayer=(?P<type>feature|preview)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Gaia {what} are not supported natively"))
}

/// `self._jwt` from the `auth` cookie (`_real_initialize`): the cookie is
/// URL-decoded JSON whose `jwt` is the session token.
fn cookie_jwt() -> Option<String> {
    let raw = crate::cookies::get_cookie("https://www.gaia.com/", "auth")?;
    let decoded = percent_encoding::percent_decode_str(&raw)
        .decode_utf8()
        .ok()?;
    let auth: Value = serde_json::from_str(&decoded).ok()?;
    auth.get("jwt")?.as_str().map(str::to_owned)
}

/// `_get(...)`-style lookups whose missing keys are a Python `KeyError`.
fn field<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| key_error(id, key))
}

/// `try_get(obj, lambda x: x[key][0][value_key])`.
fn field_value<'a>(fields: &'a Value, key: &str, value_key: &str) -> Option<&'a Value> {
    fields.get(key)?.get(0)?.get(value_key)
}

/// `str(value)` of an ID; `None` values stay `None`.
fn id_text(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => Value::from(text.as_str()),
        Some(Value::Number(number)) => Value::from(number.to_string()),
        _ => Value::Null,
    }
}

/// `strip_or_none(value)`.
fn strip_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => Value::from(text.trim()),
        _ => Value::Null,
    }
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Value {
    super::streamable::int_or_none(value)
}

impl Extractor for Gaia {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_NETRC_MACHINE = 'gaia'`: the credentials are only used for logins,
    /// which are not supported natively (see `perform_login`).
    fn netrc_machine(&self) -> Option<&'static str> {
        Some("gaia")
    }

    fn performs_login(&self) -> bool {
        true
    }

    /// `_perform_login` is a no-op with a session JWT. Otherwise it would
    /// POST the credentials to `auth.gaia.com`, which is not ported.
    fn perform_login(
        &self,
        _client: &Client,
        _session: &super::login::Session,
        _username: &str,
        _password: &str,
    ) -> Result<()> {
        if cookie_jwt().is_some() {
            return Ok(());
        }
        Err(unsupported("username and password logins"))
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let display_id = found["id"].to_owned();
        let vtype = found["type"].to_owned();
        let jwt = cookie_jwt();

        let pathinfo = download_json(
            client,
            &Url::parse_with_params(
                "https://brooklyn.gaia.com/pathinfo",
                &[("path", format!("video/{display_id}"))],
            )?,
            NAME,
            &display_id,
        )?;
        let node_id = field(&pathinfo, "id", &display_id)?
            .as_i64()
            .ok_or_else(|| unsupported("node IDs that are not integers"))?;
        let node = download_json(
            client,
            &Url::parse(&format!("https://brooklyn.gaia.com/node/{node_id}"))?,
            NAME,
            &node_id.to_string(),
        )?;
        let vdata = field(&node, &vtype, &display_id)?;
        let media_id = id_text(Some(field(vdata, "nid", &display_id)?));
        let media_id = media_id.as_str().unwrap_or_default().to_owned();
        let title = field(&node, "title", &display_id)?.clone();

        let media_url = Url::parse(&format!("https://brooklyn.gaia.com/media/{media_id}"))?;
        let bearer = jwt.as_ref().map(|jwt| format!("Bearer {jwt}"));
        let media = match &bearer {
            Some(bearer) => download_json_headers(
                client,
                &media_url,
                NAME,
                &media_id,
                &[("Authorization", bearer.as_str())],
            )?,
            None => download_json(client, &media_url, NAME, &media_id)?,
        };
        let hls = field(
            field(&media, "mediaUrls", &display_id)?,
            "bcHLS",
            &display_id,
        )?
        .as_str()
        .ok_or_else(|| unsupported("HLS manifests that are not strings"))?;
        let manifest = Url::parse(hls)?;
        let mut stream = extract_m3u8_fatal_master(client, &manifest, NAME, &media_id, None, true)?;
        // `_extract_m3u8_formats` drops the manifest's subtitle tracks.
        ignore_subtitles(&mut stream, NAME, "HLS");

        let mut subtitles = Subtitles::default();
        let text_tracks = media.get("textTracks").cloned().unwrap_or(Value::Null);
        for key in ["captions", "subtitles"] {
            let Some(tracks) = text_tracks.get(key).and_then(Value::as_object) else {
                continue;
            };
            for (lang, sub_url) in tracks {
                let sub_url = sub_url
                    .as_str()
                    .ok_or_else(|| unsupported("subtitle URLs that are not strings"))?;
                let url = Url::parse(sub_url)?;
                // Processing names an untyped track by its URL's extension.
                let ext =
                    super::direct::determine_ext(sub_url).unwrap_or_else(|| "unknown_video".into());
                let track = crate::SubtitleFormat {
                    name: None,
                    url,
                    ext,
                    protocol: None,
                    fragments: Vec::new(),
                    extra: Map::new(),
                };
                let lang = Some(lang.clone());
                match subtitles.entries.iter_mut().find(|(key, _)| *key == lang) {
                    Some((_, tracks)) => tracks.push(track),
                    None => subtitles.entries.push((lang, vec![track])),
                }
            }
        }

        let fivestar = node.get("fivestar").cloned().unwrap_or(Value::Null);
        let fields = node.get("fields").cloned().unwrap_or(Value::Null);
        let get_field_value = |key: &str| field_value(&fields, key, "value").cloned();
        // `strip_or_none(get_field_value('body') or get_field_value('teaser'))`.
        let description = {
            let body = get_field_value("body");
            let body = body.as_ref().filter(|value| super::nuxt::truthy(value));
            let teaser = get_field_value("teaser");
            let chosen = body.or(teaser.as_ref());
            strip_or_none(chosen)
        };
        let series = node
            .get("series")
            .and_then(|series| series.get("title"))
            .and_then(Value::as_str)
            .map_or(Value::Null, Value::from);
        let count = |key: &str| fivestar.get(key).and_then(|entry| entry.get("value"));
        let info_fields = vec![
            ("id", Value::from(media_id.clone())),
            ("display_id", Value::from(display_id.clone())),
            ("title", title),
            ("formats", Value::Null),
            ("description", description),
            ("timestamp", int_or_none(node.get("created"))),
            ("subtitles", Value::Null),
            ("duration", int_or_none(vdata.get("duration"))),
            ("like_count", int_or_none(count("up_count"))),
            ("dislike_count", int_or_none(count("down_count"))),
            ("comment_count", int_or_none(node.get("comment_count"))),
            ("series", series),
            (
                "season_number",
                int_or_none(get_field_value("season").as_ref()),
            ),
            (
                "season_id",
                id_text(field_value(&fields, "series_nid", "nid")),
            ),
            (
                "episode_number",
                int_or_none(get_field_value("episode").as_ref()),
            ),
        ];
        let mut info = dict_info(url, (NAME, NAME), info_fields, stream.formats);
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
            "https://www.gaia.com/video/connecting-universal-consciousness?fullplayer=feature"
        ));
        assert!(suitable(
            "https://www.gaia.com/video/connecting-universal-consciousness?fullplayer=preview"
        ));
        assert!(!suitable(
            "https://www.gaia.com/video/connecting-universal-consciousness"
        ));
    }
}
