//! Partial (`--download-sections`) downloads of indexed single-file streams
//! (YouTube's adaptive HTTPS formats). Upstream's `FFmpegFD` lets FFmpeg read
//! the remote URLs, which it does at about twice the playback rate. This
//! port extension instead reads each stream's segment index (`media_index`),
//! fetches the initialization and only the segments covering the section with
//! ranged requests (the selected streams in parallel), then runs the same
//! FFmpeg cut over the local files. Other inputs keep `FFmpegFD`.
use crate::{
    DownloadOptions, DownloadPhase, DownloadProgress, Error, Result, VideoFormat, VideoInfo,
    download::{ByteSpan, fetch_span},
    ffmpeg_fd::{self, LocalInput},
    info::ByteIndex,
    media_index,
};
use reqwest::blocking::Client;
use std::{
    io::Write,
    ops::ControlFlow,
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};

/// YouTube's `CHUNK_SIZE` for its HTTPS formats.
const DEFAULT_CHUNK: u64 = 10 << 20;

pub(crate) struct Params<'a> {
    pub client: &'a Client,
    pub ffmpeg: &'a Path,
    pub network: &'a ffmpeg_fd::Network<'a>,
    pub settings: &'a ffmpeg_fd::Settings,
    /// `--http-chunk-size`, else each format's `http_chunk_size`.
    pub chunk: Option<u64>,
    /// `--section-timecode`.
    pub timecode: Option<&'a crate::section_timecode::Request>,
}

/// One input stream to fetch.
struct Input<'a> {
    info: VideoInfo,
    format: &'a VideoFormat,
    index: ByteIndex,
}

/// The selected streams, when every one is a non-live HTTP(S) single file
/// with a byte index.
fn inputs(info: &VideoInfo) -> Option<Vec<Input<'_>>> {
    if crate::is_live(info) || !crate::sections::is_partial(info) {
        return None;
    }
    let pairs: Vec<(VideoInfo, &VideoFormat)> = if info.requested_formats.is_empty() {
        let format = info
            .formats
            .iter()
            .find(|format| info.format_id.as_deref() == Some(format.format_id.as_str()))?;
        vec![(info.clone(), format)]
    } else {
        info.requested_formats
            .iter()
            .map(|format| (info.with_selected_format(format), format))
            .collect()
    };
    pairs
        .into_iter()
        .map(|(info, format)| {
            let http = matches!(format.determine_protocol().as_str(), "http" | "https");
            (http && format.fragments.is_empty() && format.url == info.url).then_some(Input {
                index: format.byte_index?,
                info,
                format,
            })
        })
        .collect()
}

/// Whether `info` can use this downloader.
pub(crate) fn supported(info: &VideoInfo) -> bool {
    inputs(info).is_some()
}

/// Shared progress of the parallel fetches.
#[derive(Default)]
struct Shared {
    cancelled: AtomicBool,
    /// Bytes written and planned, over all inputs.
    written: AtomicU64,
    planned: AtomicU64,
    /// Inputs whose plan is known.
    planned_inputs: AtomicU64,
}

/// Fetch one input into `file`. `Ok(None)` when its index is unusable.
fn fetch(
    client: &Client,
    input: &Input<'_>,
    chunk: u64,
    options: &DownloadOptions,
    rate_limit: Option<u64>,
    shared: &Shared,
    file: &mut std::fs::File,
) -> Result<Option<f64>> {
    let progress = |shared: &Shared| {
        if shared.cancelled.load(Ordering::Relaxed) {
            Err(Error::DownloadCancelled)
        } else {
            Ok(())
        }
    };
    let ranges = input.index;
    let referrer = (!input.info.is_direct()).then_some(&input.info.webpage_url);
    let head_start = ranges.init.0.min(ranges.index.0);
    let mut head = Vec::new();
    let total = match fetch_span(
        client,
        &ByteSpan {
            url: &input.info.url,
            referrer,
            start: head_start,
            end: ranges.init.1.max(ranges.index.1),
            chunk: u64::MAX,
            rate_limit,
        },
        options,
        &mut head,
        &mut |_| progress(shared),
    ) {
        Ok(total) => total,
        // A server without ranged responses: FFmpeg reads the URL.
        Err(Error::InvalidRangeResponse) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(media) =
        media_index::parse(&head, head_start, ranges, input.format.filesize.or(total))
    else {
        return Ok(None);
    };
    let start_time = media.start_time();
    let section = |value: Option<crate::PyNumber>| {
        value
            .map(|value| value.as_f64())
            .filter(|value| *value != 0.0)
    };
    let (first, last) = media.select(
        section(input.info.section_start).unwrap_or(0.0) + start_time,
        section(input.info.section_end).map(|end| end + start_time),
    );
    let span = (media.segments[first].start, media.segments[last].end);
    crate::screen::debug(&format!(
        "Partial download of format {}: segments {}-{} of {}, bytes {}-{}",
        input.format.format_id,
        first + 1,
        last + 1,
        media.segments.len(),
        span.0,
        span.1
    ));
    let header = media.header(first, last);
    shared
        .planned
        .fetch_add(header.len() as u64 + span.1 + 1 - span.0, Ordering::Relaxed);
    shared.planned_inputs.fetch_add(1, Ordering::Relaxed);
    file.write_all(&header)?;
    shared
        .written
        .fetch_add(header.len() as u64, Ordering::Relaxed);
    let mut output = std::io::BufWriter::with_capacity(1 << 20, file);
    let mut reported = 0;
    fetch_span(
        client,
        &ByteSpan {
            url: &input.info.url,
            referrer,
            start: span.0,
            end: span.1,
            chunk,
            rate_limit,
        },
        options,
        &mut output,
        &mut |bytes| {
            shared
                .written
                .fetch_add(bytes - reported, Ordering::Relaxed);
            reported = bytes;
            progress(shared)
        },
    )?;
    output.flush()?;
    Ok(Some(start_time))
}

/// Download the section through local segment files. `Ok(None)` when an
/// input's index is unusable, for `FFmpegFD` to read the URLs instead.
pub(crate) fn download(
    params: &Params<'_>,
    info: &VideoInfo,
    destination: &Path,
    options: &DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<Option<u64>> {
    let Some(inputs) = inputs(info) else {
        return Ok(None);
    };
    crate::download::check_destination(destination, options)?;
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut files = inputs
        .iter()
        .map(|_| {
            tempfile::Builder::new()
                .prefix(&crate::scratch::prefix(".yt-dlp-rs-", parent))
                .suffix(".part")
                .tempfile_in(parent)
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    let progress = |phase, downloaded_bytes, total_bytes| DownloadProgress {
        attempt: 1,
        phase,
        downloaded_bytes,
        total_bytes,
        total_bytes_estimate: None,
        stream_index: None,
        stream_count: None,
        fragment_index: None,
        fragment_count: None,
        fragment: None,
        recorded_ms: None,
    };
    if callback(progress(DownloadPhase::Requesting, 0, None)).is_break() {
        return Err(Error::DownloadCancelled);
    }
    crate::screen::load_cookies()?;
    let shared = Shared::default();
    // Parallel streams share the rate limit.
    let rate_limit = options
        .rate_limit
        .map(|limit| (limit / inputs.len() as u64).max(1));
    let results: Vec<Result<Option<f64>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = inputs
            .iter()
            .zip(files.iter_mut())
            .map(|(input, file)| {
                let chunk = params
                    .chunk
                    .or_else(|| input.format.downloader_options.as_ref()?.http_chunk_size)
                    .filter(|size| *size > 0)
                    .unwrap_or(DEFAULT_CHUNK);
                let shared = &shared;
                let client = params.client;
                let context = crate::control::current();
                scope.spawn(move || {
                    let _context = crate::control::resume(context);
                    let result = fetch(
                        client,
                        input,
                        chunk,
                        options,
                        rate_limit,
                        shared,
                        file.as_file_mut(),
                    );
                    // A failed or unusable input stops the others.
                    if !matches!(result, Ok(Some(_))) {
                        shared.cancelled.store(true, Ordering::Relaxed);
                    }
                    result
                })
            })
            .collect();
        while !handles.iter().all(|handle| handle.is_finished()) {
            let total = (shared.planned_inputs.load(Ordering::Relaxed) == inputs.len() as u64)
                .then(|| shared.planned.load(Ordering::Relaxed));
            let written = shared.written.load(Ordering::Relaxed);
            crate::screen::flush_traffic();
            if callback(progress(DownloadPhase::Downloading, written, total)).is_break() {
                shared.cancelled.store(true, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or(Err(Error::DownloadCancelled)))
            .collect()
    });
    crate::screen::flush_traffic();
    // The first real error, rather than the cancellations it caused.
    let mut start_times = Vec::with_capacity(results.len());
    let mut cancelled = None;
    let mut unusable = false;
    for result in results {
        match result {
            Ok(Some(start_time)) => start_times.push(start_time),
            Ok(None) => unusable = true,
            Err(Error::DownloadCancelled) => cancelled = Some(Error::DownloadCancelled),
            Err(error) => return Err(error),
        }
    }
    if unusable {
        crate::screen::debug("Segment index unusable; FFmpeg reads the format URLs");
        return Ok(None);
    }
    if let Some(error) = cancelled {
        return Err(error);
    }
    let local: Vec<LocalInput> = files
        .iter()
        .zip(start_times)
        .map(|(file, start_time)| LocalInput {
            path: file.path().to_owned(),
            start_time,
        })
        .collect();
    let size = ffmpeg_fd::download(
        params.ffmpeg,
        params.network,
        params.settings,
        info,
        Some(&local),
        params.timecode,
        destination,
        options,
        callback,
    )?;
    Ok(Some(size))
}
