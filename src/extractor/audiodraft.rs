//! Upstream `AudiodraftCustomIE` and `AudiodraftGenericIE`
//! (`yt_dlp/extractor/audiodraft.py`): a contest entry page's
//! `playAudio('player_entry_ID')` (or the contest URL's `eid`) POSTed to the
//! player info endpoint, whose JSON gives the MP3 URL and entry metadata.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct AudiodraftCustom;
pub struct AudiodraftGeneric;

const CUSTOM_NAME: &str = "Audiodraft:custom";
const CUSTOM_KEY: &str = "AudiodraftCustom";
const GENERIC_NAME: &str = "Audiodraft:generic";
const GENERIC_KEY: &str = "AudiodraftGeneric";

static CUSTOM_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:[-\w]+)\.audiodraft\.com/entry/(?P<id>\d+)").unwrap()
});
static GENERIC_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://www\.audiodraft\.com/contests/[^/#]+#entries&eid=(?P<id>\d+)").unwrap()
});
static PLAYER_ENTRY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"playAudio\('(player_entry_\d+)'\);").unwrap());

/// `AudiodraftCustomIE._match_id`.
pub fn suitable(url: &str) -> bool {
    CUSTOM_URL.is_match(url)
}

/// `AudiodraftGenericIE._match_id`.
pub fn generic_suitable(url: &str) -> bool {
    GENERIC_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// Python `int_or_none(value)` for the endpoint's ints and digit strings.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

/// Python `str(value)` of a JSON scalar.
fn py_str(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.into()),
        Value::Null => Some("None".into()),
        _ => None,
    }
}

/// `_audiodraft_extract_from_id(player_entry_id)`; `temp_id` is the URL's
/// ID (`get_temp_id`) for uncaught `KeyError`s.
fn extract_from_id(
    client: &Client,
    url: &Url,
    (name, key): (&'static str, &'static str),
    player_entry_id: &str,
    temp_id: &str,
) -> Result<VideoInfo> {
    let endpoint =
        Url::parse("https://www.audiodraft.com/scripts/general/player/getPlayerInfoNew.php")
            .expect("valid endpoint URL");
    let data = super::post_json_note_headers(
        client,
        &endpoint,
        name,
        player_entry_id,
        "Downloading JSON metadata",
        &[
            (
                "Content-type",
                "application/x-www-form-urlencoded; charset=UTF-8",
            ),
            ("X-Requested-With", "XMLHttpRequest"),
        ],
        format!("id={player_entry_id}"),
    )?;
    let entry_id = data
        .get("entry_id")
        .ok_or_else(|| key_error(temp_id, "entry_id"))?;
    let id = py_str(entry_id).ok_or_else(|| {
        Error::UnsupportedHtml5("Audiodraft entry IDs that are not scalars".into())
    })?;
    let path = data.get("path").ok_or_else(|| key_error(temp_id, "path"))?;
    let media = path
        .as_str()
        .and_then(|path| Url::parse(path).ok())
        .ok_or_else(|| {
            Error::UnsupportedHtml5("Audiodraft paths that are not absolute URLs".into())
        })?;
    let mut info = super::direct::with_format(
        super::direct::build_at(media.clone(), &media, "mp3".into()),
        "",
    );
    for format in &mut info.formats {
        format.ext = "mp3".into();
        format.vcodec = Some("none".into());
        format.format_id = String::new();
    }
    info.vcodec = Some("none".into());
    info.format_id = None;
    info.id = id;
    match data.get("entry_title") {
        Some(Value::String(title)) => info.title = title.clone(),
        None | Some(Value::Null) => {
            info.title = String::new();
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        Some(_) => {
            return Err(Error::UnsupportedHtml5(
                "Audiodraft titles that are not strings".into(),
            ));
        }
    }
    info.webpage_url = match data.get("entry_url") {
        Some(Value::String(page)) => Url::parse(page).map_err(|_| {
            Error::UnsupportedHtml5("Audiodraft entry URLs that are not absolute".into())
        })?,
        None | Some(Value::Null) => url.clone(),
        Some(_) => {
            return Err(Error::UnsupportedHtml5(
                "Audiodraft entry URLs that are not strings".into(),
            ));
        }
    };
    info.direct = None;
    info.extractor = name.into();
    info.extractor_key = key.into();
    let get = |key: &str| data.get(key).cloned().unwrap_or(Value::Null);
    for (field, value) in [
        ("uploader", get("designer_name")),
        ("uploader_id", get("designer_id")),
        (
            "like_count",
            int_or_none(data.get("entry_likes")).map_or(Value::Null, Value::from),
        ),
        (
            "average_rating",
            int_or_none(data.get("entry_rating")).map_or(Value::Null, Value::from),
        ),
    ] {
        if value.is_null() {
            info.extra.insert(field.into(), Value::Null);
        } else {
            crate::metadata_parser::set_field(&mut info, field, value);
        }
    }
    Ok(crate::info_order::mark_info(
        info,
        "untyped-dict;id,title,url,vcodec,ext,uploader,uploader_id,webpage_url,like_count,average_rating",
    ))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for AudiodraftCustom {
    fn key(&self) -> &'static str {
        CUSTOM_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = CUSTOM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(CUSTOM_NAME, url);
        let webpage = download_webpage(client, url, CUSTOM_NAME, &video_id)?;
        // `_search_regex(pattern, webpage, video_id, 'play entry id')`: the
        // video ID is the name and the fourth argument the default.
        let player_entry_id = PLAYER_ENTRY
            .captures(&webpage)
            .map_or_else(|| "play entry id".to_owned(), |found| found[1].to_owned());
        extract_from_id(
            client,
            url,
            (CUSTOM_NAME, CUSTOM_KEY),
            &player_entry_id,
            &video_id,
        )
        .map(Some)
    }
}

impl Extractor for AudiodraftGeneric {
    fn key(&self) -> &'static str {
        GENERIC_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = GENERIC_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(GENERIC_NAME, url);
        extract_from_id(
            client,
            url,
            (GENERIC_NAME, GENERIC_KEY),
            &format!("player_entry_{video_id}"),
            &video_id,
        )
        .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://nokiatune.audiodraft.com/entry/5874"));
        assert!(!suitable("https://www.audiodraft.com/contests/x"));
        assert!(generic_suitable(
            "https://www.audiodraft.com/contests/570-Score-A-Video-Surprise-Us#entries&eid=30138"
        ));
        assert!(!generic_suitable(
            "https://www.audiodraft.com/contests/570-Score-A-Video-Surprise-Us"
        ));
    }
}
