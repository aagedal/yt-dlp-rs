//! Upstream `SRGSSRIE` and `SRGSSRPlayIE` (`yt_dlp/extractor/srgssr.py`):
//! the integration layer's `mediaComposition` chapter of an
//! `srgssr:BU:TYPE:ID` (or `tp.srgssr.ch` player) URL, with its block-reason
//! errors (the Switzerland-only geo error), HTTP(S) resources ranked SD < HD,
//! plain HLS and Akamai HDS+HLS resources (the `akahd` token,
//! `_extract_akamai_formats_and_subtitles` without host overrides), full
//! episode podcast URLs, subtitles and metadata; play-site URLs of srf.ch,
//! rts.ch, rsi.ch, rtr.ch and swissinfo.ch are `srgssr:` `url` results.
use super::{Extractor, dict_format, dict_info, download_json, extract_m3u8_master_optional};
use crate::{Error, ExtractResult, Result, SubtitleFormat, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct SrgSsr;
pub struct SrgSsrPlay;

const NAME: &str = "SRGSSR";
const PLAY_NAME: &str = "SRGSSRPlay";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?:https?://tp\.srgssr\.ch/p(?:/[^/]+)+\?urn=urn|srgssr):(?P<bu>srf|rts|rsi|rtr|swi):(?:[^:]+:)?(?P<type>video|audio):(?P<id>[0-9a-f\-]{36}|\d+)",
    )
    .unwrap()
});
static PLAY_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www|play)\.)?(?P<bu>srf|rts|rsi|rtr|swissinfo)\.ch/play/(?:tv|radio)/(?:[^/]+/(?P<type>video|audio)/[^?]+|popup(?P<type_2>video|audio)player)\?.*?\b(?:id=|urn=urn:[^:]+:video:)(?P<id>[0-9a-f\-]{36}|\d+)",
    )
    .unwrap()
});

/// `_ERRORS`.
fn block_message(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "AGERATING12" => {
            "To protect children under the age of 12, this video is only available between 8 p.m. and 6 a.m."
        }
        "AGERATING18" => {
            "To protect children under the age of 18, this video is only available between 11 p.m. and 5 a.m."
        }
        "GEOBLOCK" => "For legal reasons, this video is only available in Switzerland.",
        "LEGAL" => "The video cannot be transmitted for legal reasons.",
        "STARTDATE" => "This video is not yet available. Please try again later.",
        _ => return None,
    })
}

/// `_DEFAULT_LANGUAGE_CODES`.
fn default_language(bu: &str) -> &'static str {
    match bu {
        "srf" => "de",
        "rts" => "fr",
        "rsi" => "it",
        "rtr" => "rm",
        _ => "en",
    }
}

/// `SRGSSRIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `SRGSSRPlayIE._match_valid_url`.
pub fn play_suitable(url: &str) -> bool {
    PLAY_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("SRG SSR {what} are not supported natively"))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// A string-or-`None` value (`str(value)` of other types is not modelled).
fn text(value: Option<&Value>, what: &str) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(unsupported(&format!("{what} that are not strings"))),
    }
}

/// `qualities(['SD', 'HD'])(quality)`.
fn quality(quality: Option<&str>) -> i64 {
    match quality {
        Some("SD") => 0,
        Some("HD") => 1,
        _ => -1,
    }
}

/// `_download_json(token_url, video_id, note, fatal=False) or {}`'s
/// `['token']['authparams']`.
fn auth_params(client: &Client, id: &str, format_id: &str) -> Result<Option<String>> {
    use std::io::Read;
    let url = Url::parse("http://tp.srgssr.ch/akahd/token?acl=*")?;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading {format_id} token"));
    let mut response = match super::direct::request(client, &url, None) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, response.headers());
    let token: Value = match crate::json::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Failed to parse JSON  (caused by {error})"
            ));
            return Ok(None);
        }
    };
    // `try_get(token, lambda x: x['token']['authparams'])`, then truthiness.
    match token.get("token").and_then(|token| token.get("authparams")) {
        Some(Value::String(params)) if !params.is_empty() => Ok(Some(params.clone())),
        None | Some(Value::Null) => Ok(None),
        Some(value) if !truthy(value) => Ok(None),
        Some(_) => Err(unsupported("token auth params that are not strings")),
    }
}

/// `_get_media_data(bu, media_type, media_id)`.
fn media_data(client: &Client, bu: &str, media_type: &str, id: &str) -> Result<Value> {
    let mut url = Url::parse(&format!(
        "https://il.srgssr.ch/integrationlayer/2.0/{bu}/mediaComposition/{media_type}/{id}.json"
    ))?;
    if media_type == "video" {
        url.query_pairs_mut().append_pair("onlyChapters", "True");
    }
    let composition = download_json(client, &url, NAME, id)?;
    let chapters = match composition.get("chapterList") {
        None => return Err(key_error(id, "chapterList")),
        Some(Value::Array(chapters)) => chapters,
        Some(_) => return Err(unsupported("chapter lists that are not lists")),
    };
    let mut found = None;
    for chapter in chapters {
        if !chapter.is_object() {
            return Err(unsupported("chapters that are not objects"));
        }
        if chapter.get("id").and_then(Value::as_str) == Some(id) {
            found = Some(chapter.clone());
            break;
        }
    }
    let Some(media) = found else {
        return Err(Error::Reported(format!(
            "[{NAME}] {id}: No media information found; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    match media.get("blockReason") {
        None | Some(Value::Null) => {}
        Some(Value::String(reason)) => {
            if let Some(message) = block_message(reason) {
                if reason == "GEOBLOCK" {
                    return Err(crate::geo::restricted(
                        &format!("[{NAME}] {id}: {message}"),
                        &["CH"],
                    ));
                }
                return Err(Error::Reported(format!(
                    "[{NAME}] {id}: {NAME} said: {message}"
                )));
            }
        }
        Some(_) => return Err(unsupported("block reasons that are not strings")),
    }
    Ok(media)
}

fn push_subtitle(subtitles: &mut Subtitles, language: String, track: SubtitleFormat) {
    let language = Some(language);
    match subtitles
        .entries
        .iter_mut()
        .find(|(key, _)| *key == language)
    {
        Some((_, existing)) => existing.push(track),
        None => subtitles.entries.push((language, vec![track])),
    }
}

impl Extractor for SrgSsr {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_GEO_BYPASS = False` and `_GEO_COUNTRIES = ['CH']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&["CH"])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (bu, media_type, id) = (&found["bu"], &found["type"], found["id"].to_owned());
        announce(NAME, url);
        let media = media_data(client, bu, media_type, &id)?;
        let title = match media.get("title") {
            None => return Err(key_error(&id, "title")),
            Some(value) => text(Some(value), "titles")?,
        };

        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        let empty = Vec::new();
        let resources = match media.get("resourceList") {
            Some(Value::Array(resources)) => resources,
            Some(value) if truthy(value) => {
                return Err(unsupported("resource lists that are not lists"));
            }
            _ => &empty,
        };
        for source in resources {
            if !source.is_object() {
                return Err(unsupported("resources that are not objects"));
            }
            let Some(format_url) = source.get("url").filter(|url| truthy(url)) else {
                continue;
            };
            let Value::String(format_url) = format_url else {
                return Err(unsupported("resource URLs that are not strings"));
            };
            let protocol = text(source.get("protocol"), "protocols")?;
            let source_quality = text(source.get("quality"), "qualities")?;
            let encoding = text(source.get("encoding"), "encodings")?;
            // `join_nonempty(protocol, encoding, quality)`.
            let format_id = [&protocol, &encoding, &source_quality]
                .into_iter()
                .filter_map(|part| part.as_deref().filter(|part| !part.is_empty()))
                .collect::<Vec<_>>()
                .join("-");
            match protocol.as_deref() {
                Some("HDS" | "HLS") => {
                    if source.get("tokenType").and_then(Value::as_str) == Some("AKAMAI") {
                        let mut tokenized = format_url.clone();
                        if let Some(params) = auth_params(client, &id, &format_id)? {
                            tokenized.push(if tokenized.contains('?') { '&' } else { '?' });
                            tokenized.push_str(&params);
                        }
                        let (found, found_subtitles) =
                            super::akamai::formats_and_subtitles(client, &tokenized, NAME, &id)?;
                        formats.extend(found);
                        super::svt::merge_subtitles(&mut subtitles, found_subtitles);
                    } else if protocol.as_deref() == Some("HLS") {
                        let manifest = Url::parse(format_url)?;
                        if let Some(stream) = extract_m3u8_master_optional(
                            client,
                            &manifest,
                            NAME,
                            &id,
                            Some(&format_id),
                            true,
                            false,
                        ) {
                            formats.extend(stream.formats);
                            super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                        }
                    }
                }
                Some("HTTP" | "HTTPS") => formats.push(dict_format(
                    vec![
                        ("format_id", format_id.into()),
                        ("url", format_url.clone().into()),
                        ("quality", quality(source_quality.as_deref()).into()),
                    ],
                    NAME,
                    &id,
                )?),
                _ => {}
            }
        }

        // Audio segments list the whole episode's podcast URLs too.
        if super::streamable::int_or_none(media.get("position")) == 0 {
            for (key, podcast_quality) in [("podcastSdUrl", "SD"), ("podcastHdUrl", "HD")] {
                let Some(podcast_url) = media.get(key).filter(|url| truthy(url)) else {
                    continue;
                };
                let Value::String(podcast_url) = podcast_url else {
                    return Err(unsupported("podcast URLs that are not strings"));
                };
                formats.push(dict_format(
                    vec![
                        ("format_id", format!("PODCAST-{podcast_quality}").into()),
                        ("url", podcast_url.clone().into()),
                        ("quality", quality(Some(podcast_quality)).into()),
                    ],
                    NAME,
                    &id,
                )?);
            }
        }

        if media_type == "video" {
            let tracks = match media.get("subtitleList") {
                Some(Value::Array(tracks)) => tracks,
                Some(value) if truthy(value) => {
                    return Err(unsupported("subtitle lists that are not lists"));
                }
                _ => &empty,
            };
            for track in tracks {
                if !track.is_object() {
                    return Err(unsupported("subtitles that are not objects"));
                }
                let Some(track_url) = track.get("url").filter(|url| truthy(url)) else {
                    continue;
                };
                let Value::String(track_url) = track_url else {
                    return Err(unsupported("subtitle URLs that are not strings"));
                };
                let language = match track.get("locale").filter(|locale| truthy(locale)) {
                    None => default_language(bu).to_owned(),
                    Some(Value::String(locale)) => locale.clone(),
                    Some(_) => return Err(unsupported("subtitle locales that are not strings")),
                };
                let parsed = Url::parse(track_url)
                    .map_err(|_| unsupported("subtitle URLs that are not absolute"))?;
                // Processing's `determine_ext(url).lower()`.
                let ext = super::direct::determine_ext(track_url)
                    .unwrap_or_else(|| "unknown_video".into())
                    .to_lowercase();
                push_subtitle(
                    &mut subtitles,
                    language,
                    SubtitleFormat {
                        name: None,
                        url: parsed,
                        ext,
                        protocol: None,
                        fragments: Vec::new(),
                        extra: Map::new(),
                    },
                );
            }
        }

        let timestamp = text(media.get("date"), "dates")?
            .and_then(|date| crate::dates::parse_iso8601(&date))
            .map_or(Value::Null, Value::from);
        // `float_or_none(duration, 1000)`.
        let duration = match media.get("duration") {
            Some(Value::Number(number)) => number.as_f64(),
            Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
            _ => None,
        }
        .map_or(Value::Null, |value| Value::from(value / 1000.0));
        let fields = vec![
            ("id", Value::from(id.as_str())),
            ("title", title.map_or(Value::Null, Value::from)),
            (
                "description",
                text(media.get("description"), "descriptions")?.map_or(Value::Null, Value::from),
            ),
            ("timestamp", timestamp),
            (
                "thumbnail",
                text(media.get("imageUrl"), "image URLs")?.map_or(Value::Null, Value::from),
            ),
            ("duration", duration),
            ("subtitles", Value::Null),
            ("formats", Value::Null),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

impl Extractor for SrgSsrPlay {
    fn key(&self) -> &'static str {
        PLAY_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAY_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        announce(PLAY_NAME, url);
        let bu = &found["bu"][..3];
        let media_type = found
            .name("type")
            .or_else(|| found.name("type_2"))
            .map_or("", |kind| kind.as_str());
        let target = format!("srgssr:{bu}:{media_type}:{}", &found["id"]);
        // `url_result(target, 'SRGSSR')`.
        let target = Url::parse(&target)?;
        let mut info = super::direct::build_at(target.clone(), &target, String::new());
        info.formats.clear();
        info.direct = None;
        info.extra
            .insert(crate::FOLLOW_URL.into(), target.as_str().into());
        Ok(Some(ExtractResult::Video(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("srgssr:rts:video:6348260"));
        assert!(suitable(
            "srgssr:srf:audio:63cb0778-27f8-49af-9284-8c7a8c6d15fc"
        ));
        assert!(suitable(
            "https://tp.srgssr.ch/p/flash?urn=urn:rts:video:6348260"
        ));
        assert!(suitable("srgssr:swi:scc:video:42960270"));
        assert!(!suitable("srgssr:zdf:video:6348260"));
        assert!(play_suitable(
            "http://www.srf.ch/play/tv/10vor10/video/snowden-beantragt-asyl-in-russland?id=28e1a57d-5b76-4399-8ab3-9097f071e6c5"
        ));
        assert!(play_suitable(
            "https://www.srf.ch/play/tv/popupvideoplayer?id=c4dba0ca-e75b-43b2-a34f-f708a4932e01"
        ));
        assert!(play_suitable(
            "https://www.rts.ch/play/tv/19h30/video/le-19h30?urn=urn:rts:video:6348260"
        ));
        assert!(play_suitable(
            "http://play.swissinfo.ch/play/tv/business/video/why-people-were-against-tax-reforms?id=42960270"
        ));
        assert!(!play_suitable("https://www.srf.ch/play/tv/sendung/10vor10"));
    }
}
