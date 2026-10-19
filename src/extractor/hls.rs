//! Bounded VOD HLS extraction. Segment bodies are fetched only by the downloader.
use super::direct;
use crate::{Error, Result, StreamKind, VideoFormat, VideoInfo, hls::Playlist};
use percent_encoding::percent_decode_str;
use reqwest::{
    blocking::{Client, Response},
    header::{CONTENT_ENCODING, CONTENT_TYPE},
};
use std::io::Read;
use url::Url;

const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_VARIANTS: usize = 256;
const MAX_TOTAL_FRAGMENTS: usize = 10_000;

fn unsupported(message: &str) -> Error {
    Error::UnsupportedManifest(message.into())
}

fn recognized(response: &Response) -> bool {
    let mime = direct::response_mime(response);
    matches!(
        mime.as_str(),
        "application/vnd.apple.mpegurl"
            | "application/x-mpegurl"
            | "audio/mpegurl"
            | "audio/x-mpegurl"
    ) || response
        .url()
        .path()
        .to_ascii_lowercase()
        .ends_with(".m3u8")
}

fn read_playlist(response: Response) -> Result<(Url, Playlist)> {
    if response
        .content_length()
        .is_some_and(|size| size > MAX_MANIFEST_BYTES)
    {
        return Err(unsupported(
            "HLS playlist exceeds the 2097152-byte extraction limit",
        ));
    }
    let encodings = response.headers().get_all(CONTENT_ENCODING);
    if encodings.iter().count() > 1 {
        return Err(unsupported(
            "duplicate HLS Content-Encoding headers are not supported",
        ));
    }
    if encodings
        .iter()
        .any(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(unsupported("compressed HLS playlists are not supported"));
    }
    let content_types = response.headers().get_all(CONTENT_TYPE);
    if content_types.iter().count() > 1 {
        return Err(unsupported(
            "duplicate HLS Content-Type headers are not supported",
        ));
    }
    if let Some(value) = content_types.iter().next() {
        let content_type = value
            .to_str()
            .map_err(|_| unsupported("invalid HLS Content-Type header"))?;
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
                return Err(unsupported("only UTF-8/ASCII HLS playlists are supported"));
            }
        }
    }
    let base = response.url().clone();
    let mut bytes = Vec::new();
    response
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(unsupported(
            "HLS playlist exceeds the 2097152-byte extraction limit",
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| unsupported("only UTF-8/ASCII HLS playlists are supported"))?;
    Ok((base.clone(), crate::hls::parse(text, &base)?))
}

pub(crate) fn from_response(
    client: &Client,
    response: Response,
    original: &Url,
    referer: Option<&Url>,
    force_hls: bool,
    m3u8_id: Option<&str>,
) -> Result<Option<VideoInfo>> {
    if !force_hls && !recognized(&response) {
        return Ok(None);
    }
    let (manifest_url, playlist) = read_playlist(response)?;
    let filename = manifest_url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or("");
    let filename = percent_decode_str(filename).decode_utf8_lossy();
    let stem = filename
        .rsplit_once('.')
        .map_or(filename.as_ref(), |(stem, _)| stem);
    let title = if stem.is_empty() { "video" } else { stem }.to_owned();
    let mut info = VideoInfo {
        metadata: Default::default(),
        automatic_captions: Default::default(),
        subtitles: Default::default(),
        requested_subtitles: None,
        last_playlist_index: None,
        playlist_index: None,
        playlist_autonumber: None,
        playlist_count: None,
        n_entries: None,
        id: title.clone(),
        title,
        ext: String::new(),
        url: manifest_url.clone(),
        webpage_url: original.clone(),
        extractor: "generic".into(),
        extractor_key: "Generic".into(),
        direct: false,
        formats: Vec::new(),
        fragments: Vec::new(),
        requested_formats: Vec::new(),
        stream_kind: StreamKind::Standalone,
        format_id: None,
        filesize: None,
        width: None,
        height: None,
    };
    // Upstream `join_nonempty(...)` format IDs; empty IDs are numbered later.
    let join = |parts: &[Option<&str>]| -> String {
        parts
            .iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("-")
    };
    match playlist {
        Playlist::Media { fragments, ext } => {
            // A media playlist is returned as one format.
            info.formats.push(VideoFormat {
                format_id: join(&[m3u8_id]),
                stream_kind: StreamKind::Standalone,
                audio_group: None,
                language: None,
                format_note: None,
                audio_default: false,
                audio_autoselect: false,
                abr: None,
                tbr: None,
                url: info.url.clone(),
                ext: ext.clone(),
                filesize: None,
                width: None,
                height: None,
                fragments: fragments.clone(),
            });
            info.ext = ext;
            info.fragments = fragments;
        }
        Playlist::Master { variants, audio } => {
            if variants.len() + audio.len() > MAX_VARIANTS {
                return Err(unsupported(
                    "HLS master exceeds the 256-variant extraction limit",
                ));
            }
            let mut total_fragments = 0usize;
            // Upstream lists renditions before variants (and counts them for
            // unnamed variants); playlists are still fetched variants first.
            let renditions = audio.len();
            for variant in variants {
                let response = direct::request(client, &variant.url, referer)?;
                let (url, playlist) = read_playlist(response)?;
                let Playlist::Media { fragments, ext } = playlist else {
                    return Err(unsupported("nested HLS master playlists are not supported"));
                };
                total_fragments = total_fragments
                    .checked_add(fragments.len())
                    .ok_or_else(|| unsupported("HLS master fragment count overflow"))?;
                if total_fragments > MAX_TOTAL_FRAGMENTS {
                    return Err(unsupported(
                        "HLS master exceeds the 10000-fragment extraction limit",
                    ));
                }
                // Upstream `'%d' % (tbr or len(formats))` without a stream name.
                let fallback = match variant.bandwidth.filter(|tbr| *tbr != 0.0) {
                    Some(tbr) => format!("{}", tbr.trunc() as i64),
                    None => (renditions + info.formats.len()).to_string(),
                };
                let name = variant.name.clone().unwrap_or(fallback);
                info.formats.push(VideoFormat {
                    format_id: join(&[m3u8_id, Some(&name)]),
                    stream_kind: variant.stream_kind,
                    audio_group: variant.audio_group,
                    language: None,
                    format_note: None,
                    audio_default: false,
                    audio_autoselect: false,
                    abr: (variant.stream_kind == StreamKind::AudioOnly)
                        .then_some(variant.bandwidth)
                        .flatten(),
                    tbr: variant.bandwidth,
                    url,
                    ext: if variant.stream_kind == StreamKind::AudioOnly && ext == "mp4" {
                        "m4a".into()
                    } else {
                        ext
                    },
                    filesize: None,
                    width: variant.width,
                    height: variant.height,
                    fragments,
                });
            }
            let mut rendition_formats = Vec::new();
            for rendition in audio {
                let response = direct::request(client, &rendition.url, referer)?;
                let (url, playlist) = read_playlist(response)?;
                let Playlist::Media { fragments, ext } = playlist else {
                    return Err(unsupported("nested HLS master playlists are not supported"));
                };
                total_fragments = total_fragments
                    .checked_add(fragments.len())
                    .ok_or_else(|| unsupported("HLS master fragment count overflow"))?;
                if total_fragments > MAX_TOTAL_FRAGMENTS {
                    return Err(unsupported(
                        "HLS master exceeds the 10000-fragment extraction limit",
                    ));
                }
                rendition_formats.push(VideoFormat {
                    format_id: join(&[m3u8_id, Some(&rendition.group_id), Some(&rendition.name)]),
                    stream_kind: StreamKind::AudioOnly,
                    audio_group: Some(rendition.group_id),
                    language: rendition.language,
                    format_note: Some(rendition.name),
                    audio_default: rendition.default,
                    audio_autoselect: rendition.autoselect,
                    abr: None,
                    tbr: None,
                    url,
                    ext: if ext == "mp4" { "m4a".into() } else { ext },
                    filesize: None,
                    width: None,
                    height: None,
                    fragments,
                });
            }
            info.formats.splice(0..0, rendition_formats);
            // Renditions precede variants; the top level mirrors a variant.
            let first = info
                .formats
                .get(renditions)
                .or(info.formats.first())
                .ok_or_else(|| unsupported("HLS master has no variants"))?;
            info.ext = first.ext.clone();
            info.url = first.url.clone();
            info.stream_kind = first.stream_kind;
            info.fragments = first.fragments.clone();
        }
    }
    Ok(Some(info))
}
