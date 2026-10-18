use super::Extractor;
use crate::{Error, Result, VideoInfo};
use percent_encoding::percent_decode_str;
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT_ENCODING, CONTENT_TYPE, REFERER},
};
use url::Url;

pub struct DirectHttp;

pub(crate) fn request(client: &Client, url: &Url, referer: Option<&Url>) -> Result<Response> {
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
        Ok(from_response(&response, url))
    }
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
        direct: true,
        formats: Vec::new(),
        stream_kind: crate::StreamKind::Standalone,
        requested_formats: Vec::new(),
        format_id: None,
        filesize: response.content_length(),
        width: None,
        height: None,
    })
}
