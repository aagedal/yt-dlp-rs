//! Upstream `CDAIE` (`yt_dlp/extractor/cda.py`), anonymous web extraction: a
//! video's `vfilm` page metadata, its `player_data` default file (decrypted
//! or ROT13) and the other qualities from the `videoGetLink` JSON-RPC
//! endpoint (DASH manifests and direct files). Logins, the API path, age
//! confirmation, HLS qualities and `CDAFolderIE` are not ported; those are
//! unsupported errors or are left out.
use super::{
    Extractor, dict_format, dict_info, download_webpage, download_webpage_note,
    extract_mpd_optional_note, post_json_errnote_optional,
};
use crate::{Error, Result, VideoFormat, VideoInfo};
use percent_encoding::percent_decode_str;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Cda;

const NAME: &str = "CDA";
const BASE_URL: &str = "https://www.cda.pl";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:(?:www|m)\.)?cda\.pl/video|ebd\.cda\.pl/[0-9]+x[0-9]+)/(?P<id>[0-9a-z]+)",
    )
    .unwrap()
});
static PREMIUM: &str = "Ten film jest dostępny dla użytkowników premium";
static GEO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"niedostępn[ey] w(?:&nbsp;|\s+)Twoim kraju\s*<").unwrap());
static AGE_CONFIRM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(<button[^>]+name="[^"]*age_confirm[^"]*")"#).unwrap());
// The upstream patterns with backreferences, spelled out per quote.
static UPLOADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<(?:span|meta)[^>]+itemprop=["']author["'][^>]*>.*?<(?:span|meta)[^>]+itemprop=["']name["'][^>]*>(?P<uploader>[^<]+)</(?:span|meta)>"#,
    )
    .unwrap()
});
static RATING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<(?:span|meta)[^>]+itemprop=["']ratingValue["'][^>]*>(?P<rating_value>[0-9.]+)|<span[^>]+\bclass=["']rating["'][^>]*>(?P<rating_value2>[0-9.]+)"#,
    )
    .unwrap()
});
static PLAYER_DATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"player_data=\\?"(?P<player_data>.+?)"|player_data=\\?'(?P<player_data2>.+?)'"#)
        .unwrap()
});
static QUALITY_BUTTON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<a[^>]+data-quality="[^"]+"[^>]+href="([^"]+)"[^>]+class="quality-btn"[^>]*>([0-9]+p)"#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("CDA {what} are not supported natively"))
}

/// `decrypt_file(a)` of the page's `player.js` cipher.
fn decrypt_file(file: &str) -> String {
    let mut text = file.to_owned();
    for part in ["_XDDD", "_CDA", "_ADC", "_CXD", "_QWE", "_Q5", "_IKSDE"] {
        text = text.replace(part, "");
    }
    let text = percent_decode_str(&text).decode_utf8_lossy().into_owned();
    let text: String = text
        .chars()
        .map(|c| {
            let code = c as u32;
            if (33..=126).contains(&code) {
                char::from_u32(33 + (code + 14) % 94).unwrap_or(c)
            } else {
                c
            }
        })
        .collect();
    let mut text = text.replace(".cda.mp4", "");
    for part in [".2cda.pl", ".3cda.pl"] {
        text = text.replace(part, ".cda.pl");
    }
    if text.contains("/upstream") {
        return format!("https://{}", text.replace("/upstream", ".mp4/upstream"));
    }
    format!("https://{text}.mp4")
}

/// `codecs.decode(s, 'rot_13')`: ASCII letters only.
fn rot13(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a') + 13) % 26 + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A') + 13) % 26 + b'A') as char,
            other => other,
        })
        .collect()
}

/// `int_or_none(quality[:-1])`.
fn height_of(quality: &str) -> Value {
    let mut chars = quality.chars();
    chars.next_back();
    chars
        .as_str()
        .parse::<i64>()
        .map_or(Value::Null, Value::from)
}

/// `ExtractorError`s for `_web_extract`'s checks, before any format work.
fn check_page(id: &str, webpage: &str) -> Result<()> {
    if webpage.contains(PREMIUM) {
        let message = crate::extractor::login::login_required_message(
            "This video is only available for premium users",
            Some("cdapl"),
            None,
        );
        return Err(Error::Reported(format!("[{NAME}] {id}: {message}")));
    }
    if GEO.is_match(webpage) {
        return Err(unsupported("geo-restricted videos"));
    }
    if AGE_CONFIRM.is_match(webpage) {
        return Err(unsupported("age-confirmation pages"));
    }
    Ok(())
}

/// `extract_format(page, version)`: the page's `player_data` default file and
/// its other qualities (appended to `formats`), with the JSON-RPC answers.
fn extract_format(
    client: &Client,
    id: &str,
    page: &str,
    version: &str,
    formats: &mut Vec<VideoFormat>,
    duration: &mut Value,
) -> Result<()> {
    let Some(captures) = PLAYER_DATA.captures(page) else {
        return Ok(());
    };
    let json_str = captures
        .name("player_data")
        .or_else(|| captures.name("player_data2"))
        .map_or("", |text| text.as_str());
    let Ok(player_data) = serde_json::from_str::<Value>(json_str) else {
        return Ok(());
    };
    let Some(video) = player_data.get("video").filter(|video| {
        video
            .as_object()
            .is_some_and(|video| video.contains_key("file"))
    }) else {
        crate::screen::warning(&format!(
            "[{NAME}] Unable to extract {version} version information"
        ));
        return Ok(());
    };
    let video_quality = video.get("quality").cloned().unwrap_or(Value::Null);
    let qualities = video
        .get("qualities")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let quality_name = qualities
        .iter()
        .find(|(_, value)| **value == video_quality)
        .map_or_else(
            || video_quality.as_str().unwrap_or_default().to_owned(),
            |(key, _)| key.clone(),
        );
    if let Some(file) = video
        .get("file")
        .and_then(Value::as_str)
        .filter(|file| !file.is_empty())
    {
        let url = if file.starts_with("uggc") {
            let decoded = rot13(file);
            match decoded.strip_suffix("adc.mp4") {
                Some(stem) => format!("{stem}.mp4"),
                None => decoded,
            }
        } else if !file.starts_with("http") {
            decrypt_file(file)
        } else {
            file.to_owned()
        };
        formats.push(dict_format(
            vec![
                ("url", json!(url)),
                ("format_id", json!(quality_name)),
                ("height", height_of(&quality_name)),
            ],
            NAME,
            id,
        )?);
    }
    for (quality, cda_quality) in &qualities {
        if *quality == quality_name {
            continue;
        }
        let body = json!({
            "jsonrpc": "2.0",
            "method": "videoGetLink",
            "id": 2,
            "params": [id, cda_quality, video.get("ts"), video.get("hash2"), {}],
        })
        .to_string();
        let note = format!("Fetching {quality} url");
        let errnote = format!("Failed to fetch {quality} url");
        let response = post_json_errnote_optional(
            client,
            &Url::parse(&format!("{BASE_URL}/video/{id}"))?,
            (NAME, id),
            (note.as_str(), errnote.as_str()),
            &[
                ("Content-Type", "application/json"),
                ("X-Requested-With", "XMLHttpRequest"),
            ],
            body,
        )?;
        let Some(video_url) = response
            .as_ref()
            .filter(|response| response["result"]["status"] == "ok")
            .and_then(|response| response["result"]["resp"].as_str())
            .filter(|resp| Url::parse(resp).is_ok())
        else {
            continue;
        };
        let video_url = Url::parse(video_url)?;
        let path = video_url.path();
        if path.ends_with(".mpd") {
            if let Some(stream) = extract_mpd_optional_note(
                client,
                &video_url,
                NAME,
                id,
                Some("dash"),
                "Downloading MPD manifest",
            ) {
                formats.extend(stream.formats);
            }
        } else if path.ends_with(".m3u8") {
            return Err(unsupported("HLS qualities"));
        } else {
            formats.push(dict_format(
                vec![
                    ("url", json!(video_url.as_str())),
                    ("format_id", json!(quality)),
                    ("height", height_of(quality)),
                ],
                NAME,
                id,
            )?);
        }
    }
    if duration.is_null()
        && let Some(text) = video.get("duration").and_then(Value::as_str)
        && let Some(number) = crate::parse_duration(text)
    {
        *duration = serde_json::to_value(number).unwrap_or(Value::Null);
    }
    Ok(())
}

impl Extractor for Cda {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<crate::ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_web_extract`: the `vfilm` page, with its `cda.player` cookie.
        crate::cookies::set_cookie("cda.pl", "cda.player", "html5");
        let webpage = download_webpage(
            client,
            &Url::parse(&format!("{BASE_URL}/video/{video_id}/vfilm"))?,
            NAME,
            &video_id,
        )?;
        check_page(&video_id, &webpage)?;

        let uploader = UPLOADER
            .captures(&webpage)
            .map(|found| found["uploader"].to_owned());
        let rating = RATING.captures(&webpage).and_then(|found| {
            found
                .name("rating_value")
                .or_else(|| found.name("rating_value2"))
                .and_then(|value| value.as_str().parse::<f64>().ok())
        });
        // `_og_search_title(webpage)` is evaluated eagerly (warning if missing).
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            super::search_warning(NAME, "OpenGraph title");
        }
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            super::search_warning(NAME, "OpenGraph description");
        }
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            super::search_warning(NAME, "OpenGraph thumbnail");
        }
        let ld = crate::extractor::json_ld::search(&webpage);

        let mut formats = Vec::new();
        let mut duration = Value::Null;
        extract_format(
            client,
            &video_id,
            &webpage,
            "default",
            &mut formats,
            &mut duration,
        )?;

        for found in QUALITY_BUTTON.captures_iter(&webpage) {
            let resolution = found[2].to_owned();
            let href = Url::parse(BASE_URL)?.join(&found[1])?;
            let note = format!("Downloading {resolution} version information");
            let Ok(page) = download_webpage_note(client, &href, NAME, &video_id, &note) else {
                crate::screen::warning(&format!(
                    "[{NAME}] Unable to download {resolution} version information"
                ));
                continue;
            };
            extract_format(
                client,
                &video_id,
                &page,
                &resolution,
                &mut formats,
                &mut duration,
            )?;
        }

        if duration.is_null()
            && let Some(value) = ld.get("duration")
        {
            duration = value.clone();
        }
        let age_limit = 0;
        // `merge_dicts` leaves `None` values out, so JSON-LD fills them later.
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", json!(video_id)),
            ("title", title.map_or(Value::Null, Value::String)),
            (
                "description",
                description.map_or(Value::Null, Value::String),
            ),
            (
                "average_rating",
                rating.map_or(Value::Null, |rating| json!(rating)),
            ),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::String)),
            ("formats", Value::Null),
            ("duration", duration),
            ("age_limit", json!(age_limit)),
        ];
        if let Some(uploader) = uploader {
            fields.insert(3, ("uploader", json!(uploader)));
        }
        // `merge_dicts(info_dict, json_ld)`: the JSON-LD keys not set yet.
        for (key, value) in &ld {
            match fields.iter_mut().find(|(name, _)| name == key) {
                Some((_, current)) if current.is_null() => *current = value.clone(),
                Some(_) => {}
                None => fields.push((key.as_str(), value.clone())),
            }
        }
        let info = dict_info(url, (NAME, NAME), fields, formats);
        Ok(Some(crate::ExtractResult::Video(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.cda.pl/video/57413289"));
        assert!(suitable("https://m.cda.pl/video/617297677"));
        assert!(suitable("http://ebd.cda.pl/0x0/5749950c"));
        assert!(!suitable("https://www.cda.pl/domino264/folder/31188385"));
    }

    #[test]
    fn decrypts_player_files() {
        assert_eq!(rot13("uggc://x.cda.pl/a.mp4"), "http://k.pqn.cy/n.zc4");
        assert_eq!(height_of("720p"), json!(720));
        assert_eq!(height_of("auto"), Value::Null);
    }
}
