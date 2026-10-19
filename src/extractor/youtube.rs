//! Narrow watch-page slice of YoutubeIE at the pinned upstream revision.
//! Bounded WEB API fallback, native HLS VOD and explicit offline JS runtime.
mod captions;
mod challenge;
mod client;
mod metadata;
mod player;
use super::{Extractor, direct, hls};
use crate::{Error, Result, StreamKind, VideoFormat, VideoInfo};
use reqwest::{
    blocking::Client,
    header::{CONTENT_ENCODING, CONTENT_TYPE},
};
use scraper::{Html, Selector};
use serde_json::Value;
use std::{io::Read, path::PathBuf};
use url::Url;

pub struct Youtube;

/// Opt-in YouTube challenge runtime. Defaults to challenge-free extraction.
#[derive(Debug, Clone, Default)]
pub struct YoutubeOptions {
    /// Explicit Deno 2.9.7 executable path; never discovered or installed.
    /// Inspected only when an eligible format has a supported URL challenge.
    pub js_runtime: Option<PathBuf>,
    /// Include translated manual captions in automatic-caption metadata.
    /// Original ASR captions/translations are always discovered without fetching.
    pub include_translated_subtitles: bool,
}

struct ConfiguredYoutube {
    include_translated_subtitles: bool,
    runtime: Option<challenge::Runtime>,
}

impl Youtube {
    pub fn with_options(options: YoutubeOptions) -> impl Extractor {
        Self::with_http_options(
            options,
            reqwest::header::HeaderMap::new(),
            Default::default(),
            Default::default(),
        )
    }

    pub(crate) fn with_http_options(
        options: YoutubeOptions,
        headers: reqwest::header::HeaderMap,
        cookies: std::sync::Arc<crate::CookieJar>,
        proxy: crate::http::TransportConfig,
    ) -> impl Extractor {
        ConfiguredYoutube {
            include_translated_subtitles: options.include_translated_subtitles,
            runtime: options
                .js_runtime
                .map(|path| challenge::Runtime::with_http_options(path, headers, cookies, proxy)),
        }
    }
}

struct Candidate {
    format_id: String,
    source: Value,
    ext: String,
    stream_kind: StreamKind,
    abr: Option<f64>,
    tbr: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
}

impl Candidate {
    fn into_format(self, url: Url) -> VideoFormat {
        VideoFormat {
            protocol: None,
            audio_group: None,
            language: None,
            format_note: None,
            audio_default: false,
            audio_autoselect: false,
            fragments: Vec::new(),
            format_id: self.format_id,
            url,
            ext: self.ext,
            stream_kind: self.stream_kind,
            abr: self.abr,
            tbr: self.tbr,
            filesize: None,
            width: self.width,
            height: self.height,
        }
    }
}

const MAX_PAGE_BYTES: u64 = 2 * 1024 * 1024;

fn unsupported(reason: &str) -> Error {
    Error::UnsupportedYoutube(reason.into())
}

fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

// Claim all URLs on these exact hosts so unsupported site pages cannot fall
// through to generic HTML5 extraction and appear to be supported YouTube videos.
fn video_id(url: &Url) -> Result<Option<String>> {
    let host = url.host_str().unwrap_or("");
    if !matches!(
        host,
        "youtube.com"
            | "www.youtube.com"
            | "m.youtube.com"
            | "music.youtube.com"
            | "youtu.be"
            | "www.youtu.be"
            | "www.youtube-nocookie.com"
            | "youtube-nocookie.com"
    ) {
        return Ok(None);
    }
    if url.query_pairs().any(|(key, _)| key == "list") {
        return Err(unsupported(
            "playlists and video URLs with list= are not supported",
        ));
    }
    let id = if matches!(host, "youtu.be" | "www.youtu.be") {
        url.path()
            .strip_prefix('/')
            .filter(|s| !s.contains('/'))
            .map(str::to_owned)
    } else if url.path() == "/watch" {
        let ids: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| key == "v")
            .map(|(_, value)| value.into_owned())
            .collect();
        if ids.len() != 1 {
            None
        } else {
            ids.into_iter().next()
        }
    } else {
        ["/shorts/", "/embed/", "/live/"]
            .iter()
            .find_map(|prefix| url.path().strip_prefix(prefix))
            .map(str::to_owned)
    };
    id.filter(|id| valid_id(id))
        .map(Some)
        .ok_or_else(|| unsupported("expected a single video URL with an 11-character ID"))
}

fn initial_player_response(html: &str) -> Result<Option<Value>> {
    let document = Html::parse_document(html);
    let scripts = Selector::parse("script").expect("static selector");
    for script in document.select(&scripts) {
        let text = script.text().collect::<String>();
        for marker in [
            "ytInitialPlayerResponse",
            "window[\"ytInitialPlayerResponse\"]",
            "window['ytInitialPlayerResponse']",
        ] {
            for (offset, _) in text.match_indices(marker) {
                // Avoid matching a longer identifier or a quoted property name.
                if offset > 0
                    && (text.as_bytes()[offset - 1].is_ascii_alphanumeric()
                        || matches!(text.as_bytes()[offset - 1], b'_' | b'$'))
                {
                    continue;
                }
                let rest = text[offset + marker.len()..].trim_start();
                let Some(json) = rest.strip_prefix('=').map(str::trim_start) else {
                    continue;
                };
                if !json.starts_with('{') {
                    return Err(unsupported("initial player response is not a JSON object"));
                }
                let mut values = serde_json::Deserializer::from_str(json).into_iter::<Value>();
                return values
                    .next()
                    .and_then(std::result::Result::ok)
                    .map(Some)
                    .ok_or_else(|| unsupported("malformed initial player response"));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
fn player_response(html: &str) -> Result<Value> {
    initial_player_response(html)?.ok_or_else(|| unsupported("no initial player response"))
}

fn parse_candidates(
    player: &Value,
    id: &str,
    allow_challenges: bool,
    api: bool,
) -> Result<(String, Vec<Candidate>)> {
    if player
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)
        != Some("OK")
    {
        return Err(unsupported(
            "video is not playable without authentication, consent, or other access handling",
        ));
    }
    let details = &player["videoDetails"];
    if details["videoId"].as_str() != Some(id) {
        return Err(unsupported(
            "player response video ID does not match the requested video",
        ));
    }
    for key in ["isLive", "isLiveContent", "isUpcoming", "isPostLiveDvr"] {
        if details[key].as_bool() == Some(true) {
            return Err(unsupported(
                "live, upcoming, and post-live videos are not supported",
            ));
        }
    }
    if player
        .pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails/isLiveNow")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err(unsupported("live videos are not supported"));
    }
    let title = details["title"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| unsupported("player response has no title"))?
        .to_owned();
    let streaming = &player["streamingData"];
    if streaming
        .get("licenseInfos")
        .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
    {
        return Err(unsupported("DRM-protected player response"));
    }
    let mut formats = Vec::new();
    for source in ["formats", "adaptiveFormats"]
        .into_iter()
        .flat_map(|key| streaming[key].as_array().into_iter().flatten())
    {
        if ["drmFamilies", "drmTrackType", "targetDurationSec"]
            .iter()
            .any(|key| source.get(*key).is_some_and(|v| !v.is_null()))
            || source.get("type").is_some_and(|value| {
                !value.is_null() && value.as_str() != Some("FORMAT_STREAM_TYPE_NORMAL")
            })
        {
            continue;
        }
        let Some(itag) = source["itag"].as_u64().filter(|i| *i > 0) else {
            continue;
        };
        if api && itag != 18 {
            continue;
        }
        let fields: serde_json::Map<_, _> = ["url", "signatureCipher", "cipher"]
            .into_iter()
            .filter_map(|key| {
                source
                    .get(key)
                    .filter(|value| !value.is_null())
                    .map(|value| (key.to_owned(), value.clone()))
            })
            .collect();
        let url_source = Value::Object(fields);
        let Ok(challenged) = challenge::has_challenges(&url_source) else {
            continue;
        };
        if challenged && !allow_challenges {
            continue;
        }
        let Some(mime) = source["mimeType"].as_str() else {
            continue;
        };
        let (kind, parameters) = mime.split_once(';').unwrap_or((mime, ""));
        let ext = match kind.trim() {
            "video/mp4" => "mp4",
            "video/webm" | "audio/webm" => "webm",
            "audio/mp4" => "m4a",
            _ => continue,
        };
        let codecs = parameters
            .split(';')
            .find_map(|p| p.trim().strip_prefix("codecs="))
            .unwrap_or("")
            .trim_matches('"');
        let codecs: Vec<_> = codecs.split(',').map(str::trim).collect();
        let video_codec = |codec: &str| {
            ["avc1", "av01", "vp8", "vp9", "vp09"].iter().any(|prefix| {
                codec
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('.'))
            })
        };
        let audio_codec = |codec: &str| {
            ["mp4a", "opus", "vorbis"].iter().any(|prefix| {
                codec
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('.'))
            })
        };
        if codecs
            .iter()
            .any(|codec| !video_codec(codec) && !audio_codec(codec))
        {
            continue;
        }
        let video = codecs.iter().any(|codec| video_codec(codec));
        let audio = codecs.iter().any(|codec| audio_codec(codec));
        let stream_kind = match (kind.trim().starts_with("audio/"), video, audio) {
            (false, true, true) => StreamKind::Standalone,
            (false, true, false) => StreamKind::VideoOnly,
            (true, false, true) => StreamKind::AudioOnly,
            _ => continue,
        };
        let rate = |key: &str| {
            source
                .get(key)
                .and_then(Value::as_f64)
                .filter(|number| number.is_finite() && *number > 0.0)
        };
        let tbr = rate("averageBitrate")
            .or_else(|| rate("bitrate"))
            .map(|number| number / 1000.0);
        if formats
            .iter()
            .any(|f: &Candidate| f.format_id == itag.to_string())
        {
            continue;
        }
        let dimension = |key: &str| {
            source[key]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
        };
        formats.push(Candidate {
            format_id: itag.to_string(),
            source: url_source,
            ext: ext.into(),
            stream_kind,
            tbr,
            abr: if stream_kind == StreamKind::AudioOnly {
                tbr
            } else {
                None
            },
            width: dimension("width"),
            height: dimension("height"),
        });
    }
    Ok((title, formats))
}

fn no_formats() -> Error {
    unsupported(
        "no eligible media formats; signature/n URL challenges require an explicit runtime; WEB DASH requires a GVS token; SABR, live and DRM are unsupported",
    )
}

// WEB HLS has an optional GVS token in the pinned upstream policy, unlike DASH.
// No DASH URL is prepared, solved or requested without token acquisition support.
fn hls_manifest(player: &Value, allow_challenges: bool) -> Option<String> {
    let url = player.pointer("/streamingData/hlsManifestUrl")?.as_str()?;
    let challenged = challenge::has_manifest_challenges(url).ok()?;
    if challenged && !allow_challenges {
        return None;
    }
    Some(url.to_owned())
}

fn manifest_formats(client: &Client, url: &Url, page: &Url) -> Result<Vec<VideoFormat>> {
    let response = direct::request(client, url, Some(page))?;
    let info = hls::from_response(client, response, url, Some(page), true, None)?
        .ok_or_else(|| unsupported("HLS manifest could not be parsed"))?;
    // Upstream names YouTube HLS formats by itag; until that is ported this
    // extractor keeps its native order (variants, then audio renditions) and
    // `hls-N`/`hls-audio-N` IDs.
    let (mut variants, mut audio): (Vec<_>, Vec<_>) =
        info.formats.into_iter().partition(|format| {
            !(format.stream_kind == crate::StreamKind::AudioOnly && format.format_note.is_some())
        });
    for (index, format) in variants.iter_mut().enumerate() {
        format.format_id = format!("hls-{index}");
    }
    for (index, format) in audio.iter_mut().enumerate() {
        format.format_id = format!("hls-audio-{index}");
    }
    let mut formats = variants;
    formats.append(&mut audio);
    for format in &mut formats {
        if let Some(group) = &mut format.audio_group {
            *group = format!("youtube-hls:{group}");
        }
    }
    Ok(formats)
}

#[cfg(test)]
fn parse_formats(player: &Value, id: &str) -> Result<(String, Vec<VideoFormat>)> {
    let (title, candidates) = parse_candidates(player, id, false, false)?;
    if candidates.is_empty() {
        return Err(no_formats());
    }
    let formats = candidates
        .into_iter()
        .map(|candidate| {
            let url = Url::parse(candidate.source["url"].as_str().unwrap())?;
            Ok(candidate.into_format(url))
        })
        .collect::<Result<_>>()?;
    Ok((title, formats))
}

#[cfg(test)]
fn extract_watch_page(client: &Client, page: &Url, id: &str) -> Result<VideoInfo> {
    extract_watch_page_with_runtime(client, page, id, None, None)
}

fn extract_watch_page_with_runtime(
    client: &Client,
    page: &Url,
    id: &str,
    runtime: Option<&challenge::Runtime>,
    player_source: Option<&player::PlayerSource>,
) -> Result<VideoInfo> {
    extract_watch_page_with_caption_options(client, page, id, runtime, player_source, false)
}

fn extract_watch_page_with_caption_options(
    client: &Client,
    page: &Url,
    id: &str,
    runtime: Option<&challenge::Runtime>,
    player_source: Option<&player::PlayerSource>,
    translated_manual: bool,
) -> Result<VideoInfo> {
    let response = direct::request(client, page, None)?;
    if direct::response_mime(&response) != "text/html" {
        return Err(unsupported("watch page is not HTML"));
    }
    if let Some(content_type) = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
    {
        for parameter in content_type.split(';').skip(1) {
            if let Some((name, value)) = parameter.trim().split_once('=')
                && name.trim().eq_ignore_ascii_case("charset")
                && !matches!(
                    value
                        .trim()
                        .trim_matches(['\'', '"'])
                        .to_ascii_lowercase()
                        .as_str(),
                    "utf-8" | "utf8" | "us-ascii"
                )
            {
                return Err(unsupported("only UTF-8/ASCII watch pages are supported"));
            }
        }
    }
    if response
        .headers()
        .get(CONTENT_ENCODING)
        .is_some_and(|v| !v.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(unsupported("compressed watch pages are not supported"));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_PAGE_BYTES)
    {
        return Err(Error::HtmlTooLarge(MAX_PAGE_BYTES));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_PAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    if bytes.len() as u64 > MAX_PAGE_BYTES {
        return Err(Error::HtmlTooLarge(MAX_PAGE_BYTES));
    }
    let html = std::str::from_utf8(&bytes).map_err(|_| unsupported("watch page is not UTF-8"))?;
    // Never use API fallback to bypass a malformed, gated, or unsupported
    // initial response. The missing-response path gets exactly one WEB request.
    let (player, api) = match initial_player_response(html)? {
        Some(player) => (player, false),
        None => (client::fetch_player(client, page, html, id)?, true),
    };
    // Eligibility and WEB API token policy precede solving and all media/manifest
    // requests. The same complete batch resolves direct and HLS path challenges.
    let (title, candidates) = parse_candidates(&player, id, runtime.is_some(), api)?;
    let manifests: Vec<_> = hls_manifest(&player, runtime.is_some())
        .into_iter()
        .collect();
    if candidates.is_empty() && manifests.is_empty() {
        return Err(no_formats());
    }
    let captions = captions::parse(&player, translated_manual)?;
    for warning in &captions.warnings {
        eprintln!("WARNING: {id}: {warning}");
    }
    let sources: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.source.clone())
        .collect();
    let (urls, manifests) =
        challenge::resolve_media_and_manifests(&sources, &manifests, html, runtime, player_source)?;
    let mut formats: Vec<_> = candidates
        .into_iter()
        .zip(urls)
        .map(|(candidate, url)| candidate.into_format(url))
        .collect();
    for format in &mut formats {
        let response = direct::probe(client, &format.url, Some(page))?;
        let media = direct::from_response(&response, &format.url)
            .ok_or_else(|| unsupported("format URL does not serve direct HTTP media"))?;
        if media.ext != format.ext {
            return Err(unsupported(
                "format MIME type disagrees with the media response",
            ));
        }
        format.url = media.url;
        format.filesize = media.filesize;
    }
    for manifest in manifests {
        formats.extend(manifest_formats(client, &manifest, page)?);
    }
    if let Some(language) = &captions.audio_language {
        for format in &mut formats {
            if format.stream_kind != StreamKind::VideoOnly && format.language.is_none() {
                format.language = Some(language.clone());
            }
        }
    }
    let first = formats.first().ok_or_else(no_formats)?;
    Ok(VideoInfo {
        protocol: None,
        metadata: metadata::parse(&player),
        automatic_captions: captions.automatic,
        subtitles: captions.subtitles,
        requested_subtitles: None,
        last_playlist_index: None,
        playlist_index: None,
        playlist_autonumber: None,
        playlist_count: None,
        n_entries: None,
        fragments: first.fragments.clone(),
        id: id.into(),
        title,
        ext: first.ext.clone(),
        url: first.url.clone(),
        webpage_url: Url::parse(&format!("https://www.youtube.com/watch?v={id}"))?,
        extractor: "youtube".into(),
        extractor_key: "Youtube".into(),
        direct: false,
        format_id: None,
        stream_kind: first.stream_kind,
        requested_formats: Vec::new(),
        filesize: first.filesize,
        width: first.width,
        height: first.height,
        formats,
    })
}

impl Extractor for Youtube {
    fn key(&self) -> &'static str {
        "Youtube"
    }
    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(id) = video_id(url)? else {
            return Ok(None);
        };
        crate::screen::set_extractor("youtube");
        let page = Url::parse(&format!("https://www.youtube.com/watch?v={id}"))?;
        extract_watch_page_with_runtime(client, &page, &id, None, None).map(Some)
    }
}

impl Extractor for ConfiguredYoutube {
    fn key(&self) -> &'static str {
        "Youtube"
    }
    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(id) = video_id(url)? else {
            return Ok(None);
        };
        crate::screen::set_extractor("youtube");
        let page = Url::parse(&format!("https://www.youtube.com/watch?v={id}"))?;
        extract_watch_page_with_caption_options(
            client,
            &page,
            &id,
            self.runtime.as_ref(),
            None,
            self.include_translated_subtitles,
        )
        .map(Some)
    }
}

/// JSON for saved-fixture comparisons: thumbnails sanitized and sorted like
/// orchestration does, without the sanitizer-assigned `id`/`resolution`.
#[cfg(test)]
pub(crate) fn comparable_info(info: &VideoInfo) -> Value {
    let mut info = info.clone();
    info.metadata.sanitize_thumbnails();
    let mut value = serde_json::to_value(&info).unwrap();
    if let Some(thumbnails) = value.get_mut("thumbnails").and_then(Value::as_array_mut) {
        for thumbnail in thumbnails {
            let object = thumbnail.as_object_mut().unwrap();
            object.remove("id");
            object.remove("resolution");
            for key in ["width", "height", "preference"] {
                object.entry(key).or_insert(Value::Null);
            }
        }
    }
    value
}

#[cfg(test)]
mod integration;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};

    const FIXTURE: &str = include_str!("../../tests/fixtures/youtube/progressive.html");
    fn player() -> Value {
        player_response(&FIXTURE.replace("{{ORIGIN}}", "https://media.example")).unwrap()
    }

    #[test]
    fn exact_hosts_and_supported_video_url_shapes() {
        for input in [
            "https://www.youtube.com/watch?v=BaW_jenozKc&t=3",
            "https://youtu.be/BaW_jenozKc",
            "https://m.youtube.com/shorts/BaW_jenozKc",
            "https://www.youtube-nocookie.com/embed/BaW_jenozKc",
            "https://music.youtube.com/watch?v=BaW_jenozKc",
            "https://youtube.com/live/BaW_jenozKc",
        ] {
            assert_eq!(
                video_id(&Url::parse(input).unwrap()).unwrap().as_deref(),
                Some("BaW_jenozKc")
            );
        }
        for input in [
            "https://youtube.com.evil.invalid/watch?v=BaW_jenozKc",
            "https://example.com/watch?v=BaW_jenozKc",
        ] {
            assert!(video_id(&Url::parse(input).unwrap()).unwrap().is_none());
        }
        for input in [
            "https://youtube.com/playlist?list=foo",
            "https://youtube.com/watch?v=BaW_jenozKc&list=foo",
            "https://youtu.be/BaW_jenozKc/extra",
            "https://youtube.com/watch?v=bad",
            "https://youtube.com/watch?v=BaW_jenozKc&v=BaW_jenozKc",
            "https://youtube.com/@channel",
        ] {
            assert!(
                matches!(
                    crate::YoutubeDl::new().unwrap().extract_info(input),
                    Err(Error::UnsupportedYoutube(_))
                ),
                "{input}"
            );
        }
    }

    #[test]
    fn json_assignment_handles_braces_in_strings_and_trailing_javascript() {
        assert_eq!(player()["videoDetails"]["title"], "Fixture } ; & video");
        for marker in [
            "ytInitialPlayerResponse",
            "window[\"ytInitialPlayerResponse\"]",
            "window['ytInitialPlayerResponse']",
        ] {
            assert!(
                player_response(&format!(
                    "<script>{marker} = {{\"x\": 1}}; other();</script>"
                ))
                .is_ok()
            );
        }
        for page in [
            "<p>ytInitialPlayerResponse = {}</p>",
            "<script>ytInitialPlayerResponse = {broken}</script>",
            "<script>ytInitialPlayerResponse = null</script>",
            "<script>_ytInitialPlayerResponse = {}</script>",
            "<script>$ytInitialPlayerResponse = {}</script>",
        ] {
            assert!(matches!(
                player_response(page),
                Err(Error::UnsupportedYoutube(_))
            ));
        }
    }

    #[test]
    fn rejects_access_identity_live_and_drm_responses() {
        for (pointer, value) in [
            ("/playabilityStatus/status", Value::from("LOGIN_REQUIRED")),
            ("/videoDetails/videoId", Value::from("wrong_video")),
            ("/videoDetails/title", Value::from("")),
            ("/videoDetails/isLiveContent", Value::from(true)),
        ] {
            let mut data = player();
            *data.pointer_mut(pointer).unwrap() = value;
            assert!(matches!(
                parse_formats(&data, "BaW_jenozKc"),
                Err(Error::UnsupportedYoutube(_))
            ));
        }
        for key in ["isLive", "isUpcoming", "isPostLiveDvr"] {
            let mut data = player();
            data["videoDetails"][key] = true.into();
            assert!(parse_formats(&data, "BaW_jenozKc").is_err());
        }
        let mut data = player();
        data["streamingData"]["licenseInfos"] = serde_json::json!([{"drmFamily": "WIDEVINE"}]);
        assert!(parse_formats(&data, "BaW_jenozKc").is_err());
    }

    #[test]
    fn supports_muxed_webm_and_ignores_malformed_dimensions_and_duplicate_itags() {
        let mut data = player();
        data["streamingData"]["formats"][0]["mimeType"] = "video/webm; codecs=\"vp9, opus\"".into();
        data["streamingData"]["formats"][0]["width"] = 0.into();
        data["streamingData"]["formats"][0]["height"] = u64::MAX.into();
        let duplicate = data["streamingData"]["formats"][0].clone();
        data["streamingData"]["formats"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        let (_, formats) = parse_formats(&data, "BaW_jenozKc").unwrap();
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].ext, "webm");
        assert_eq!((formats[0].width, formats[0].height), (None, None));
    }

    #[test]
    fn unsupported_formats_are_never_exposed_or_probed() {
        for (key, value) in [
            ("signatureCipher", Value::from("s=secret")),
            ("cipher", Value::from("s=secret")),
            (
                "url",
                Value::from("https://media.example/video?n=challenge"),
            ),
            (
                "url",
                Value::from("https://media.example/n/challenge/video"),
            ),
            ("url", Value::from("blob:https://youtube.com/id")),
            ("type", Value::from("FORMAT_STREAM_TYPE_OTF")),
            ("targetDurationSec", Value::from(5)),
            ("drmFamilies", serde_json::json!(["WIDEVINE"])),
            ("mimeType", Value::from("application/x-mpegURL")),
            (
                "mimeType",
                Value::from("video/mp4; codecs=\"avc1unknown, mp4aunknown\""),
            ),
        ] {
            let mut data = player();
            data["streamingData"]["formats"][0][key] = value;
            let (_, formats) = parse_formats(&data, "BaW_jenozKc").unwrap();
            assert_eq!(formats.len(), 1, "{key}");
            assert_eq!(formats[0].format_id, "22");
            data["streamingData"]["formats"]
                .as_array_mut()
                .unwrap()
                .pop();
            assert!(matches!(
                parse_formats(&data, "BaW_jenozKc"),
                Err(Error::UnsupportedYoutube(_))
            ));
        }
        let mut data = player();
        let formats = data["streamingData"]
            .as_object_mut()
            .unwrap()
            .remove("formats")
            .unwrap();
        data["streamingData"]["adaptiveFormats"] = formats;
        for source in data["streamingData"]["adaptiveFormats"]
            .as_array_mut()
            .unwrap()
        {
            source["type"] = "FORMAT_STREAM_TYPE_OTF".into();
        }
        data["streamingData"]["hlsManifestUrl"] = "https://media.example/live.m3u8".into();
        assert!(parse_formats(&data, "BaW_jenozKc").is_err());
    }

    #[test]
    fn adaptive_streams_preserve_declaration_order_kinds_and_kilobit_rates() {
        let mut data = player();
        data["streamingData"]["adaptiveFormats"] = serde_json::json!([
            {"itag":137,"url":"https://media.example/video.mp4","mimeType":"video/mp4; codecs=\"avc1.640028\"","width":1920,"height":1080,"bitrate":2400000},
            {"itag":140,"url":"https://media.example/audio.m4a","mimeType":"audio/mp4; codecs=\"mp4a.40.2\"","bitrate":128000,"averageBitrate":129500},
            {"itag":248,"url":"https://media.example/video.webm","mimeType":"video/webm; codecs=\"vp09.00.40.08\"","width":1920,"height":1080},
            {"itag":251,"url":"https://media.example/audio.webm","mimeType":"audio/webm; codecs=\"opus\"","bitrate":160000},
            {"itag":18,"url":"https://media.example/duplicate.mp4","mimeType":"video/mp4; codecs=\"avc1.42001E\""}
        ]);
        let (_, formats) = parse_formats(&data, "BaW_jenozKc").unwrap();
        assert_eq!(
            formats
                .iter()
                .map(|f| f.format_id.as_str())
                .collect::<Vec<_>>(),
            ["18", "22", "137", "140", "248", "251"]
        );
        assert_eq!(formats[0].stream_kind, StreamKind::Standalone);
        assert_eq!(formats[2].stream_kind, StreamKind::VideoOnly);
        assert_eq!(formats[2].tbr, Some(2400.0));
        assert_eq!(formats[2].abr, None);
        assert_eq!(formats[3].stream_kind, StreamKind::AudioOnly);
        assert_eq!(formats[3].ext, "m4a");
        assert_eq!(formats[3].abr, Some(129.5));
        assert_eq!(formats[3].tbr, Some(129.5));
        assert_eq!(formats[4].ext, "webm");
        assert_eq!(formats[4].stream_kind, StreamKind::VideoOnly);
        assert_eq!(formats[5].stream_kind, StreamKind::AudioOnly);
        assert_eq!(formats[5].abr, Some(160.0));
    }

    #[test]
    fn absent_or_invalid_average_bitrate_falls_back_to_valid_total_rate() {
        for value in [
            Value::Null,
            serde_json::json!(0),
            serde_json::json!(-1),
            serde_json::json!("invalid"),
        ] {
            let mut data = player();
            data["streamingData"]["formats"][0]["averageBitrate"] = value;
            data["streamingData"]["formats"][0]["bitrate"] = 500000.into();
            let (_, formats) = parse_formats(&data, "BaW_jenozKc").unwrap();
            assert_eq!(formats[0].tbr, Some(500.0));
        }
    }

    #[test]
    fn malformed_bitrates_are_unknown_and_unsupported_codec_sources_are_excluded() {
        for value in [
            Value::Null,
            serde_json::json!("128000"),
            serde_json::json!(-1),
            serde_json::json!(0),
            serde_json::json!(true),
        ] {
            let mut data = player();
            let mut source = data["streamingData"]["formats"][0].clone();
            source["mimeType"] = "audio/mp4; codecs=\"mp4a.40.2\"".into();
            source["bitrate"] = value;
            data["streamingData"]["formats"] = serde_json::json!([source]);
            let (_, formats) = parse_formats(&data, "BaW_jenozKc").unwrap();
            assert_eq!((formats[0].abr, formats[0].tbr), (None, None));
        }
        for mime in [
            "audio/mp4; codecs=\"ac-3\"",
            "video/mp4; codecs=\"hvc1.1.6.L93\"",
            "audio/webm; codecs=\"vp9, opus\"",
            "video/mp4; codecs=\"avc1.640028, unknown\"",
            "video/mp4",
            "audio/mp4; codecs=\"mp4aunknown\"",
        ] {
            let mut data = player();
            data["streamingData"]["formats"][0]["mimeType"] = mime.into();
            let (_, formats) = parse_formats(&data, "BaW_jenozKc").unwrap();
            assert_eq!(formats.len(), 1, "{mime}");
            assert_eq!(formats[0].format_id, "22");
        }
    }

    #[test]
    fn rejects_invalid_watch_page_responses_and_media_probes() {
        let bad_source = FIXTURE.replace("{{ORIGIN}}", "blob:https://youtube.com");
        for (mime, extra, body, probe_html) in [
            ("text/plain", "", b"no page".to_vec(), false),
            (
                "text/html; charset=iso-8859-1",
                "",
                b"ASCII page".to_vec(),
                false,
            ),
            (
                "text/html",
                "Content-Encoding: gzip\r\n",
                b"compressed".to_vec(),
                false,
            ),
            ("text/html", "", vec![0xff], false),
            (
                "text/html",
                "",
                vec![b' '; MAX_PAGE_BYTES as usize + 1],
                false,
            ),
            ("text/html", "", bad_source.into_bytes(), false),
            ("text/html", "", FIXTURE.as_bytes().to_vec(), true),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let page = Url::parse(&format!("{origin}/watch?v=BaW_jenozKc")).unwrap();
            let body = if probe_html {
                FIXTURE.replace("{{ORIGIN}}", &origin).into_bytes()
            } else {
                body
            };
            let worker = thread::spawn(move || {
                for index in 0..if probe_html { 2 } else { 1 } {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        assert_eq!(stream.read(&mut byte).unwrap(), 1);
                        request.push(byte[0]);
                    }
                    let response_body = if index == 0 {
                        body.as_slice()
                    } else {
                        b"<html>not media</html>"
                    };
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", response_body.len()).unwrap();
                    let _ = stream.write_all(response_body);
                }
            });
            let client = Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap();
            assert!(matches!(
                extract_watch_page(&client, &page, "BaW_jenozKc"),
                Err(Error::UnsupportedYoutube(_) | Error::HtmlTooLarge(_))
            ));
            worker.join().unwrap();
        }
    }

    #[test]
    fn local_watch_page_probe_select_and_download() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let page = Url::parse(&format!("{origin}/watch?v=BaW_jenozKc")).unwrap();
        let html = FIXTURE.replace("{{ORIGIN}}", &origin);
        let worker = thread::spawn(move || {
            // Page, two format probes, selected media download.
            for expected in [
                "/watch?v=BaW_jenozKc",
                "/media/low.mp4",
                "/media/high.mp4",
                "/media/high.mp4",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    assert_eq!(stream.read(&mut byte).unwrap(), 1);
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(&format!("GET {expected} HTTP/1.1\r\n")));
                let (mime, body) = if expected.starts_with("/watch") {
                    ("text/html; charset=utf-8", html.as_bytes())
                } else {
                    ("video/mp4", b"media".as_slice())
                };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                let _ = stream.write_all(body);
            }
        });
        let client = Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let mut info = extract_watch_page(&client, &page, "BaW_jenozKc").unwrap();
        assert_eq!(info.formats.len(), 2);
        assert_eq!(info.title, "Fixture } ; & video");
        info.select_format(Some("18")).unwrap();
        assert_eq!(info.height, Some(360));
        info.select_format(Some("worst")).unwrap();
        assert_eq!(info.format_id.as_deref(), Some("18"));
        assert!(info.select_format(Some("137+140")).is_err());
        info.select_format(None).unwrap();
        assert_eq!(info.format_id.as_deref(), Some("22"));
        assert_eq!(info.filesize, Some(5));
        let expected: Value = serde_json::from_str(
            &include_str!("../../tests/fixtures/youtube/expected.json")
                .replace("{{ORIGIN}}", &origin),
        )
        .unwrap();
        let actual = comparable_info(&info);
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(&actual[key], value, "{key}");
        }
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("video.mp4");
        let ydl = crate::YoutubeDl::new().unwrap();
        assert_eq!(ydl.download(&info, &destination).unwrap(), 5);
        assert_eq!(std::fs::read(destination).unwrap(), b"media");
        worker.join().unwrap();
    }
}
