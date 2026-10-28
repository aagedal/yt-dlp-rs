//! Upstream `BeatportIE` (`yt_dlp/extractor/beatport.py`): Beatport track
//! previews from the page's `window.Playables`: the track's preview files
//! (MP3/AAC audio), its artists, name and mix as the title, and its images.
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Beatport;

const NAME: &str = "Beatport";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.|pro\.)?beatport\.com/track/(?P<display_id>[^/]+)/(?P<id>[0-9]+)",
    )
    .unwrap()
});

static PLAYABLES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)window\.Playables\s*=\s*(\{.+?\});").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Beatport {what} are not supported natively"))
}

fn extractor_error(id: &str, cause: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by {cause}); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `obj[key]` of a JSON object.
fn get<'a>(obj: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    match obj {
        Value::Object(map) => map
            .get(key)
            .ok_or_else(|| extractor_error(id, &format!("KeyError('{key}')"))),
        _ => Err(unsupported("playables that are not objects")),
    }
}

/// A JSON string field used in string concatenation.
fn string<'a>(value: &'a Value, what: &str) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| unsupported(&format!("{what} that are not strings")))
}

/// `t['id'] == int(track_id)`.
fn same_id(value: &Value, track_id: &str) -> bool {
    let Value::Number(number) = value else {
        return false;
    };
    match number.as_i64() {
        Some(left) => track_id.parse::<i64>() == Ok(left),
        None => number.as_f64() == track_id.parse::<f64>().ok(),
    }
}

/// Python's `str()` of a track ID.
fn python_str(value: &Value) -> Result<String> {
    match value {
        Value::Null => Ok("None".into()),
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) if number.is_f64() => {
            Ok(crate::screen::py_float(number.as_f64().unwrap_or_default()))
        }
        Value::Number(number) => Ok(number.to_string()),
        _ => Err(unsupported("track IDs that are not numbers or strings")),
    }
}

impl Extractor for Beatport {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let track_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let playables = PLAYABLES.captures(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {track_id}: Unable to extract playables info; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let playables: Value = serde_json::from_str(&playables[1])
            .map_err(|_| unsupported("playables that are not plain JSON"))?;
        let Value::Array(tracks) = get(&playables, "tracks", &track_id)? else {
            return Err(unsupported("track lists that are not lists"));
        };
        let mut track = None;
        for candidate in tracks {
            if same_id(get(candidate, "id", &track_id)?, &track_id) {
                track = Some(candidate);
                break;
            }
        }
        let track = track.ok_or_else(|| extractor_error(&track_id, "StopIteration()"))?;
        let Value::Array(artists) = get(track, "artists", &track_id)? else {
            return Err(unsupported("artist lists that are not lists"));
        };
        let mut names = Vec::new();
        for artist in artists {
            names.push(string(get(artist, "name", &track_id)?, "artist names")?);
        }
        let mut title = format!(
            "{} - {}",
            names.join(", "),
            string(get(track, "name", &track_id)?, "track names")?
        );
        let mix = get(track, "mix", &track_id)?;
        if super::nuxt::truthy(mix) {
            title = format!("{title} ({})", string(mix, "mixes")?);
        }
        let Value::Object(previews) = get(track, "preview", &track_id)? else {
            return Err(unsupported("previews that are not objects"));
        };
        let mut formats = Vec::new();
        for (ext, info) in previews {
            let preview_url = get(info, "url", &track_id)?;
            if !super::nuxt::truthy(preview_url) {
                continue;
            }
            let mut fields = vec![
                ("url", preview_url.clone()),
                ("ext", ext.as_str().into()),
                ("format_id", ext.as_str().into()),
                ("vcodec", "none".into()),
            ];
            let acodec = match ext.as_str() {
                "mp3" => Some("mp3"),
                "mp4" => Some("aac"),
                _ => None,
            };
            if let Some(acodec) = acodec {
                fields.extend([
                    ("acodec", acodec.into()),
                    ("abr", 96.into()),
                    ("asr", 44100.into()),
                ]);
            }
            let mut format = dict_format(fields, NAME, &track_id)?;
            if acodec.is_some() {
                crate::info_order::mark_int_rates(&mut format);
            }
            formats.push(format);
        }
        let Value::Object(images) = get(track, "images", &track_id)? else {
            return Err(unsupported("image maps that are not objects"));
        };
        let mut thumbnails = Vec::new();
        for (name, info) in images {
            let Value::Object(info) = info else {
                return Err(unsupported("images that are not objects"));
            };
            let image_url = info.get("url").filter(|value| super::nuxt::truthy(value));
            let Some(image_url) = image_url else {
                continue;
            };
            if name == "dynamic" {
                continue;
            }
            let mut image = serde_json::Map::new();
            image.insert("id".into(), name.as_str().into());
            image.insert("url".into(), image_url.clone());
            image.insert("height".into(), int_or_none(info.get("height")));
            image.insert("width".into(), int_or_none(info.get("width")));
            thumbnails.push(Value::Object(image));
        }
        let id = python_str(track.get("id").unwrap_or(&Value::Null))?;
        let id = if id.is_empty() { track_id } else { id };
        let slug = track
            .get("slug")
            .filter(|value| super::nuxt::truthy(value))
            .cloned()
            .unwrap_or_else(|| display_id.into());
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", id.into()),
                ("display_id", slug),
                ("title", title.into()),
                ("formats", Value::Null),
                ("thumbnails", Value::Array(thumbnails)),
            ],
            formats,
        );
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail!=id,url,height,width");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://beatport.com/track/synesthesia-original-mix/5379371"
        ));
        assert!(suitable("https://pro.beatport.com/track/x/1"));
        assert!(!suitable("https://beatport.com/release/x/1"));
    }

    #[test]
    fn track_ids_compare_as_numbers() {
        assert!(same_id(&serde_json::json!(5379371), "5379371"));
        assert!(same_id(&serde_json::json!(5379371.0), "5379371"));
        assert!(!same_id(&serde_json::json!("5379371"), "5379371"));
    }
}
