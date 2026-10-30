//! Upstream `RadioCanadaIE` and `RadioCanadaAudioVideoIE`
//! (`yt_dlp/extractor/radiocanada.py`): the media console's metadata and
//! validation API as an HLS result with its closed captions, and the
//! audio-video pages as `radiocanada:medianet:` URL results.
use super::{Extractor, dict_info, download_json, extract_m3u8_fatal_master, follow_url};
use crate::{ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct RadioCanada;
pub struct RadioCanadaAudioVideo;

const NAME: &str = "radiocanada";
const KEY: &str = "RadioCanada";
const AUDIO_VIDEO_KEY: &str = "RadioCanadaAudioVideo";
const CLIENT_KEY: &str = "773aea60-0e80-41bb-9c7f-e6d7c3ad17fb";
const GEO_COUNTRIES: [&str; 1] = ["CA"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:radiocanada:|https?://ici\.radio-canada\.ca/widgets/mediaconsole/)(?P<app_code>[^:/]+)[:/](?P<id>[0-9]+)")
        .unwrap()
});
static AUDIO_VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://ici\.radio-canada\.ca/([^/]+/)*media-(?P<id>[0-9]+)").unwrap()
});

/// `_call_api`: `query` first, then `client_key` and `output`, then the
/// media's `appCode` and `idMedia`.
fn call_api(
    client: &Client,
    path: &str,
    video_id: Option<&str>,
    app_code: Option<&str>,
    query: &[(&str, &str)],
) -> Result<Value> {
    let mut url = Url::parse(&format!("https://services.radio-canada.ca/media/{path}"))?;
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
        pairs.append_pair("client_key", CLIENT_KEY);
        pairs.append_pair("output", "json");
        if let Some(video_id) = video_id {
            pairs.append_pair("appCode", app_code.unwrap_or_default());
            pairs.append_pair("idMedia", video_id);
        }
    }
    download_json(client, &url, NAME, video_id.unwrap_or_default())
}

/// `get_meta(name)`: the first non-empty text of a named meta.
fn get_meta(metas: &[Value], name: &str) -> Option<String> {
    metas.iter().find_map(|meta| {
        if meta.get("name").and_then(Value::as_str) != Some(name) {
            return None;
        }
        match meta.get("text") {
            Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
            _ => None,
        }
    })
}

/// `int_or_none` of a meta's text.
fn meta_int(metas: &[Value], name: &str) -> Value {
    get_meta(metas, name)
        .and_then(|text| text.trim().parse::<i64>().ok())
        .map_or(Value::Null, Value::from)
}

/// `_extract_info(app_code, video_id)`.
fn extract_info(client: &Client, url: &Url, app_code: &str, video_id: &str) -> Result<VideoInfo> {
    let response = call_api(
        client,
        "meta/v1/index.ashx",
        Some(video_id),
        Some(app_code),
        &[],
    )?;
    let metas = match response.get("Metas") {
        Some(Value::Array(metas)) => metas.clone(),
        _ => {
            return Err(crate::Error::Reported(format!(
                "[{NAME}] {video_id}: An extractor error has occurred. (caused by KeyError('Metas')); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
    };
    let text = |name: &str| get_meta(&metas, name);

    // `protectionType` does not necessarily mean the video is DRM protected.
    if text("protectionType").is_some() {
        crate::screen::warning(&format!("[{NAME}] This video is probably DRM protected."));
    }

    let validation = call_api(
        client,
        "validation/v2/",
        Some(video_id),
        Some(app_code),
        &[
            ("connectionType", "hd"),
            ("deviceType", "ipad"),
            ("multibitrate", "true"),
        ],
    )?;
    let Some(v_url) = validation
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
    else {
        let error = validation
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if error == "Le contenu sélectionné n'est pas disponible dans votre pays" {
            return Err(crate::geo::restricted(
                &format!("[{NAME}] {video_id}: {error}"),
                &GEO_COUNTRIES,
            ));
        }
        if error == "Le contenu sélectionné est disponible seulement en premium" {
            return Err(crate::Error::Reported(format!(
                "[{NAME}] {video_id}: {}",
                super::login::login_required_message(
                    error,
                    None,
                    Some(super::login::Hint::Cookies),
                )
            )));
        }
        return Err(crate::Error::Reported(format!(
            "[{NAME}] {video_id}: {NAME} said: {error}"
        )));
    };
    let manifest = Url::parse(v_url)?;
    let formats = extract_m3u8_fatal_master(client, &manifest, NAME, video_id, None, false)?;

    let fields: Vec<(&str, Value)> = vec![
        ("id", Value::from(video_id)),
        (
            "title",
            text("Title")
                .or_else(|| text("AV-nomEmission"))
                .map_or(Value::Null, Value::from),
        ),
        (
            "description",
            text("Description")
                .or_else(|| text("ShortDescription"))
                .map_or(Value::Null, Value::from),
        ),
        (
            "thumbnail",
            text("imageHR")
                .or_else(|| text("imageMR"))
                .or_else(|| text("imageBR"))
                .map_or(Value::Null, Value::from),
        ),
        ("duration", meta_int(&metas, "length")),
        ("series", text("Emission").map_or(Value::Null, Value::from)),
        ("season_number", meta_int(&metas, "SrcSaison")),
        ("episode_number", meta_int(&metas, "SrcEpisode")),
        (
            "upload_date",
            text("Date")
                .and_then(|date| crate::dates::unified_strdate(&date, false))
                .map_or(Value::Null, Value::from),
        ),
        ("subtitles", Value::Null),
        ("formats", Value::Null),
    ];
    // Closed captions, as French subtitles with their `determine_ext`.
    let mut subtitles = crate::Subtitles::default();
    if let Some(caption) = text("closedCaption").or_else(|| text("closedCaptionHTML5")) {
        let ext = super::direct::determine_ext(&caption).unwrap_or_else(|| "vtt".into());
        subtitles = serde_json::from_value(json!({"fr": [{"url": caption, "ext": ext}]}))
            .unwrap_or_default();
    }
    let mut info = dict_info(url, (NAME, KEY), fields, formats.formats);
    info.subtitles = subtitles;
    Ok(info)
}

impl Extractor for RadioCanada {
    fn key(&self) -> &'static str {
        KEY
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&GEO_COUNTRIES)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let app_code = found["app_code"].to_owned();
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(KEY);
        crate::screen::screen(&format!(
            "[{KEY}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        extract_info(client, url, &app_code, &video_id).map(Some)
    }
}

impl Extractor for RadioCanadaAudioVideo {
    fn key(&self) -> &'static str {
        AUDIO_VIDEO_KEY
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = AUDIO_VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        // `url_result('radiocanada:medianet:' + id)`.
        Ok(Some(follow_url(
            url,
            format!("radiocanada:medianet:{}", &found["id"]),
        )))
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn audio_video_suitable(url: &str) -> bool {
    AUDIO_VIDEO_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suitable(url: &str) -> bool {
        VALID_URL.is_match(url)
    }

    fn audio_video_suitable(url: &str) -> bool {
        AUDIO_VIDEO_URL.is_match(url)
    }

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://ici.radio-canada.ca/widgets/mediaconsole/medianet/7184272"
        ));
        assert!(suitable("radiocanada:toutv:140872"));
        assert!(audio_video_suitable(
            "https://ici.radio-canada.ca/info/videos/media-7527184/barack-obama-au-vietnam"
        ));
        assert!(!audio_video_suitable("https://ici.radio-canada.ca/info/"));
    }
}
