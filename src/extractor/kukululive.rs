//! Upstream `KukuluLiveIE` (`yt_dlp/extractor/kukululive.py`): Kukulu Live
//! streams (the `fplayer` quality metadata's HLS and audio-only addresses)
//! and time-shifted recordings (the timeshift player's `fplayer_source`
//! segments: one video, or a `multi_video` playlist of parts).
use super::{Extractor, dict_format, dict_info, download_webpage, download_webpage_note};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct KukuluLive;

const NAME: &str = "KukuluLive";
const BASE: &str = "https://live.erinn.biz";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://live\.erinn\.biz/live\.php\?h(?P<id>\d+)").unwrap());
static LIVESTREAM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+timeshift\s*=\s*false").unwrap());
static SOURCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"var\s+fplayer_source\s*=").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Kukulu Live {what} are not supported natively"))
}

/// A request's `errnote` for its HTTP error.
fn errnote(error: Error, id: &str, note: &str) -> Error {
    match error {
        Error::ExtractorRequest { source, .. } => {
            Error::Reported(super::request_errnote_text(NAME, id, note, &source))
        }
        error => error,
    }
}

/// `_html_search_meta(names, html)` (not fatal): upstream's warning names
/// the first.
fn search_meta(html: &str, names: &[&str]) -> Value {
    match names
        .iter()
        .find_map(|name| crate::html::html_search_meta_clean(html, name))
    {
        Some(found) => found.into(),
        None => {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract {}; {}",
                names[0],
                crate::BUG_REPORTS_MESSAGE
            ));
            Value::Null
        }
    }
}

/// `urllib.parse.parse_qs(qs)`: blank values dropped, repeated keys grouped.
fn parse_qs(text: &str) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in url::form_urlencoded::parse(text.as_bytes()) {
        if value.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, values)) => values.push(value.into_owned()),
            None => groups.push((key.into_owned(), vec![value.into_owned()])),
        }
    }
    groups
}

/// `traverse_obj(quality_meta, (key, 0))`.
fn first<'a>(meta: &'a [(String, Vec<String>)], key: &str) -> Option<&'a str> {
    meta.iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, values)| values.first())
        .map(String::as_str)
}

/// `_get_quality_meta(video_id, desc, code, force_h264)`.
fn quality_meta(
    client: &Client,
    video_id: &str,
    desc: &str,
    code: &str,
    force_h264: bool,
) -> Result<Vec<(String, Vec<String>)>> {
    let desc = if force_h264 {
        format!("{desc} (force_h264)")
    } else {
        desc.to_owned()
    };
    let mut url = Url::parse(&format!("{BASE}/live.player.fplayer.php"))?;
    url.query_pairs_mut()
        .append_pair("hash", video_id)
        .append_pair("action", &format!("get{code}liveByAjax"));
    if force_h264 {
        url.query_pairs_mut().append_pair("force_h264", "1");
    }
    let text = download_webpage_note(
        client,
        &url,
        NAME,
        video_id,
        &format!("Downloading {desc} quality metadata"),
    )
    .map_err(|error| {
        errnote(
            error,
            video_id,
            &format!("Unable to download {desc} quality metadata"),
        )
    })?;
    Ok(parse_qs(&text))
}

/// `_add_quality_formats(formats, quality_meta)`.
fn add_quality_formats(
    formats: &mut Vec<crate::VideoFormat>,
    meta: &[(String, Vec<String>)],
    video_id: &str,
) -> Result<()> {
    let vcodec = first(meta, "vcodec");
    let quality = first(meta, "now_quality");
    let priority = match quality {
        Some("low") => 0,
        Some("h264") => 1,
        Some("high") => 2,
        _ => -1,
    };
    let text = |value: Option<&str>| value.map_or(Value::Null, Value::from);
    if let Some(hls) = first(meta, "hlsaddr").and_then(super::json_ld::url_or_none) {
        formats.push(dict_format(
            vec![
                ("format_id", text(quality)),
                ("url", hls.into()),
                ("ext", "mp4".into()),
                ("vcodec", text(vcodec)),
                ("quality", priority.into()),
            ],
            NAME,
            video_id,
        )?);
    }
    if let Some(audio) = first(meta, "hlsaddr_audioonly").and_then(super::json_ld::url_or_none) {
        let format_id = match quality {
            Some(quality) if !quality.is_empty() => format!("{quality}-audioonly"),
            _ => "audioonly".into(),
        };
        formats.push(dict_format(
            vec![
                ("format_id", format_id.into()),
                ("url", audio.into()),
                ("ext", "m4a".into()),
                ("vcodec", "none".into()),
                ("quality", priority.into()),
            ],
            NAME,
            video_id,
        )?);
    }
    Ok(())
}

/// `_search_json(r'var\s+fplayer_source\s*=', ..., contains_pattern=r'\[(?s:.+)\]',
/// transform_source=js_to_json)` (fatal).
fn fplayer_source(player_html: &str, video_id: &str) -> Result<Value> {
    let found = SOURCE.find(player_html).and_then(|found| {
        let rest = player_html[found.end()..].trim_start();
        let end = rest.rfind(']')?;
        rest.starts_with('[').then(|| &rest[..=end])
    });
    let Some(candidate) = found else {
        return Err(Error::Reported(format!(
            "[{NAME}] {video_id}: Unable to extract stream data; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let text = crate::js_to_json::js_to_json(candidate);
    serde_json::Deserializer::from_str(&crate::json::replace_lone_surrogates(&text))
        .into_iter::<Value>()
        .next()
        .and_then(std::result::Result::ok)
        .ok_or_else(|| {
            let cause = crate::json::python_raw_decode_error_text(&text).map_or_else(
                String::new,
                |error| {
                    format!(
                        " (caused by JSONDecodeError({}))",
                        crate::batch::py_repr(&error)
                    )
                },
            );
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract stream data - Failed to parse JSON{cause}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

/// One `entries(segments, playlist)` item.
fn entry(
    page: &Url,
    video_id: &str,
    title: Option<&str>,
    description: &Value,
    thumbnail: &Value,
    segment: &Value,
    part: Option<usize>,
) -> Result<VideoInfo> {
    let (id, title) = match part {
        Some(part) => (
            format!("{video_id}_{part}"),
            Value::from(format!("{} (Part {part})", title.unwrap_or("None"))),
        ),
        None => (video_id.to_owned(), title.map_or(Value::Null, Value::from)),
    };
    let file = segment
        .get("file")
        .and_then(Value::as_str)
        .ok_or_else(|| unsupported("segment files that are not strings"))?;
    let format_url = super::videopress::urljoin(Some(BASE), file)
        .ok_or_else(|| unsupported("segment files that are not URLs"))?;
    let format = dict_format(
        vec![
            ("url", format_url.into()),
            ("ext", "mp4".into()),
            ("protocol", "m3u8_native".into()),
        ],
        NAME,
        &id,
    )?;
    Ok(dict_info(
        page,
        (NAME, NAME),
        vec![
            ("id", id.into()),
            ("title", title),
            ("description", description.clone()),
            (
                "timestamp",
                super::streamable::int_or_none(segment.get("time_start")),
            ),
            ("thumbnail", thumbnail.clone()),
            ("formats", Value::Null),
        ],
        vec![format],
    ))
}

impl Extractor for KukuluLive {
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
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let html = download_webpage(client, url, NAME, &video_id)?;
        if html.contains(">タイムシフトが見つかりませんでした。<") {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: This stream has expired"
            )));
        }
        let title = crate::html_elements::elements_text_and_html_by_attribute(
            "id",
            "livetitle",
            &html.replace("<SPAN", "<span").replace("SPAN>", "span>"),
        )
        // An uncaught `HTMLParseError` is reported as its message.
        .map_err(Error::Reported)?
        .into_iter()
        .next()
        .map(|(text, _)| crate::html::clean_html(&text));
        let description = search_meta(&html, &["Description"]);
        let thumbnail = search_meta(&html, &["og:image", "twitter:image"]);

        if LIVESTREAM.is_match(&html) {
            let mut formats = Vec::new();
            for (desc, code) in [("high", "Z"), ("low", "ForceLow")] {
                let meta = quality_meta(client, &video_id, desc, code, false)?;
                add_quality_formats(&mut formats, &meta, &video_id)?;
                if desc == "high" && first(&meta, "vcodec") == Some("HEVC") {
                    let meta = quality_meta(client, &video_id, desc, code, true)?;
                    add_quality_formats(&mut formats, &meta, &video_id)?;
                }
            }
            return Ok(Some(ExtractResult::Video(dict_info(
                url,
                (NAME, NAME),
                vec![
                    ("id", video_id.into()),
                    ("title", title.map_or(Value::Null, Value::from)),
                    ("description", description),
                    ("thumbnail", thumbnail),
                    ("is_live", true.into()),
                    ("formats", Value::Null),
                ],
                formats,
            ))));
        }

        // VOD extraction.
        let mut player = Url::parse(&format!("{BASE}/live.timeshift.fplayer.php"))?;
        player.query_pairs_mut().append_pair("hash", &video_id);
        let player_html =
            download_webpage_note(client, &player, NAME, &video_id, "Downloading player html")
                .map_err(|error| errnote(error, &video_id, "Unable to download player html"))?;
        let sources: Vec<Value> = match fplayer_source(&player_html, &video_id)? {
            // `lambda _, v: v['file']`: items with a truthy file.
            Value::Array(items) => items
                .into_iter()
                .filter(|item| {
                    item.as_object()
                        .and_then(|item| item.get("file"))
                        .is_some_and(super::nuxt::truthy)
                })
                .collect(),
            _ => return Err(unsupported("stream data that is not a list")),
        };
        if sources.len() == 1 {
            return entry(
                url,
                &video_id,
                title.as_deref(),
                &description,
                &thumbnail,
                &sources[0],
                None,
            )
            .map(|info| Some(ExtractResult::Video(info)));
        }
        let entries = sources
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                entry(
                    url,
                    &video_id,
                    title.as_deref(),
                    &description,
                    &thumbnail,
                    segment,
                    Some(index + 1),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        // `playlist_result(entries, video_id, title, description, multi_video=True)`.
        let mut keys = vec!["id"];
        let mut extra = Map::new();
        let playlist_title = title.filter(|title| !title.is_empty());
        if playlist_title.is_some() {
            keys.push("title");
        }
        if !description.is_null() {
            keys.push("description");
            extra.insert("description".into(), description);
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!(
                "playlist-dict;{}{}",
                keys.join(","),
                crate::info_order::GENERATOR
            )
            .into(),
        );
        extra.insert(crate::info::MULTI_VIDEO.into(), Value::Bool(true));
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: video_id,
            title: playlist_title.unwrap_or_default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://live.erinn.biz/live.php?h675134569"));
        assert!(!suitable("https://live.erinn.biz/live.php?x675134569"));
        assert_eq!(
            parse_qs("vcodec=HEVC&now_quality=high&empty=&vcodec=x&sp=a+b"),
            vec![
                ("vcodec".to_owned(), vec!["HEVC".to_owned(), "x".to_owned()]),
                ("now_quality".to_owned(), vec!["high".to_owned()]),
                ("sp".to_owned(), vec!["a b".to_owned()]),
            ]
        );
    }
}
