//! Upstream `RCSBaseIE`, `RCSEmbedsIE`, `RCSIE` and `RCSVariousIE`
//! (`yt_dlp/extractor/rcs.py`): the `video-json` (or the page's
//! `##start-video##` block) of a Corriere/Gazzetta/RCS video, its HLS
//! renditions with the plain-HTTP MP4 equivalents (checked with a HEAD
//! request for the file size) and the MP3 audio. Embeds of other RCS players
//! are followed as `url_transparent` results.
use super::{
    Extractor, dict_format, dict_info, download_json, download_webpage,
    extract_m3u8_master_optional,
};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct RCSEmbeds;
pub struct RCS;
pub struct RCSVarious;

const BASE_NAME: &str = "RCS";
const EMBEDS_NAME: &str = "RCSEmbeds";
const VARIOUS_NAME: &str = "RCSVarious";

static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}").unwrap()
});
static RCS_ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\w-]+-\d{10}").unwrap());

static EMBEDS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?P<vid>video)\.(?P<cdn>(?:rcs|(?:corriere\w+\.)?corriere|(?:gazzanet\.)?gazzetta)\.it)/video-embed/(?P<id>[^/=&\?]+?)(?:$|\?)",
    )
    .unwrap()
});
static RCS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?P<vid>video|viaggi)\.(?P<cdn>(?:corrieredelmezzogiorno\.|corrieredelveneto\.|corrieredibologna\.|corrierefiorentino\.)?corriere\.it|(?:gazzanet\.)?gazzetta\.it)/[^?#]+?/(?P<id>[^/\?]+)(?:\?|/?$)",
    )
    .unwrap()
});
static VARIOUS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://www\.(?P<cdn>leitv\.it|youreporter\.it|amica\.it)/(?:[^/]+/)?(?P<id>[^/]+?)(?:$|\?|/)")
        .unwrap()
});
// `_EMBED_REGEX`: a `data-frame-src=` or `<iframe ... src=` RCS embed (one
// alternative per quote, as upstream's `(["\'])` backreference).
static EMBED_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:data-frame-src=|<iframe[^\n]+src=)(?:"(?P<url1>(?:https?:)?//video\.(?:rcs|(?:corriere\w+\.)?corriere|(?:gazzanet\.)?gazzetta)\.it/video-embed/.+?)"|'(?P<url2>(?:https?:)?//video\.(?:rcs|(?:corriere\w+\.)?corriere|(?:gazzanet\.)?gazzetta)\.it/video-embed/.+?)')"#,
    )
    .unwrap()
});
// `(["\'])(?P<url>...)\1;` as two alternatives.
static JSON_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"url\s*=\s*(?:"(?P<url1>(?:https?:)?//video\.rcs\.it/fragment-includes/video-includes/[^"]+?\.json)"|'(?P<url2>(?:https?:)?//video\.rcs\.it/fragment-includes/video-includes/[^']+?\.json)')\s*;"#,
    )
    .unwrap()
});
/// `_search_json('##start-video##', ..., end_pattern='##end-video##', js_to_json)`.
static START_VIDEO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)##start-video##\s*(\{.*\})\s*##end-video##").unwrap());
static MIGRATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:https?:)?//(?P<host>[\w\.\-]+)\.net/i(?P<path>.+)$").unwrap());
static HLS_HOST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(https?://[^/]+)/hls/([^?#]+?\.mp4).+").unwrap());

/// `_MIGRATION_MAP`.
const MIGRATION: &[(&str, &str)] = &[
    ("videoamica-vh.akamaihd", "amica"),
    ("media2-amica-it.akamaized", "amica"),
    ("corrierevam-vh.akamaihd", "corriere"),
    ("media2vam-corriere-it.akamaized", "corriere"),
    ("cormezzogiorno-vh.akamaihd", "corrieredelmezzogiorno"),
    (
        "media2vam-mezzogiorno-corriere-it.akamaized",
        "corrieredelmezzogiorno",
    ),
    ("corveneto-vh.akamaihd", "corrieredelveneto"),
    (
        "media2vam-veneto-corriere-it.akamaized",
        "corrieredelveneto",
    ),
    ("corbologna-vh.akamaihd", "corrieredibologna"),
    (
        "media2vam-bologna-corriere-it.akamaized",
        "corrieredibologna",
    ),
    ("corfiorentino-vh.akamaihd", "corrierefiorentino"),
    (
        "media2vam-fiorentino-corriere-it.akamaized",
        "corrierefiorentino",
    ),
    ("corinnovazione-vh.akamaihd", "corriereinnovazione"),
    ("media2-gazzanet-gazzetta-it.akamaized", "gazzanet"),
    ("videogazzanet-vh.akamaihd", "gazzanet"),
    ("videogazzaworld-vh.akamaihd", "gazzaworld"),
    ("gazzettavam-vh.akamaihd", "gazzetta"),
    ("media2vam-gazzetta-it.akamaized", "gazzetta"),
    ("videoiodonna-vh.akamaihd", "iodonna"),
    ("media2-leitv-it.akamaized", "leitv"),
    ("videoleitv-vh.akamaihd", "leitv"),
    ("videoliving-vh.akamaihd", "living"),
    ("media2-living-corriere-it.akamaized", "living"),
    ("media2-oggi-it.akamaized", "oggi"),
    ("videooggi-vh.akamaihd", "oggi"),
    ("media2-quimamme-it.akamaized", "quimamme"),
    ("quimamme-vh.akamaihd", "quimamme"),
    ("videorunning-vh.akamaihd", "running"),
    ("media2-style-corriere-it.akamaized", "style"),
    ("style-vh.akamaihd", "style"),
    ("videostyle-vh.akamaihd", "style"),
    ("media2-stylepiccoli-it.akamaized", "stylepiccoli"),
    ("stylepiccoli-vh.akamaihd", "stylepiccoli"),
    ("doveviaggi-vh.akamaihd", "viaggi"),
    ("media2-doveviaggi-it.akamaized", "viaggi"),
    ("media2-vivimilano-corriere-it.akamaized", "vivimilano"),
    ("vivimilano-vh.akamaihd", "vivimilano"),
    ("media2-youreporter-it.akamaized", "youreporter"),
];

/// `_MIGRATION_MAP[host]`.
fn migration(host: &str) -> Option<&'static str> {
    MIGRATION
        .iter()
        .find(|(key, _)| *key == host)
        .map(|(_, value)| *value)
}

/// `RCSEmbedsIE._match_valid_url`.
pub fn embeds_suitable(url: &str) -> bool {
    EMBEDS_URL.is_match(url)
}

/// `RCSIE._match_valid_url` (`(?!video-embed/)`).
pub fn rcs_suitable(url: &str) -> bool {
    RCS_URL.is_match(url)
        && !url.split_once("://").is_some_and(|(_, rest)| {
            rest.split_once('/')
                .is_some_and(|(_, path)| path.starts_with("video-embed/"))
        })
}

/// `RCSVariousIE._match_valid_url`.
pub fn various_suitable(url: &str) -> bool {
    VARIOUS_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("RCS {what} are not supported natively"))
}

/// `clean_html(value) or ...`: a non-empty string's cleaned text.
fn clean_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(crate::html::clean_html)
        .filter(|text| !text.is_empty())
}

/// `url_transparent` to an RCS embed (`RCSEmbedsIE`'s URL).
fn transparent(embed: &str) -> ExtractResult {
    let page = Url::parse(embed).expect("RCS embed URL");
    let mut follow = super::direct::build_at(page.clone(), &page, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow.extra.insert(
        crate::FOLLOW_TRANSPARENT.into(),
        json!({"url": embed, "fields": {}}),
    );
    ExtractResult::Video(follow)
}

/// `_get_video_src`: the media files' `(type, url, bitrate)` after the CDN
/// migrations.
fn video_sources(video: &Value, video_id: &str) -> Result<Vec<(String, String, Value)>> {
    let mut sources = Vec::new();
    let profile = video.get("mediaProfile");
    let geoblocking = profile
        .and_then(|profile| profile.get("geoblocking"))
        .is_some_and(super::nuxt::truthy);
    let files = profile
        .and_then(|profile| profile.get("mediaFile"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|file| file.get("mimeType").is_some_and(super::nuxt::truthy));
    for source in files {
        let Some(mut url) = source
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Err(unsupported("media files without a string value"));
        };
        for (from, to) in [
            (
                "media2vam.corriere.it.edgesuite.net",
                "media2vam-corriere-it.akamaized.net",
            ),
            (
                "media.youreporter.it.edgesuite.net",
                "media-youreporter-it.akamaized.net",
            ),
            (
                "corrierepmd.corriere.it.edgesuite.net",
                "corrierepmd-corriere-it.akamaized.net",
            ),
            (
                "media2vam-corriere-it.akamaized.net/fcs.quotidiani/vr/videos/",
                "video.corriere.it/vr360/videos/",
            ),
            ("http://", "https://"),
        ] {
            url = url.replace(from, to);
        }
        let mime = source
            .get("mimeType")
            .and_then(Value::as_str)
            .ok_or_else(|| unsupported("media MIME types that are not strings"))?;
        let type_ = super::direct::mimetype2ext_or(mime, None);
        if type_ == "m3u8" && url.contains("-vh.akamaihd") {
            // "still needed for some old content": a `*.net` URL's path under
            // the migrated host's `vod` path.
            if let Some(found) = MIGRATE.captures(&url) {
                let host = &found["host"];
                let key = migration(host).ok_or_else(|| {
                    Error::Reported(format!(
                        "[{BASE_NAME}] {video_id}: An extractor error has occurred. (caused by KeyError('{host}')); {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?;
                url = format!("https://vod.rcsobjects.it/hls/{key}{}", &found["path"]);
            }
        }
        if geoblocking || (type_ == "m3u8" && url.contains("fcs.quotidiani_!")) {
            url = url.replace("vod.rcsobjects", "vod-it.rcsobjects");
        }
        if type_ == "m3u8" && url.contains("vod") {
            url = url.replace(".csmil", ".urlset");
        }
        if type_ == "mp3" {
            url = url.replace(
                "media2vam-corriere-it.akamaized.net",
                "vod.rcsobjects.it/corriere",
            );
        }
        let bitrate = source.get("bitrate").cloned().unwrap_or(Value::Null);
        sources.push((type_, url, bitrate));
    }
    Ok(sources)
}

/// `_create_formats`: the HLS renditions, then each one's MP4 equivalent
/// when the file answers a HEAD request.
fn create_formats(
    client: &Client,
    sources: &[(String, String, Value)],
    video_id: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let mut formats = Vec::new();
    for (type_, url, bitrate) in sources {
        match type_.as_str() {
            "m3u8" => {
                let Ok(manifest) = Url::parse(url) else {
                    continue;
                };
                // `_extract_m3u8_formats` (the master only, not its first variant).
                let Some(info) = extract_m3u8_master_optional(
                    client,
                    &manifest,
                    BASE_NAME,
                    video_id,
                    Some("hls"),
                    false,
                    false,
                ) else {
                    continue;
                };
                formats.extend(info.formats.iter().cloned());
                for format in &info.formats {
                    if format.vcodec.as_deref() == Some("none") {
                        continue;
                    }
                    let format_url = format.url.as_str();
                    let Some(captures) = HLS_HOST.captures(format_url) else {
                        continue;
                    };
                    let http_url = format!("{}/{}", &captures[1], &captures[2]);
                    if http_url == format_url {
                        continue;
                    }
                    let Ok(parsed) = Url::parse(&http_url) else {
                        continue;
                    };
                    // `HEADRequest` for the file size; a failure skips the format.
                    let Ok(head) = super::direct::head_with_headers(client, &parsed, &[]) else {
                        continue;
                    };
                    let size = head
                        .headers()
                        .get("content-length")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.trim().parse::<u64>().ok());
                    let mut http = format.clone();
                    http.extra.shift_remove("manifest_url");
                    http.format_id = format.format_id.replacen("hls-", "https-", 1);
                    http.url = parsed;
                    http.protocol = Some("https".into());
                    http.extra.insert(
                        "filesize_approx".into(),
                        size.map_or(Value::Null, Value::from),
                    );
                    // `http_f.update(...)`: the copy keeps its own key order.
                    crate::info_order::mark_format_update(&mut http, "filesize_approx");
                    formats.push(http);
                }
            }
            "mp3" => {
                let mut format = dict_format(
                    vec![
                        ("format_id", "https-mp3".into()),
                        ("ext", "mp3".into()),
                        ("acodec", "mp3".into()),
                        ("vcodec", "none".into()),
                        ("abr", bitrate.clone()),
                        ("url", url.clone().into()),
                    ],
                    BASE_NAME,
                    video_id,
                )?;
                // `int_or_none` abr: an integer in the dict.
                crate::info_order::mark_int_rates(&mut format);
                formats.push(format);
            }
            _ => {}
        }
    }
    Ok(formats)
}

/// `RCSBaseIE._real_extract`.
fn real_extract(
    client: &Client,
    url: &Url,
    cdn: &str,
    video_id: &str,
    extractor: (&'static str, &'static str),
) -> Result<ExtractResult> {
    let name = extractor.0;
    let mut cdn = cdn.to_owned();
    let mut video_id = video_id.to_owned();
    let mut display_id: Option<String> = None;
    let mut video_data: Option<Value> = None;
    let mut target = url.clone();
    // The page most recently downloaded (upstream's `webpage`).
    let mut page = String::new();

    if UUID_RE.is_match(&video_id) || RCS_ID_RE.is_match(&video_id) {
        target = Url::parse(&format!("https://video.{cdn}/video-json/{video_id}"))?;
    } else {
        let webpage = download_webpage(client, url, name, &video_id)?;
        let data_config = crate::html_elements::element_html_by_id("divVideoPlayer", &webpage)
            .ok()
            .flatten()
            .or_else(|| {
                crate::html_elements::elements_text_and_html_by_class("divVideoPlayer", &webpage)
                    .ok()
                    .and_then(|elements| elements.into_iter().next().map(|(_, html)| html))
            });
        if let Some(data_config) = data_config {
            let attributes = crate::html_elements::extract_attributes(&data_config);
            let config = attributes
                .get("data-config")
                .and_then(Value::as_str)
                .and_then(|text| crate::json::from_str::<Value>(text).ok())
                .unwrap_or_else(|| json!({}));
            if let Some(newspaper) = config.get("newspaper").and_then(Value::as_str)
                && !newspaper.is_empty()
            {
                cdn = format!("{newspaper}.it");
            }
            display_id = Some(video_id.clone());
            video_id = config
                .get("uuid")
                .and_then(Value::as_str)
                .filter(|uuid| !uuid.is_empty())
                .map_or_else(|| video_id.clone(), str::to_owned);
            target = Url::parse(&format!("https://video.{cdn}/video-json/{video_id}"))?;
        } else if let Some(json_url) = JSON_URL.captures(&webpage).and_then(|found| {
            found
                .name("url1")
                .or(found.name("url2"))
                .map(|url| url.as_str().to_owned())
        }) {
            let json_url = match json_url.strip_prefix("//") {
                Some(rest) => format!("https://{rest}"),
                None => json_url,
            };
            let data = download_json(client, &Url::parse(&json_url)?, name, &video_id)?;
            display_id = Some(video_id.clone());
            video_id = data
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map_or_else(|| video_id.clone(), str::to_owned);
            video_data = Some(data);
        }
        page = webpage;
    }

    if video_data.is_none() {
        let webpage = download_webpage(client, &target, name, &video_id)?;
        video_data = START_VIDEO.captures(&webpage).and_then(|found| {
            serde_json::from_str::<Value>(&crate::js_to_json::js_to_json(&found[1])).ok()
        });
        if video_data.is_none()
            && let Some(found) = EMBED_REGEX.captures(&webpage)
        {
            // `RCSEmbedsIE._extract_url`: another RCS site's embedded player.
            let embed = found
                .name("url1")
                .or(found.name("url2"))
                .map(|url| url.as_str().to_owned())
                .unwrap_or_default();
            // `_sanitize_url`: `https` and the path's basename, without the query.
            let embed = match embed.strip_prefix("//") {
                Some(rest) => format!("https://{rest}"),
                None => embed,
            };
            let embed = embed
                .split(['?', '#'])
                .next()
                .unwrap_or_default()
                .to_owned();
            return Ok(transparent(&embed));
        }
        page = webpage;
    }
    let video_data = video_data.ok_or_else(|| {
        Error::Reported(format!(
            "Video data not found in the page; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;

    let description = clean_text(video_data.get("description"))
        .or_else(|| clean_text(video_data.get("htmlDescription")))
        .or_else(|| crate::html::html_search_meta(&page, "description"))
        .or_else(|| {
            // `_html_search_meta('description', webpage)` (non-fatal).
            super::search_warning(name, "description");
            None
        });
    let uploader = video_data
        .get("provider")
        .filter(|value| super::nuxt::truthy(value))
        .cloned()
        .unwrap_or_else(|| cdn.clone().into());
    let sources = video_sources(&video_data, &video_id)?;
    let formats = create_formats(client, &sources, &video_id)?;

    Ok(ExtractResult::Video(dict_info(
        url,
        extractor,
        vec![
            ("id", video_id.clone().into()),
            ("display_id", display_id.map_or(Value::Null, Value::from)),
            (
                "title",
                video_data.get("title").cloned().unwrap_or(Value::Null),
            ),
            ("description", description.map_or(Value::Null, Value::from)),
            ("uploader", uploader),
            ("formats", Value::Null),
        ],
        formats,
    )))
}

impl Extractor for RCSEmbeds {
    fn key(&self) -> &'static str {
        EMBEDS_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = EMBEDS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(EMBEDS_NAME);
        crate::screen::screen(&format!(
            "[{EMBEDS_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let cdn = found["cdn"].to_owned();
        let id = found["id"].to_owned();
        real_extract(client, url, &cdn, &id, (EMBEDS_NAME, EMBEDS_NAME)).map(Some)
    }
}

impl Extractor for RCS {
    fn key(&self) -> &'static str {
        BASE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !rcs_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = RCS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(BASE_NAME);
        crate::screen::screen(&format!(
            "[{BASE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let cdn = found["cdn"].to_owned();
        let id = found["id"].to_owned();
        real_extract(client, url, &cdn, &id, (BASE_NAME, BASE_NAME)).map(Some)
    }
}

impl Extractor for RCSVarious {
    fn key(&self) -> &'static str {
        VARIOUS_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VARIOUS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(VARIOUS_NAME);
        crate::screen::screen(&format!(
            "[{VARIOUS_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let cdn = found["cdn"].to_owned();
        let id = found["id"].to_owned();
        real_extract(client, url, &cdn, &id, (VARIOUS_NAME, VARIOUS_NAME)).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(embeds_suitable(
            "https://video.rcs.it/video-embed/iodonna-0001585037"
        ));
        assert!(rcs_suitable(
            "https://video.corriere.it/sport/formula-1/vettel/b727632a-f9d0-11ea-91b0-38d50a849abb"
        ));
        assert!(!rcs_suitable(
            "https://video.rcs.it/video-embed/iodonna-0001585037"
        ));
        assert!(various_suitable(
            "https://www.leitv.it/benessere/mal-di-testa/"
        ));
        assert!(!various_suitable("https://www.example.com/"));
    }
}
