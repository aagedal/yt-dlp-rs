//! Upstream `MediciIE` (`yt_dlp/extractor/medici.py`): a medici.tv or
//! edu.medici.tv performance's `satie` movie-file API (the playout HLS and its
//! subtitle tracks, the title, subtitle, picture and publish date), with the
//! `auth._token.mAuth` cookie as the bearer token when the browser has one.
use super::json_ld::url_or_none;
use super::{
    Extractor, dict_info, download_json_headers, download_webpage_note, extract_m3u8_fatal_master,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Medici;

const NAME: &str = "Medici";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?P<sub>www|edu)\.)?medici\.tv/[a-z]{2}/[\w.-]+/(?P<id>[^/?#&]+)")
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `data[key]` for a KeyError-style lookup (`KeyError` is reported against
/// the display ID).
fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `traverse_obj(data, ('video', key))` as a truthy flag.
fn video_flag(data: &Value, key: &str) -> bool {
    data.get("video")
        .and_then(|video| video.get(key))
        .is_some_and(super::nuxt::truthy)
}

impl Extractor for Medici {
    fn key(&self) -> &'static str {
        NAME
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
        let subdomain = if found.name("sub").is_some_and(|sub| sub.as_str() == "edu") {
            "edu-"
        } else {
            ""
        };
        // The CSRF token cookie is set by the page.
        download_webpage_note(
            client,
            url,
            NAME,
            &display_id,
            "Requesting CSRF token cookie",
        )?;

        let host = url.host_str().unwrap_or_default();
        let origin = format!("https://{host}");
        let referer = format!("{origin}/");
        let authorization =
            crate::cookies::get_cookie(url.as_str(), "auth._token.mAuth").map(|token| {
                percent_encoding::percent_decode_str(&token)
                    .decode_utf8_lossy()
                    .into_owned()
            });
        let mut headers = vec![
            ("Device-Type", "web"),
            ("Origin", origin.as_str()),
            ("Referer", referer.as_str()),
            ("Accept", "application/json, text/plain, */*"),
        ];
        if let Some(authorization) = &authorization {
            headers.push(("Authorization", authorization.as_str()));
        }
        let api = Url::parse(&format!(
            "https://api.medici.tv/{subdomain}satie/edito/movie-file/{display_id}/"
        ))?;
        let data = download_json_headers(client, &api, NAME, &display_id, &headers)?;

        if !video_flag(&data, "is_full_video") && video_flag(&data, "is_limited_by_user_access") {
            crate::screen::warning(&format!(
                "[{NAME}] The full video is for subscribers only. Only previews will be downloaded. If you have used the --cookies-from-browser option, try using the --cookies option instead"
            ));
        }

        let video_url = data
            .get("video")
            .ok_or_else(|| key_error(&display_id, "video"))?
            .get("video_url")
            .ok_or_else(|| key_error(&display_id, "video_url"))?
            .as_str()
            .ok_or_else(|| {
                Error::UnsupportedHtml5("Medici video URLs that are not strings".into())
            })?;
        let manifest = Url::parse(video_url)?;
        let stream = extract_m3u8_fatal_master(client, &manifest, NAME, &display_id, None, true)?;

        let id = match data.get("id") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            _ => return Err(key_error(&display_id, "id")),
        };
        let text = |key: &str| data.get(key).and_then(Value::as_str);
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", id.into()),
            ("display_id", display_id.clone().into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        if let Some(title) = text("title") {
            fields.push(("title", title.into()));
        }
        if let Some(description) = text("subtitle") {
            fields.push(("description", description.into()));
        }
        if let Some(thumbnail) = text("picture").and_then(url_or_none) {
            fields.push(("thumbnail", thumbnail.into()));
        }
        if let Some(timestamp) = text("date_publish").and_then(crate::dates::parse_iso8601) {
            fields.push(("timestamp", timestamp.into()));
        }
        let mut info = dict_info(url, (NAME, NAME), fields, stream.formats);
        info.subtitles = stream.subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.medici.tv/en/operas/thomas-ades-the-exterminating-angel-calixto-bieito-opera-bastille-paris"
        ));
        assert!(suitable(
            "https://edu.medici.tv/en/masterclasses/yvonne-loriod-olivier-messiaen"
        ));
        assert!(!suitable("https://www.medici.tv/"));
    }
}
