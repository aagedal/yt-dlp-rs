//! Upstream `NTVDeIE` (`yt_dlp/extractor/ntvde.py`): n-tv.de Mediathek
//! videos from the page's `article` object and `#playerwrapper` player
//! data: the progressive file and non-fatal HLS and DASH manifests, with the
//! headline, intro, kicker, publication time, poster and length.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct NtvDe;

const NAME: &str = "n-tv.de";
const KEY: &str = "NTVDe";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?n-tv\.de/mediathek/(?:videos|magazine)/[^/?#]+/[^/?#]+-article(?P<id>[^/?#]+)\.html",
    )
    .unwrap()
});
static ARTICLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"article:").unwrap());
static PLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r##"\$\(\s*"#playerwrapper"\s*\)\s*\.data\(\s*"player","##).unwrap()
});
static IVW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"ivw:[^},]+").unwrap());

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
    Error::UnsupportedHtml5(format!("n-tv.de {what} are not supported natively"))
}

/// Fatal `_search_json(start, webpage, name, id, transform_source=...)`.
fn search_json(
    start: &Regex,
    webpage: &str,
    name: &str,
    id: &str,
    transform: &dyn Fn(&str) -> String,
) -> Result<Value> {
    match super::search_json_detailed(start, webpage, transform) {
        Some(Ok(value)) => Ok(value),
        Some(Err(error)) => {
            let cause = error.map_or_else(String::new, |error| {
                format!(
                    " (caused by JSONDecodeError({}))",
                    crate::batch::py_repr(&error)
                )
            });
            Err(Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract {name} - Failed to parse JSON{cause}; {}",
                crate::BUG_REPORTS_MESSAGE
            )))
        }
        None => Err(Error::Reported(format!(
            "[{NAME}] {id}: Unable to extract {name}; {}",
            crate::BUG_REPORTS_MESSAGE
        ))),
    }
}

/// A truthy `vdata.get(key)` as a manifest URL.
fn manifest(vdata: &Value, key: &str) -> Result<Option<Url>> {
    match vdata.get(key).filter(|value| super::nuxt::truthy(value)) {
        None => Ok(None),
        Some(Value::String(text)) => Url::parse(text)
            .map(Some)
            .map_err(|_| unsupported("manifest URLs that are not URLs")),
        Some(_) => Err(unsupported("manifest URLs that are not strings")),
    }
}

/// `traverse_obj(obj, KEY)` for a string field.
fn text(obj: &Value, key: &str) -> Result<Value> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::String(text)) => Ok(text.as_str().into()),
        Some(_) => Err(unsupported("article fields that are not strings")),
    }
}

impl Extractor for NtvDe {
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
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let info = search_json(
            &ARTICLE,
            &webpage,
            "info",
            &video_id,
            &crate::js_to_json::js_to_json,
        )?;
        let player = search_json(&PLAYER, &webpage, "player data", &video_id, &|text| {
            crate::js_to_json::js_to_json(&IVW.replace_all(text, ""))
        })?;
        if !player.is_object() {
            return Err(unsupported("player data that are not objects"));
        }
        let setup = player
            .get("setup")
            .ok_or_else(|| key_error(&video_id, "setup"))?;
        if !setup.is_object() {
            return Err(unsupported("player setups that are not objects"));
        }
        let vdata = setup
            .get("source")
            .ok_or_else(|| key_error(&video_id, "source"))?;
        if !vdata.is_object() {
            return Err(unsupported("player sources that are not objects"));
        }

        let mut formats = Vec::new();
        if let Some(progressive) = vdata
            .get("progressive")
            .filter(|value| super::nuxt::truthy(value))
        {
            formats.push(dict_format(
                vec![("format_id", "http".into()), ("url", progressive.clone())],
                NAME,
                &video_id,
            )?);
        }
        if let Some(hls) = manifest(vdata, "hls")?
            && let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &hls,
                NAME,
                &video_id,
                Some("hls"),
                false,
                false,
            )
        {
            formats.extend(stream.formats);
        }
        if let Some(dash) = manifest(vdata, "dash")?
            && let Some(mut stream) = super::extract_mpd_optional_note(
                client,
                &dash,
                NAME,
                &video_id,
                Some("dash"),
                "Downloading MPD manifest",
            )
        {
            super::ignore_subtitles(&mut stream, NAME, "DASH");
            formats.extend(stream.formats);
        }

        let (title, description, alt_title, timestamp) = if info.is_object() {
            (
                text(&info, "headline")?,
                text(&info, "intro")?,
                text(&info, "kicker")?,
                super::streamable::int_or_none(info.get("publishedDateAsUnixTimeStamp")),
            )
        } else {
            (Value::Null, Value::Null, Value::Null, Value::Null)
        };
        let thumbnail = vdata
            .get("poster")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map_or(Value::Null, Value::from);
        let duration = super::streamable::int_or_none(vdata.get("length"));
        let mut fields = vec![("id", Value::from(video_id))];
        for (key, value) in [
            ("title", title),
            ("description", description),
            ("alt_title", alt_title),
            ("timestamp", timestamp),
            ("thumbnail", thumbnail),
            ("duration", duration),
        ] {
            if !value.is_null() {
                fields.push((key, value));
            }
        }
        fields.push(("formats", Value::Null));
        Ok(Some(dict_info(url, (NAME, KEY), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.n-tv.de/mediathek/videos/panorama/Schnee-und-Glaette-article14438086.html"
        ));
        assert!(suitable(
            "https://www.n-tv.de/mediathek/magazine/auslandsreport/Juedische-Siedler-article24523089.html"
        ));
        assert!(!suitable("https://www.n-tv.de/mediathek/videos/panorama/"));
    }
}
