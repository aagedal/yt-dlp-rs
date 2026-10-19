use super::Extractor;
use crate::{Error, Result, VideoInfo};
use percent_encoding::percent_decode_str;
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT_ENCODING, CONTENT_TYPE, REFERER},
};
use url::Url;

pub struct DirectHttp;

/// An extraction request upstream also makes (`_request_webpage`), subject
/// to `--sleep-requests`.
pub(crate) fn request(client: &Client, url: &Url, referer: Option<&Url>) -> Result<Response> {
    crate::screen::before_extraction_request();
    probe(client, url, referer)
}

/// A request this port adds to inspect media or child playlists that
/// upstream does not fetch during extraction; never delayed.
pub(crate) fn probe(client: &Client, url: &Url, referer: Option<&Url>) -> Result<Response> {
    let mut request = client.get(url.clone()).header(ACCEPT_ENCODING, "identity");
    if let Some(referer) = referer {
        request = request.header(REFERER, referer.as_str());
    }
    let response = crate::retry::check_response(request.send()?)?;
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(Error::PartialResponse);
    }
    Ok(response)
}

pub(crate) fn response_mime(response: &Response) -> String {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

// Manifests need their own downloader; never save an HLS/DASH playlist as a video.
pub(crate) fn media_extension(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => Some("mp4"),
        "webm" => Some("webm"),
        "mkv" => Some("mkv"),
        "mov" => Some("mov"),
        "avi" => Some("avi"),
        "flv" => Some("flv"),
        "mp3" => Some("mp3"),
        "m4a" => Some("m4a"),
        "aac" => Some("aac"),
        "ogg" | "oga" => Some("ogg"),
        "opus" => Some("opus"),
        "wav" => Some("wav"),
        "flac" => Some("flac"),
        _ => None,
    }
}

fn mime_extension(mime: &str) -> Option<&'static str> {
    match mime {
        "video/mp4" => Some("mp4"),
        "video/webm" | "audio/webm" => Some("webm"),
        "video/x-matroska" => Some("mkv"),
        "video/quicktime" => Some("mov"),
        "video/x-msvideo" => Some("avi"),
        "video/x-flv" => Some("flv"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" => Some("m4a"),
        "audio/aac" => Some("aac"),
        "audio/ogg" | "video/ogg" | "application/ogg" => Some("ogg"),
        "audio/opus" => Some("opus"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/flac" | "audio/x-flac" => Some("flac"),
        _ => None,
    }
}

impl Extractor for DirectHttp {
    fn key(&self) -> &'static str {
        "Generic"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        // GET rather than HEAD: some media servers reject HEAD or return different headers.
        // The body remains streamed and is dropped after inspecting the headers.
        let response = request(client, url, None)?;
        let timestamp = last_modified(&response);
        let mime = response_mime(&response);
        Ok(from_response(&response, url).map(|mut info| {
            info.metadata.timestamp = timestamp;
            with_format(info, &mime)
        }))
    }
}

/// Upstream generic direct results carry one format: a matching media
/// Content-Type names it by its subtype (with `vcodec: none` for audio);
/// otherwise `process_video_result` numbers the bare result `0`.
pub(crate) fn with_format(mut info: VideoInfo, mime: &str) -> VideoInfo {
    let (kind, subtype) = mime.split_once('/').unwrap_or((mime, ""));
    let media = matches!(kind, "audio" | "video") || mime == "application/ogg";
    let format_id = if media && !subtype.is_empty() {
        subtype.to_owned()
    } else {
        "0".to_owned()
    };
    if kind == "audio" {
        info.stream_kind = crate::StreamKind::AudioOnly;
    }
    info.format_id = Some(format_id.clone());
    info.formats = vec![crate::VideoFormat {
        protocol: None,
        vcodec: (kind == "audio").then(|| "none".to_owned()),
        acodec: None,
        dynamic_range: None,
        video_ext: None,
        audio_ext: None,
        vbr: None,
        source_preference: None,
        quality: None,
        fps: None,
        asr: None,
        audio_channels: None,
        format_id,
        stream_kind: info.stream_kind,
        audio_group: None,
        language: None,
        format_note: None,
        audio_default: false,
        audio_autoselect: false,
        abr: None,
        tbr: None,
        url: info.url.clone(),
        ext: info.ext.clone(),
        filesize: info.filesize,
        fragments: Vec::new(),
        width: None,
        height: None,
    }];
    info
}

/// Generic `unified_timestamp(response.headers.get('Last-Modified'))`.
pub(crate) fn last_modified(response: &Response) -> Option<i64> {
    let value = response.headers().get(reqwest::header::LAST_MODIFIED)?;
    // Python decodes header bytes as Latin-1.
    let text: String = value.as_bytes().iter().map(|&b| char::from(b)).collect();
    crate::dates::unified_timestamp(&text, true)
}

pub(crate) fn from_response(response: &Response, url: &Url) -> Option<VideoInfo> {
    let final_url = response.url().clone();
    let mime = response_mime(response);
    let filename = final_url
        .path_segments()
        .and_then(|mut p| p.next_back())
        .unwrap_or("");
    let filename = percent_decode_str(filename).decode_utf8_lossy();
    let suffix = filename
        .rsplit_once('.')
        .and_then(|(_, ext)| media_extension(ext));
    // A misleading .mp4 URL serving HTML is not a direct media result.
    let ext = mime_extension(&mime).or_else(|| {
        if mime.is_empty() || mime == "application/octet-stream" {
            suffix
        } else {
            None
        }
    });
    let ext = ext?;
    let stem = filename
        .rsplit_once('.')
        .map_or(filename.as_ref(), |(stem, _)| stem);
    let title = if stem.is_empty() { "video" } else { stem };
    Some(VideoInfo {
        protocol: None,
        format_sort_fields: Vec::new(),
        vcodec: None,
        acodec: None,
        dynamic_range: None,
        video_ext: None,
        audio_ext: None,
        metadata: Default::default(),
        automatic_captions: Default::default(),
        subtitles: Default::default(),
        requested_subtitles: None,
        last_playlist_index: None,
        playlist_index: None,
        playlist_autonumber: None,
        playlist_count: None,
        n_entries: None,
        fragments: Vec::new(),
        id: title.to_owned(),
        title: title.to_owned(),
        ext: ext.to_owned(),
        url: final_url,
        webpage_url: url.clone(),
        extractor: "generic".into(),
        extractor_key: "Generic".into(),
        direct: Some(true),
        formats: Vec::new(),
        stream_kind: crate::StreamKind::Standalone,
        requested_formats: Vec::new(),
        format_id: None,
        // Upstream's generic direct format has no filesize; the transfer's
        // Content-Length is only known to the downloader.
        filesize: None,
        width: None,
        height: None,
    })
}
