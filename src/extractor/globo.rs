//! Upstream `GloboIE` and `GloboArticleIE` (`yt_dlp/extractor/globo.py`): a
//! Globoplay video's GraphQL record, its playback session (`video-session`,
//! with the `GLBID` cookie's bearer token when present) and its HLS
//! manifest; and a `ge.globo.com`-style article as a playlist of `globo:ID`
//! URL results. DASH manifests, DRM and subtitle tracks are not supported
//! natively.
use super::clyp::key_error;
use super::{
    Extractor, dict_info, direct, download_json_headers, download_webpage,
    extract_m3u8_fatal_master, post_json_note_headers,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Globo;
pub struct GloboArticle;

const NAME: &str = "Globo";
const ARTICLE: &str = "GloboArticle";
/// `str(uuid.uuid4())`: the port has no UUID generator, so the session's
/// `vsid` is fixed.
const SESSION_ID: &str = "00000000-0000-4000-8000-000000000000";

/// `GloboIE._VIDEO_VIEW`.
const VIDEO_VIEW: &str = r#"
    query getVideoView($videoId: ID!) {
        video(id: $videoId) {
            duration
            description
            relatedEpisodeNumber
            relatedSeasonNumber
            headline
            title {
                originProgramId
                headline
            }
        }
    }
    "#;

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:globo:|https?://[^/?#]+?\.globo\.com/(?:[^/?#]+/))(?P<id>\d{7,})").unwrap()
});
/// `GloboArticleIE._VALID_URL` without its `(?!globoplay)` lookahead, which
/// `article_suitable` applies by hand.
static ARTICLE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?P<rest>.+?\.globo\.com/(?:[^/?#]+/)*(?P<id>[^/?#.]+)(?:\.html)?)")
        .unwrap()
});
static VIDEO_ID_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"\bdata-video-id=["'](\d{7,})["']"#,
        r#"\bdata-player-videosids=["'](\d{7,})["']"#,
        r#"\bvideosIDs\s*:\s*["']?(\d{7,})"#,
        r#"\bdata-id=["'](\d{7,})["']"#,
        r#"<div[^>]+\bid=["'](\d{7,})["']"#,
        r#"<bs-player[^>]+\bvideoid=["'](\d{8,})["']"#,
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});

/// `GloboIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `GloboArticleIE.suitable`: not a Globoplay video, then the article regex.
pub fn article_suitable(url: &str) -> bool {
    if VALID_URL.is_match(url) {
        return false;
    }
    ARTICLE_URL
        .captures(url)
        .is_some_and(|found| !found["rest"].starts_with("globoplay"))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Globo {what} are not supported natively"))
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `str_or_none` of an ID.
fn id_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `int_or_none`, dropping `None`.
fn int_field(value: Option<&Value>) -> Value {
    super::streamable::int_or_none(value)
}

/// The playback session's body: `filter_dict` over the upstream dict.
fn session_body(video_id: &str, bearer: Option<String>) -> String {
    let mut body = Map::new();
    body.insert("player_type".into(), "mirakulo_8k_hdr".into());
    body.insert("video_id".into(), video_id.into());
    body.insert("quality".into(), "max".into());
    body.insert("content_protection".into(), "widevine".into());
    body.insert("vsid".into(), SESSION_ID.into());
    body.insert("consumption".into(), "streaming".into());
    body.insert("capabilities".into(), json!({"low_latency": true}));
    body.insert("tz".into(), "-03:00".into());
    if let Some(bearer) = bearer {
        body.insert("Authorization".into(), bearer.into());
    }
    body.insert("version".into(), 1.into());
    crate::output::python_json_dumps(&Value::Object(body))
}

/// `try_get(self._get_cookies('https://globo.com'), lambda x: f'Bearer {x["GLBID"].value}')`.
fn glbid_bearer() -> Option<String> {
    crate::cookies::get_cookie("https://globo.com", "GLBID").map(|value| format!("Bearer {value}"))
}

impl Extractor for Globo {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);

        let variables = crate::output::python_json_dumps(&json!({"videoId": video_id}));
        let graphql = Url::parse_with_params(
            "https://cloud-jarvis.globo.com/graphql",
            &[
                ("operationName", "getVideoView"),
                ("variables", variables.as_str()),
                ("query", VIDEO_VIEW),
            ],
        )?;
        let info = download_json_headers(
            client,
            &graphql,
            NAME,
            &video_id,
            &[
                ("content-type", "application/json"),
                ("x-platform-id", "web"),
                ("x-device-id", "desktop"),
                ("x-client-version", "2024.12-5"),
            ],
        )?;
        let info = info
            .get("data")
            .and_then(|data| data.get("video"))
            .cloned()
            .ok_or_else(|| key_error(&video_id, "data"))?;

        let session = post_json_note_headers(
            client,
            &Url::parse("https://playback.video.globo.com/v4/video-session")?,
            NAME,
            &video_id,
            &format!("Downloading resource info for {video_id}"),
            &[("Content-Type", "application/json")],
            session_body(&video_id, glbid_bearer()),
        )?;
        if session
            .get("resource")
            .and_then(|resource| resource.get("drm_protection_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(unsupported("DRM-protected videos"));
        }
        let main_source = session
            .get("sources")
            .and_then(|sources| sources.get(0))
            .ok_or_else(|| key_error(&video_id, "sources"))?;
        let main_url = main_source
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&video_id, "url"))?;
        if direct::determine_ext(main_url).as_deref() == Some("mpd") {
            return Err(unsupported("DASH manifests"));
        }
        let text = main_source.get("text").cloned().unwrap_or(Value::Null);
        if text.as_object().is_some_and(|tracks| !tracks.is_empty()) {
            return Err(unsupported("subtitle tracks"));
        }
        let manifest = Url::parse(main_url)?;
        let stream =
            extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, Some("hls"), true)?;

        let title = info
            .get("headline")
            .filter(|value| value.is_string())
            .cloned();
        let uploader = info
            .get("title")
            .and_then(|title| title.get("headline"))
            .filter(|value| value.is_string())
            .cloned();
        let uploader_id = id_text(
            info.get("title")
                .and_then(|title| title.get("originProgramId")),
        )
        .map_or(Value::Null, Value::from);
        let mut fields: Vec<(&str, Value)> = vec![("id", Value::from(video_id.as_str()))];
        if let Some(title) = title {
            fields.push(("title", title));
        }
        let duration = super::clyp::float_or_none(info.get("duration"));
        if let Some(milliseconds) = duration.as_f64() {
            fields.push(("duration", Value::from(milliseconds / 1000.0)));
        }
        if let Some(uploader) = uploader {
            fields.push(("uploader", uploader));
        }
        if !uploader_id.is_null() {
            fields.push(("uploader_id", uploader_id));
        }
        for (key, source) in [
            ("episode_number", "relatedEpisodeNumber"),
            ("season_number", "relatedSeasonNumber"),
        ] {
            let number = int_field(info.get(source));
            if !number.is_null() {
                fields.push((key, number));
            }
        }
        fields.push(("formats", Value::Null));
        fields.push(("subtitles", Value::Null));

        let mut result = dict_info(url, (NAME, NAME), fields, stream.formats);
        result.subtitles = stream.subtitles;
        Ok(Some(result))
    }
}

impl Extractor for GloboArticle {
    fn key(&self) -> &'static str {
        ARTICLE
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !article_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = ARTICLE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(ARTICLE, url);
        let webpage = download_webpage(client, url, ARTICLE, &display_id)?;

        let mut video_ids: Vec<String> = Vec::new();
        for regex in VIDEO_ID_REGEXES.iter() {
            for captures in regex.captures_iter(&webpage) {
                let id = captures[1].to_owned();
                if !video_ids.contains(&id) {
                    video_ids.push(id);
                }
            }
        }
        let entries = video_ids
            .iter()
            .map(|video_id| url_entry(&format!("globo:{video_id}"), NAME))
            .collect::<Result<Vec<_>>>()?;
        let title =
            crate::html::og_search_property(&webpage, "title").map(|title| title.trim().to_owned());
        let description = crate::html::html_search_meta(&webpage, "description");

        let mut extra = Map::new();
        extra.insert(
            "description".into(),
            description.map_or(Value::Null, Value::from),
        );
        let keys = if title.is_some() {
            "id,title,description"
        } else {
            "id,description"
        };
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{keys}").into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: display_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: ARTICLE.into(),
            extractor_key: ARTICLE.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// `url_result(target, ie=ie_key)` as a playlist entry.
fn url_entry(target: &str, ie_key: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ie_key.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": ie_key,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://globoplay.globo.com/v/3607726/"));
        assert!(suitable("globo:3607726"));
        assert!(!suitable(
            "https://ge.globo.com/video/ta-na-area-10287094.ghtml"
        ));
        assert!(article_suitable(
            "https://ge.globo.com/video/ta-na-area-como-foi-assistir-ao-jogo-do-palmeiras-que-a-globo-nao-passou-10287094.ghtml"
        ));
        assert!(!article_suitable("https://globoplay.globo.com/v/3607726/"));
    }

    #[test]
    fn session_body_keeps_python_order() {
        let body = session_body("3607726", None);
        assert!(body.starts_with(
            r#"{"player_type": "mirakulo_8k_hdr", "video_id": "3607726", "quality": "max""#
        ));
        assert!(body.ends_with(r#""tz": "-03:00", "version": 1}"#));
    }
}
