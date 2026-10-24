//! Upstream `VidyardIE` (`yt_dlp/extractor/vidyard.py`): the
//! `play.vidyard.com/player/ID.json` payload's chapters, each with its HLS
//! (the `auto` master, else every variant) and progressive formats, VTT
//! captions and the `play.vidyard.com/video/ID` metadata (title, duration,
//! thumbnail, chapters); several chapters are a playlist. Generic pages'
//! iframe, inline/lightbox and script embeds are found through
//! [`embed_urls`].
use super::clyp::{float_or_none, key_error};
use super::json_ld::url_or_none;
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Vidyard;

pub(crate) const NAME: &str = "Vidyard";
pub(crate) const KEY: &str = "Vidyard";

/// `VidyardBaseIE._HEADERS`.
const REFERER: &str = "https://play.vidyard.com/";

/// `_VALID_URL`, tried in order by `_match_valid_url`.
static VALID_URLS: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        r"^https?://[\w-]+(?:\.hubs)?\.vidyard\.com/watch/(?P<id>[\w-]+)",
        r"^https?://(?:embed|share)\.vidyard\.com/share/(?P<id>[\w-]+)",
        r"^https?://play\.vidyard\.com/(?:player/)?(?P<id>[\w-]+)",
    ]
    .map(|pattern| Regex::new(pattern).unwrap())
});

/// `_match_id`'s ID of a suitable URL.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URLS
        .iter()
        .find_map(|regex| regex.captures(url))
        .map(|found| found["id"].to_owned())
}

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    temp_id(url).is_some()
}

/// `VidyardIE._extract_embed_urls` under `orderedSet`: `_EMBED_REGEX`
/// iframes (joined to the page URL and kept when suitable), then
/// `vidyard-player-embed` `<img>`/`<div>` `data-uuid`s and
/// `vidyard_embed_code_ID` scripts as `https://play.vidyard.com/ID`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    static IFRAME: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<iframe[^>]* src=["'](?P<url>(?:https?:)?//play\.vidyard\.com/[\w-]+)"#)
            .unwrap()
    });
    // The quote backreference (`\2`) as an alternation of both quotes.
    static INLINE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"<(?:img|div)[^>]* class=(?:"(?:[^>"']* )?vidyard-player-embed(?: [^>"']*)?"|'(?:[^>"']* )?vidyard-player-embed(?: [^>"']*)?')[^>]+>"#,
        )
        .unwrap()
    });
    static SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<script[^>]* id=["']vidyard_embed_code_([\w-]+)["']"#).unwrap()
    });
    let mut found: Vec<Url> = Vec::new();
    let mut push = |embed: Url| {
        if !found.contains(&embed) {
            found.push(embed);
        }
    };
    for captures in IFRAME.captures_iter(webpage) {
        let Ok(embed) = url.join(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if suitable(embed.as_str()) {
            push(embed);
        }
    }
    for element in INLINE.find_iter(webpage) {
        let attributes = crate::html_elements::extract_attributes(element.as_str());
        if let Some(Value::String(uuid)) = attributes.get("data-uuid")
            && !uuid.is_empty()
            && let Ok(embed) = Url::parse(&format!("https://play.vidyard.com/{uuid}"))
        {
            push(embed);
        }
    }
    for captures in SCRIPT.captures_iter(webpage) {
        if let Ok(embed) = Url::parse(&format!("https://play.vidyard.com/{}", &captures[1])) {
            push(embed);
        }
    }
    found
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Vidyard {what} are not supported natively"))
}

/// `mimetype2ext(mime, default=None)`.
fn mimetype2ext(mime: Option<&Value>) -> Value {
    const MISSING: &str = "\u{0}";
    match mime {
        Some(Value::String(mime)) => {
            let ext = super::direct::mimetype2ext_or(mime, Some(MISSING));
            if ext == MISSING {
                Value::Null
            } else {
                ext.into()
            }
        }
        _ => Value::Null,
    }
}

/// `_get_formats_and_subtitles(sources, video_id)`.
fn formats_and_subtitles(
    client: &Client,
    sources: &Value,
    video_id: &str,
) -> Result<(Vec<crate::VideoFormat>, crate::Subtitles)> {
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    let add_hls = |manifest: &str,
                   formats: &mut Vec<crate::VideoFormat>,
                   subtitles: &mut crate::Subtitles|
     -> Result<()> {
        let manifest = Url::parse(manifest)?;
        if let Some(mut stream) = super::extract_m3u8_master_optional_headers_id(
            client,
            &manifest,
            NAME,
            video_id,
            Some("hls"),
            &[("Referer", REFERER)],
        ) {
            formats.append(&mut stream.formats);
            merge_subtitles(subtitles, std::mem::take(&mut stream.subtitles));
        }
        Ok(())
    };
    let sources = sources.as_object();
    // `sources.pop('hls', None)`; a list's (or dict's) items.
    let hls_list: Vec<&Value> = match sources.and_then(|sources| sources.get("hls")) {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    let url_of = |item: &Value| {
        item.get("url")
            .and_then(Value::as_str)
            .and_then(url_or_none)
    };
    if let Some(master) = hls_list
        .iter()
        .filter(|item| item.get("profile").and_then(Value::as_str) == Some("auto"))
        .find_map(|item| url_of(item))
    {
        add_hls(&master, &mut formats, &mut subtitles)?;
    }
    // The variants duplicate a master's formats.
    if formats.is_empty() {
        for variant in hls_list.iter().filter_map(|item| url_of(item)) {
            add_hls(&variant, &mut formats, &mut subtitles)?;
        }
    }
    for (source_type, source_list) in sources
        .into_iter()
        .flatten()
        .filter(|(key, _)| *key != "hls")
    {
        let items: Vec<&Value> = match source_list {
            Value::Array(items) => items.iter().collect(),
            Value::Object(items) => items.values().collect(),
            _ => Vec::new(),
        };
        for source in items {
            let Some(Value::String(media)) = source.get("url") else {
                continue;
            };
            if url_or_none(media).is_none() {
                continue;
            }
            let profile = match source.get("profile") {
                None | Some(Value::Null) => None,
                Some(Value::String(profile)) => Some(profile.as_str()),
                Some(_) => return Err(unsupported("source profiles that are not strings")),
            };
            let format_id = ["http", source_type.as_str(), profile.unwrap_or_default()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("-");
            let mut fields = vec![
                ("url", Value::from(media.as_str())),
                ("ext", mimetype2ext(source.get("mimeType"))),
                ("format_id", format_id.into()),
            ];
            let (width, height) = profile.map_or((None, None), super::html5::parse_resolution);
            if let Some(width) = width {
                fields.push(("width", width.into()));
            }
            if let Some(height) = height {
                fields.push(("height", height.into()));
            }
            formats.push(dict_format(fields, NAME, video_id)?);
        }
    }
    // `_remove_duplicate_formats`: the first format of each URL.
    let mut seen: Vec<String> = Vec::new();
    formats.retain(|format| {
        let url = format.url.to_string();
        if seen.contains(&url) {
            false
        } else {
            seen.push(url);
            true
        }
    });
    Ok((formats, subtitles))
}

/// `_merge_subtitles(source, target=target)`: tracks whose URL a language
/// already has are dropped.
fn merge_subtitles(target: &mut crate::Subtitles, source: crate::Subtitles) {
    for (language, tracks) in source.entries {
        match target.entries.iter_mut().find(|(key, _)| *key == language) {
            Some((_, existing)) => {
                let known: Vec<Url> = existing.iter().map(|track| track.url.clone()).collect();
                existing.extend(
                    tracks
                        .into_iter()
                        .filter(|track| !known.contains(&track.url)),
                );
            }
            None => target.entries.push((language, tracks)),
        }
    }
}

/// `_get_direct_subtitles(caption_json)`: `vttUrl` captions by language
/// (else `und`) with their names.
fn direct_subtitles(captions: Option<&Value>) -> Result<crate::Subtitles> {
    let mut subtitles = crate::Subtitles::default();
    let items: Vec<&Value> = match captions {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    for caption in items {
        let Some(Value::String(vtt)) = caption.get("vttUrl") else {
            continue;
        };
        if url_or_none(vtt).is_none() {
            continue;
        }
        let language = match caption.get("language") {
            Some(Value::String(language)) if !language.is_empty() => language.clone(),
            None | Some(Value::Null) | Some(Value::String(_)) | Some(Value::Bool(false)) => {
                "und".into()
            }
            Some(_) => return Err(unsupported("caption languages that are not strings")),
        };
        let name = match caption.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(name)) => Some(name.clone()),
            Some(_) => return Err(unsupported("caption names that are not strings")),
        };
        let track_url = Url::parse(vtt.trim())?;
        let ext = super::direct::determine_ext(track_url.as_str())
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        let track = crate::SubtitleFormat {
            name,
            url: track_url,
            ext,
            protocol: None,
            fragments: Vec::new(),
            extra: Default::default(),
        };
        match subtitles
            .entries
            .iter_mut()
            .find(|(key, _)| key.as_deref() == Some(language.as_str()))
        {
            Some((_, tracks)) => tracks.push(track),
            None => subtitles.entries.push((Some(language), vec![track])),
        }
    }
    Ok(subtitles)
}

/// An ordered dict update (`{**a, **b}`): a known key keeps its place.
fn put(fields: &mut Vec<(&'static str, Value)>, key: &'static str, value: Value) {
    match fields.iter_mut().find(|(name, _)| *name == key) {
        Some((_, existing)) => *existing = value,
        None => fields.push((key, value)),
    }
}

/// `_get_additional_metadata(video_id)` (non-fatal): the video endpoint's
/// title, duration, thumbnail and chapters.
fn additional_metadata(client: &Client, facade: &str) -> Result<Vec<(&'static str, Value)>> {
    let api = Url::parse(&format!("https://play.vidyard.com/video/{facade}"))?;
    let data = match super::download_json_note(
        client,
        &api,
        NAME,
        facade,
        "Downloading additional metadata",
    ) {
        Ok(value) => Some(value),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {facade}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            None
        }
        // Python's `JSONDecodeError` text for a body that does not start a
        // JSON value; other parse failures keep the parser's message.
        Err(Error::Reported(message)) if message.contains(": Failed to parse JSON ") => {
            let python = [
                "(caused by expected value at line 1 column 1)",
                "(caused by EOF while parsing a value at line 1 column 0)",
            ]
            .iter()
            .any(|cause| message.ends_with(cause));
            crate::screen::warning(&if python {
                format!(
                    "[{NAME}] {facade}: Failed to parse JSON: Expecting value in '': line 1 column 1 (char 0)"
                )
            } else {
                message
            });
            None
        }
        Err(error) => return Err(error),
    };
    let data = data.as_ref();
    let mut fields = Vec::new();
    if let Some(Value::String(title)) = data.and_then(|data| data.get("name")) {
        fields.push(("title", Value::from(title.as_str())));
    }
    let duration = int_or_none(data.and_then(|data| data.get("seconds")));
    if !duration.is_null() {
        fields.push(("duration", duration));
    }
    // `('thumbnailUrl', {'url': {url_or_none}}, all)`: always a list.
    let thumbnails: Vec<Value> = data
        .and_then(|data| data.get("thumbnailUrl"))
        .and_then(Value::as_str)
        .and_then(url_or_none)
        .map(|thumbnail| serde_json::json!({"url": thumbnail}))
        .into_iter()
        .collect();
    fields.push(("thumbnails", Value::Array(thumbnails)));
    let sections: Vec<&Value> = match data.and_then(|data| data.get("videoSections")) {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    let chapters: Vec<Value> = sections
        .into_iter()
        .filter_map(|section| {
            let start = float_or_none(section.get("milliseconds"));
            let start = start.as_f64()?;
            let mut chapter = Map::new();
            if let Some(Value::String(title)) = section.get("title") {
                chapter.insert("title".into(), title.as_str().into());
            }
            chapter.insert("start_time".into(), Value::from(start / 1000.0));
            Some(Value::Object(chapter))
        })
        .collect();
    if !chapters.is_empty() {
        fields.push(("chapters", Value::Array(chapters)));
    }
    Ok(fields)
}

/// `_process_video_json(json_data, video_id)`.
fn process_video_json(
    client: &Client,
    data: &Value,
    video_id: &str,
    page: &Url,
) -> Result<VideoInfo> {
    let sources = data
        .get("sources")
        .ok_or_else(|| key_error(video_id, "sources"))?;
    let (formats, mut subtitles) = formats_and_subtitles(client, sources, video_id)?;
    merge_subtitles(&mut subtitles, direct_subtitles(data.get("captions"))?);
    let facade = match data.get("facadeUuid") {
        Some(Value::String(facade)) => facade.clone(),
        Some(_) => return Err(unsupported("facade UUIDs that are not strings")),
        None => return Err(key_error(video_id, "facadeUuid")),
    };
    let mut fields = additional_metadata(client, &facade)?;
    put(&mut fields, "id", facade.as_str().into());
    match data.get("videoId") {
        Some(Value::Bool(flag)) => {
            put(
                &mut fields,
                "display_id",
                if *flag { "True" } else { "False" }.into(),
            );
        }
        Some(Value::Number(number)) if number.is_i64() || number.is_u64() => {
            put(&mut fields, "display_id", number.to_string().into());
        }
        _ => {}
    }
    if let Some(Value::String(title)) = data.get("name") {
        put(&mut fields, "title", title.as_str().into());
    }
    if let Some(Value::String(description)) = data.get("description") {
        let description = crate::html::unescape_html(description);
        if !description.is_empty() {
            put(&mut fields, "description", description.into());
        }
    }
    let milliseconds = float_or_none(data.get("milliseconds"));
    let duration = match milliseconds.as_f64() {
        Some(milliseconds) => Value::from(milliseconds / 1000.0),
        None => int_or_none(data.get("seconds")),
    };
    if !duration.is_null() {
        put(&mut fields, "duration", duration);
    }
    let thumbnails: Vec<Value> = ["small", "normal"]
        .into_iter()
        .filter_map(|size| {
            data.get("thumbnailUrls")?
                .get(size)?
                .as_str()
                .and_then(url_or_none)
        })
        .map(|thumbnail| serde_json::json!({"url": thumbnail}))
        .collect();
    if !thumbnails.is_empty() {
        put(&mut fields, "thumbnails", Value::Array(thumbnails));
    }
    let tag_items: Vec<&Value> = match data.get("tags") {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    let tags: Vec<Value> = tag_items
        .into_iter()
        .filter_map(|tag| tag.get("name").filter(|name| name.is_string()).cloned())
        .collect();
    if !tags.is_empty() {
        put(&mut fields, "tags", Value::Array(tags));
    }
    fields.push(("formats", Value::Null));
    fields.push(("subtitles", Value::Null));
    fields.push(("http_headers", serde_json::json!({"Referer": REFERER})));
    let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
    let shape = format!(
        "dict;{};chapter=title,start_time;thumbnail=url;subtitle=url,name",
        keys.join(",")
    );
    let info = dict_info(page, (NAME, KEY), fields, formats);
    let mut info = crate::info_order::mark_info(info, &shape);
    info.subtitles = subtitles;
    Ok(info)
}

impl Extractor for Vidyard {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(video_id) = temp_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_fetch_video_json`: `['payload']`.
        let api = Url::parse(&format!("https://play.vidyard.com/player/{video_id}.json"))?;
        let data = download_json(client, &api, NAME, &video_id)?;
        let payload = data
            .get("payload")
            .ok_or_else(|| key_error(&video_id, "payload"))?;
        let chapters = payload
            .get("chapters")
            .ok_or_else(|| key_error(&video_id, "chapters"))?;
        let Value::Array(chapters) = chapters else {
            return Err(unsupported("chapter lists that are not lists"));
        };
        if let [chapter] = chapters.as_slice() {
            let info = process_video_json(client, chapter, &video_id, url)?;
            return Ok(Some(ExtractResult::Video(info)));
        }
        let entries = chapters
            .iter()
            .map(|chapter| process_video_json(client, chapter, &video_id, url))
            .collect::<Result<Vec<_>>>()?;
        // `playlist_result(entries, str(video_json['playerUuid']),
        // video_json.get('name'))`.
        let playlist_id = match payload.get("playerUuid") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(number)) => number.to_string(),
            Some(_) => return Err(unsupported("player UUIDs that are not strings")),
            None => return Err(key_error(&video_id, "playerUuid")),
        };
        let title = match payload.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => Some(title.clone()).filter(|title| !title.is_empty()),
            Some(_) => return Err(unsupported("player names that are not strings")),
        };
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            if title.is_some() {
                "playlist-dict;id,title"
            } else {
                "playlist-dict;id"
            }
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: KEY.into(),
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
        for (url, id) in [
            (
                "https://vyexample03.hubs.vidyard.com/watch/oTDMPlUv--51Th455G5u7Q",
                "oTDMPlUv--51Th455G5u7Q",
            ),
            (
                "https://share.vidyard.com/watch/PaQzDAT1h8JqB8ivEu2j6Y?",
                "PaQzDAT1h8JqB8ivEu2j6Y",
            ),
            (
                "http://share.vidyard.com/share/diYeo6YR2yiGgL8odvS8Ri",
                "diYeo6YR2yiGgL8odvS8Ri",
            ),
            (
                "https://play.vidyard.com/iDqTwWGrd36vaLuaCY3nTs.html",
                "iDqTwWGrd36vaLuaCY3nTs",
            ),
            (
                "https://play.vidyard.com/player/7GAApnNNbcZZ46k6JqJQSh.json?disable_analytics=0",
                "7GAApnNNbcZZ46k6JqJQSh",
            ),
            (
                "https://play.vidyard.com/qhMAu5A76GZVrFzOPgSf9A/type/standalone",
                "qhMAu5A76GZVrFzOPgSf9A",
            ),
        ] {
            assert_eq!(temp_id(url).as_deref(), Some(id), "{url}");
        }
        assert!(!suitable("https://vidyard.com/watch/x"));
        assert!(!suitable("https://cdn.vidyard.com/videos/x"));
    }

    #[test]
    fn embeds_follow_upstream_patterns() {
        let page = Url::parse("http://example.test/page").unwrap();
        let html = r#"<iframe class="v" src="//play.vidyard.com/AAA.html?x"></iframe>
<img style="x" class="vidyard-player-embed" src="https://play.vidyard.com/BBB.jpg" data-uuid="BBB" data-v="4" />
<div class='a vidyard-player-embed b' data-uuid='CCC'></div>
<script type="text/javascript" id="vidyard_embed_code_DDD" src="//play.vidyard.com/DDD.js"></script>
<iframe src="https://play.vidyard.com/AAA"></iframe>"#;
        let found: Vec<String> = embed_urls(&page, html).iter().map(Url::to_string).collect();
        assert_eq!(
            found,
            [
                "http://play.vidyard.com/AAA",
                "https://play.vidyard.com/AAA",
                "https://play.vidyard.com/BBB",
                "https://play.vidyard.com/CCC",
                "https://play.vidyard.com/DDD",
            ]
        );
    }
}
