//! Upstream `NexxIE` (`yt_dlp/extractor/nexx.py`): the video's metadata from
//! `arc.nexx.cloud`, then the formats of its Azure CDN (the HLS, DASH and
//! Smooth manifests, and the progressive MP4s). Not ported: the
//! `session/init` and `videos/byid` API fallback for videos arc does not
//! return, the `free` and `3q` CDNs, caption tracks, and `NexxEmbedIE`; those
//! are explicit unsupported errors rather than guesses.
use super::{
    Extractor, dict_format, dict_info, download_json, extract_ism_optional,
    extract_m3u8_master_optional, extract_mpd_optional_note, streamable::int_or_none,
};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Nexx;

const NAME: &str = "Nexx";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    super::url_regex(
        r"(?x)^(?:
            https?://api\.nexx(?:\.cloud|cdn\.com)/v3(?:\.\d)?/(?P<domain_id>\d+)/videos/byid/|
            nexx:(?:(?P<domain_id_s>\d+):)?|
            https?://arc\.nexx\.cloud/api/video/
        )(?P<id>\d+)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Nexx {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `traverse_obj(video, (('episodedata', 'general'), 'episode'), expected_type=int, get_all=False)`.
fn episode_number(video: &Value, key: &str) -> Value {
    ["episodedata", "general"]
        .iter()
        .filter_map(|branch| video.get(*branch)?.get(key)?.as_i64())
        .next()
        .map_or(Value::Null, Value::from)
}

/// `_get_cdn_shield_base(shield_type, static)` of the Azure formats.
fn cdn_shield_base(stream: &Value, shield_type: &str, is_static: bool, id: &str) -> Result<String> {
    for secure in ["", "s"] {
        let key = format!("cdnShield{shield_type}HTTP{}", secure.to_uppercase());
        if let Some(shield) = stream.get(&key).and_then(Value::as_str)
            && !shield.is_empty()
        {
            return Ok(format!("http{secure}://{shield}"));
        }
    }
    let account = stream
        .get("azureAccount")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(id, "azureAccount"))?;
    let prefix = match (account.contains("fb"), is_static) {
        (true, true) => "df",
        (true, false) => "f",
        (false, true) => "d",
        (false, false) => "p",
    };
    let number: u32 = account
        .replace("nexxplayplus", "")
        .replace("nexxplayfb", "")
        .parse()
        .map_err(|_| unsupported("Azure accounts that are not numbered"))?;
    Ok(format!("http://nx-{prefix}{number:02}.akamaized.net/"))
}

/// `_extract_azure_formats`: the manifests (non-fatal) and the progressive
/// MP4s of `azureFileDistribution`.
fn azure_formats(client: &Client, video: &Value, video_id: &str) -> Result<Vec<VideoFormat>> {
    let stream = video
        .get("streamdata")
        .ok_or_else(|| key_error(video_id, "streamdata"))?;
    let locator = stream
        .get("azureLocator")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(video_id, "azureLocator"))?;
    let language = video
        .get("general")
        .and_then(|general| general.get("language_raw"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let azure_stream_base = cdn_shield_base(stream, "", false, video_id)?;
    let is_ml = language.contains(',');
    let manifest_base = format!(
        "{azure_stream_base}{locator}/{video_id}_src{}.ism/Manifest",
        if is_ml { "_manifest" } else { "" }
    );
    let token = video
        .get("protectiondata")
        .and_then(|data| data.get("token"))
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(|token| format!("?hdnts={token}"))
        .unwrap_or_default();
    let manifest = |suffix: &str| -> Result<Url> {
        Ok(Url::parse(&format!("{manifest_base}{suffix}{token}"))?)
    };

    let mut formats = Vec::new();
    if let Some(stream) = extract_m3u8_master_optional(
        client,
        &manifest("(format=m3u8-aapl)")?,
        NAME,
        video_id,
        Some("azure-hls"),
        false,
        false,
    ) {
        formats.extend(stream.formats);
    }
    if let Some(stream) = extract_mpd_optional_note(
        client,
        &manifest("(format=mpd-time-csf)")?,
        NAME,
        video_id,
        Some("azure-dash"),
        "Downloading MPD manifest",
    ) {
        formats.extend(stream.formats);
    }
    let (ism_formats, _) =
        extract_ism_optional(client, NAME, &manifest("")?, video_id, Some("azure-mss"))?;
    formats.extend(ism_formats);

    let progressive_base = cdn_shield_base(stream, "Prog", true, video_id)?;
    if let Some(distribution) = stream
        .get("azureFileDistribution")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        for entry in distribution.split(',') {
            let parts: Vec<&str> = entry.split(':').collect();
            if parts.len() != 2 {
                continue;
            }
            let Ok(tbr) = parts[0].parse::<i64>() else {
                continue;
            };
            if tbr == 0 {
                continue;
            }
            let mut fields = vec![
                (
                    "url",
                    Value::from(format!(
                        "{progressive_base}{locator}/{video_id}_src_{}_{tbr}.mp4",
                        parts[1]
                    )),
                ),
                ("format_id", Value::from(format!("azure-http-{tbr}"))),
                ("tbr", Value::from(tbr)),
            ];
            let dimensions: Vec<&str> = parts[1].split('x').collect();
            if dimensions.len() == 2 {
                fields.push(("width", int_or_none(Some(&Value::from(dimensions[0])))));
                fields.push(("height", int_or_none(Some(&Value::from(dimensions[1])))));
            }
            let mut format = dict_format(fields, NAME, video_id)?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
    }
    Ok(formats)
}

impl Extractor for Nexx {
    fn key(&self) -> &'static str {
        NAME
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

        // Upstream's non-fatal arc lookup, then the session API when arc has
        // no such video; only the arc lookup is ported.
        let arc = Url::parse(&format!("https://arc.nexx.cloud/api/video/{video_id}.json"))?;
        let response = download_json(client, &arc, NAME, &video_id)
            .map_err(|_| unsupported("video lookups that fall back to the session API"))?;
        let video = response
            .get("result")
            .and_then(|result| match result {
                Value::Object(_) => Some(result.clone()),
                Value::Array(items) => items
                    .iter()
                    .find(|item| {
                        item.get("general")
                            .and_then(|general| general.get("ID"))
                            .and_then(Value::as_i64)
                            == video_id.parse::<i64>().ok()
                    })
                    .cloned(),
                _ => None,
            })
            .ok_or_else(|| unsupported("video lookups that fall back to the session API"))?;

        let general = video
            .get("general")
            .ok_or_else(|| key_error(&video_id, "general"))?;
        let title = general
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "title"))?;
        let cdn = video
            .get("streamdata")
            .and_then(|stream| stream.get("cdnType"))
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&video_id, "cdnType"))?;
        let formats = match cdn {
            "azure" => azure_formats(client, &video, &video_id)?,
            other => return Err(unsupported(&format!("{other} CDN formats"))),
        };

        if video
            .get("captiondata")
            .and_then(Value::as_array)
            .is_some_and(|tracks| !tracks.is_empty())
        {
            return Err(unsupported("caption tracks"));
        }

        let creator = ["studio", "studio_adref"]
            .iter()
            .find_map(|key| {
                general
                    .get(*key)
                    .filter(|value| super::nuxt::truthy(value))
                    .cloned()
            })
            .unwrap_or(Value::Null);
        let thumbnail = video
            .get("imagedata")
            .and_then(|data| data.get("thumb"))
            .filter(|value| value.is_string())
            .cloned()
            .unwrap_or(Value::Null);
        let duration = general
            .get("runtime")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
            .and_then(|duration| serde_json::to_value(duration).ok())
            .unwrap_or(Value::Null);
        // `traverse_obj(video, ('connectedmedia', ..., 'title'), expected_type=str)`.
        let cast: Vec<Value> = match video.get("connectedmedia") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.get("title").filter(|title| title.is_string()).cloned())
                .collect(),
            Some(Value::Object(items)) => items
                .values()
                .filter_map(|item| item.get("title").filter(|title| title.is_string()).cloned())
                .collect(),
            _ => Vec::new(),
        };

        let fields = vec![
            ("id", json!(video_id)),
            ("title", title),
            (
                "alt_title",
                general.get("subtitle").cloned().unwrap_or(Value::Null),
            ),
            (
                "description",
                general.get("description").cloned().unwrap_or(Value::Null),
            ),
            ("release_year", int_or_none(general.get("year"))),
            ("creator", creator),
            ("thumbnail", thumbnail),
            ("duration", duration),
            ("timestamp", int_or_none(general.get("uploaded"))),
            ("episode_number", episode_number(&video, "episode")),
            ("season_number", episode_number(&video, "season")),
            ("cast", Value::Array(cast)),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = crate::Subtitles::default();
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://api.nexx.cloud/v3/748/videos/byid/128907"));
        assert!(suitable("nexx:128907"));
        assert!(suitable("nexx:748:128907"));
        assert!(suitable("https://arc.nexx.cloud/api/video/128907.json"));
        assert!(!suitable("https://embed.nexx.cloud/748/video/abc"));
    }
}
