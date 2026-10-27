//! Upstream `TV2DKIE` and `TV2DKBornholmPlayIE` (`yt_dlp/extractor/tv2dk.py`):
//! the Danish regional TV 2 sites' `video_player`/`simple_player` `x-data`
//! JW Player IDs (inside the first `<article>`, else the whole page) as a
//! playlist of `jwplatform:ID` results, and TV 2 Bornholm Play's
//! `specifikVideo` AJAX response (its flowplayer title and HLS, DASH and
//! direct sources).
use super::{
    Extractor, dict_format, dict_info, download_webpage, extract_m3u8_master_optional,
    extract_mpd_optional_note,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Tv2Dk;
pub struct Tv2DkBornholmPlay;

const NAME: &str = "TV2DK";
const BORNHOLM_NAME: &str = "TV2DKBornholmPlay";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:tvsyd|tv2ostjylland|tvmidtvest|tv2fyn|tv2east|tv2lorry|tv2nord|tv2kosmopol)\.dk/(?:[^/?#]+/)*(?P<id>[^/?\#&]+)",
    )
    .unwrap()
});
static BORNHOLM_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://play\.tv2bornholm\.dk/\?.*?\bid=(?P<id>\d+)").unwrap());
static PLAYER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"x-data="(?:video_player|simple_player)\((\{[^"]+\})"#).unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"title\s*:\s*(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)')"#).unwrap());
static SOURCES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)sources:\s*(\[.+?\]),").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `TV2DKBornholmPlayIE._match_id`.
pub fn bornholm_suitable(url: &str) -> bool {
    BORNHOLM_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// The player IDs: `traverse_obj(re.findall(...), (..., {js_to_json},
/// {json.loads}, ('jwpMediaId', 'videoId'), {str}))`.
fn player_ids(search_space: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for found in PLAYER.captures_iter(search_space) {
        let text = crate::js_to_json::js_to_json(&found[1]);
        let Ok(Value::Object(data)) = crate::json::from_str::<Value>(&text) else {
            continue;
        };
        for key in ["jwpMediaId", "videoId"] {
            if let Some(Value::String(id)) = data.get(key) {
                ids.push(id.clone());
            }
        }
    }
    ids
}

impl Extractor for Tv2Dk {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `traverse_obj(webpage, {find_element(tag='article')}) or webpage`:
        // the first article's content, unless empty or unparsable.
        let article = crate::html_elements::element_text_and_html_by_tag("article", &webpage)
            .ok()
            .map(|(text, _)| text)
            .filter(|text| !text.is_empty());
        let search_space = article.as_deref().unwrap_or(&webpage);
        // `playlist_from_matches(..., getter=lambda x: f'jwplatform:{x}')`:
        // ordered, deduplicated targets.
        let mut targets: Vec<String> = Vec::new();
        for id in player_ids(search_space) {
            let target = format!("jwplatform:{id}");
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        let entries = targets
            .iter()
            .map(|target| super::iltalehti::jwplatform_entry(target))
            .collect::<Result<Vec<_>>>()?;
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: video_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

fn regex_error(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{BORNHOLM_NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// The `specifikVideo` POST (`_download_json(..., data=json.dumps(...))`).
fn specifik_video(client: &Client, video_id: &str) -> Result<Value> {
    use std::io::Read;
    let url = Url::parse("https://play.tv2bornholm.dk/controls/AJAX.aspx/specifikVideo")?;
    crate::screen::screen(&format!(
        "[{BORNHOLM_NAME}] {video_id}: Downloading JSON metadata"
    ));
    let body = json!({"playlist_id": video_id, "serienavn": ""}).to_string();
    // `json.dumps`' default separators.
    let body = body.replace("\":", "\": ").replace(",\"", ", \"");
    let mut response = super::direct::post(
        client,
        &url,
        &[
            ("X-Requested-With", "XMLHttpRequest"),
            ("Content-Type", "application/json; charset=UTF-8"),
        ],
        body,
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: BORNHOLM_NAME,
            id: video_id.to_owned(),
            what: "JSON metadata",
            source: Box::new(error),
        },
        error => error,
    })?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{BORNHOLM_NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

impl Extractor for Tv2DkBornholmPlay {
    fn key(&self) -> &'static str {
        BORNHOLM_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = BORNHOLM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(BORNHOLM_NAME, url);
        let response = specifik_video(client, &video_id)?;
        let video = match response.get("d") {
            Some(Value::String(video)) => video.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "TV 2 Bornholm responses that are not strings are not supported natively"
                        .into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "{video_id}: An extractor error has occurred. (caused by KeyError('d')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let title = TITLE
            .captures(&video)
            .and_then(|found| found.name("dq").or_else(|| found.name("sq")))
            .map(|title| title.as_str().to_owned())
            .ok_or_else(|| regex_error(&video_id, "title"))?;
        let sources = SOURCES
            .captures(&video)
            .ok_or_else(|| regex_error(&video_id, "sources"))?;
        let sources: Value = crate::json::from_str(&crate::js_to_json::js_to_json(&sources[1]))
            .map_err(|_| {
                Error::Reported(format!(
                    "[{BORNHOLM_NAME}] {video_id}: Failed to parse JSON"
                ))
            })?;
        let mut formats = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for source in sources.as_array().into_iter().flatten() {
            let Some(src) = source
                .get("src")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            if seen.contains(&src) {
                continue;
            }
            seen.push(src.clone());
            let ext = super::direct::determine_ext(&src);
            let src_type = source.get("type").and_then(Value::as_str);
            let manifest = Url::parse(&src).ok();
            if src_type == Some("application/x-mpegurl") || ext.as_deref() == Some("m3u8") {
                if let Some(stream) = manifest.and_then(|manifest| {
                    extract_m3u8_master_optional(
                        client,
                        &manifest,
                        BORNHOLM_NAME,
                        &video_id,
                        Some("hls"),
                        false,
                        false,
                    )
                }) {
                    formats.extend(stream.formats);
                }
            } else if src_type == Some("application/dash+xml") || ext.as_deref() == Some("mpd") {
                if let Some(stream) = manifest.and_then(|manifest| {
                    extract_mpd_optional_note(
                        client,
                        &manifest,
                        BORNHOLM_NAME,
                        &video_id,
                        Some("dash"),
                        "Downloading MPD manifest",
                    )
                }) {
                    formats.extend(stream.formats);
                }
            } else {
                formats.push(dict_format(
                    vec![("url", src.into())],
                    BORNHOLM_NAME,
                    &video_id,
                )?);
            }
        }
        Ok(Some(dict_info(
            url,
            (BORNHOLM_NAME, BORNHOLM_NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.tvsyd.dk/nyheder/28-10-2019/1930/1930-28-okt-2019?autoplay=1#player"
        ));
        assert!(suitable(
            "https://www.tv2kosmopol.dk/metropolen/chaufforer-beordres"
        ));
        assert!(!suitable("https://www.tv2.dk/nyheder/x"));
        assert!(bornholm_suitable(
            "http://play.tv2bornholm.dk/?area=specifikTV&id=781021"
        ));
        assert!(!bornholm_suitable("http://play.tv2bornholm.dk/?area=x"));
    }

    #[test]
    fn player_ids_keep_both_keys_in_order() {
        let page = r#"<div x-data="video_player({jwpMediaId: 'a1', videoId: 'b2'})"></div><div x-data="simple_player({videoId: 3})"></div><div x-data="video_player({bad)"></div>"#;
        assert_eq!(player_ids(page), ["a1", "b2"]);
    }
}
