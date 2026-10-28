//! Upstream `FFmpegFD` for partial (`--download-sections`) downloads: FFmpeg
//! reads the selected format URLs itself (`-ss`/`-t` input options, stream
//! copy) and writes the destination's temporary file. FFmpeg's own output is
//! inherited like upstream's `Popen`.
use crate::{
    CookieJar, DownloadOptions, DownloadPhase, DownloadProgress, Error, PartFiles, Result,
    VideoInfo, info::PyNumber,
};
use reqwest::{
    cookie::CookieStore,
    header::{COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use std::{
    ffi::OsString,
    ops::ControlFlow,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

/// Upstream `FFmpegFD.SUPPORTED_PROTOCOLS`.
pub(crate) const SUPPORTED_PROTOCOLS: [&str; 9] = [
    "http",
    "https",
    "ftp",
    "ftps",
    "m3u8",
    "m3u8_native",
    "rtmp",
    "rtmp_ffmpeg",
    "http_dash_segments",
];

/// Upstream `EXT_TO_OUT_FORMATS`.
fn out_format(ext: &str) -> &str {
    match ext {
        "aac" => "adts",
        "flac" => "flac",
        "m4a" => "ipod",
        "mka" | "mkv" => "matroska",
        "mpg" => "mpeg",
        "ogv" => "ogg",
        "ts" => "mpegts",
        "wma" | "wmv" => "asf",
        "weba" => "webm",
        "vtt" => "webvtt",
        other => other,
    }
}

/// Python `str()` of a number.
fn py_str(value: PyNumber) -> String {
    match value {
        PyNumber::Int(value) => value.to_string(),
        PyNumber::Float(value) => crate::screen::py_float(value),
    }
}

fn difference(end: PyNumber, start: PyNumber) -> PyNumber {
    match (end, start) {
        (PyNumber::Int(end), PyNumber::Int(start)) => PyNumber::Int(end - start),
        (end, start) => PyNumber::Float(end.as_f64() - start.as_f64()),
    }
}

/// Request settings FFmpeg needs for network inputs.
pub(crate) struct Network<'a> {
    pub headers: &'a HeaderMap,
    pub cookies: &'a CookieJar,
    pub proxy: Option<&'a str>,
}

/// FFmpeg settings from the downloader parameters.
#[derive(Debug, Clone, Default)]
pub(crate) struct Settings {
    /// `--downloader-args` by key (`ffmpeg`, `ffmpeg_i1`, `ffmpeg_o`, ...).
    pub args: std::collections::HashMap<String, Vec<String>>,
    /// `-loglevel quiet` (upstream `quiet`).
    pub quiet: bool,
    /// `--hls-use-mpegts`: MPEG-TS instead of MP4 for HLS inputs.
    pub hls_use_mpegts: Option<bool>,
    /// `--test`: `-fs _TEST_FILE_SIZE`.
    pub test: bool,
}

/// The `-headers` value for one input (title-cased names): the defaults, a User-Agent,
/// the page Referer of non-direct media and the jar's cookies.
fn header_lines(network: &Network<'_>, info: &VideoInfo, url: &url::Url) -> String {
    let mut headers = network.headers.clone();
    if !headers.contains_key(USER_AGENT) {
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("yt-dlp-rs/", env!("CARGO_PKG_VERSION"))),
        );
    }
    if !info.is_direct()
        && !headers.contains_key(REFERER)
        && let Ok(value) = HeaderValue::from_str(info.webpage_url.as_str())
    {
        headers.insert(REFERER, value);
    }
    if let Some(cookie) = network.cookies.cookies(url) {
        headers.insert(COOKIE, cookie);
    }
    headers
        .iter()
        .map(|(name, value)| {
            format!(
                "{}: {}\r\n",
                py_title(name.as_str()),
                String::from_utf8_lossy(value.as_bytes())
            )
        })
        .collect()
}

/// Python `str.title()` for header names, as `HTTPHeaderDict` stores them.
pub(crate) fn py_title(name: &str) -> String {
    let mut previous_letter = false;
    name.chars()
        .map(|c| {
            let mapped = if previous_letter {
                c.to_ascii_lowercase()
            } else {
                c.to_ascii_uppercase()
            };
            previous_letter = c.is_ascii_alphabetic();
            mapped
        })
        .collect()
}

/// Upstream `cli_configuration_args`: the first key with arguments.
fn configured<'a>(settings: &'a Settings, keys: &[String]) -> &'a [String] {
    keys.iter()
        .find_map(|key| settings.args.get(key))
        .map_or(&[], Vec::as_slice)
}

/// A local file standing in for one input's URL: the segments of a partial
/// download (`indexed_fd`), whose timestamps keep the original timeline.
pub(crate) struct LocalInput {
    pub path: PathBuf,
    /// The original file's start time, which upstream's relative `-ss` is
    /// measured from; the local file starts later.
    pub start_time: f64,
}

/// Each input with its `manifest_stream_number`.
fn inputs(info: &VideoInfo) -> Vec<(VideoInfo, usize)> {
    if info.requested_formats.is_empty() {
        let number = info
            .formats
            .iter()
            .find(|format| info.format_id.as_deref() == Some(format.format_id.as_str()))
            .and_then(|format| format.manifest_stream_number);
        vec![(info.clone(), number.unwrap_or(0))]
    } else {
        info.requested_formats
            .iter()
            .map(|format| {
                (
                    info.with_selected_format(format),
                    format.manifest_stream_number.unwrap_or(0),
                )
            })
            .collect()
    }
}

/// The output format (`-f`) for the last input's protocol.
fn muxer<'a>(settings: &Settings, info: &'a VideoInfo, protocol: &str) -> &'a str {
    if protocol.starts_with("m3u8") {
        // `use_mpegts`: `--hls-use-mpegts`, else whether the stream is live.
        if settings
            .hls_use_mpegts
            .unwrap_or_else(|| crate::is_live(info))
        {
            "mpegts"
        } else {
            "mp4"
        }
    } else if protocol == "rtmp" {
        "flv"
    } else {
        out_format(&info.ext)
    }
}

/// `FFmpegFD._call_downloader` arguments, without the executable; `local`
/// replaces the inputs' URLs, `timecode` adds `-timecode` and `growing`
/// (its timecode) writes a growing QuickTime file (all port extensions).
#[allow(clippy::too_many_arguments)]
fn arguments(
    network: &Network<'_>,
    settings: &Settings,
    info: &VideoInfo,
    local: Option<&[LocalInput]>,
    timecode: Option<&str>,
    growing: Option<&str>,
    temporary: &Path,
    piped: bool,
) -> Result<Vec<OsString>> {
    let mut args: Vec<OsString> = vec!["-y".into()];
    let verbose = crate::screen::verbose();
    if settings.quiet {
        args.extend(["-loglevel".into(), "quiet".into()]);
    } else if verbose {
        args.extend(["-loglevel".into(), "verbose".into()]);
    }
    if !verbose {
        args.push("-hide_banner".into());
    }
    let start = info
        .section_start
        .filter(|start| start.as_f64() != 0.0)
        .unwrap_or(PyNumber::Int(0));
    let end = info.section_end.filter(|end| end.as_f64() != 0.0);
    let formats = inputs(info);
    let mut protocol = String::new();
    for (input, (format, _)) in formats.iter().enumerate() {
        protocol = format.protocol.clone().unwrap_or_default();
        let local = local.and_then(|local| local.get(input));
        // FFmpegSinkFD's input is `-` (FFmpeg's stdin), not an HTTP URL.
        if !piped && local.is_none() && matches!(format.url.scheme(), "http" | "https") {
            let headers = header_lines(network, format, &format.url);
            if !headers.is_empty() {
                args.extend(["-headers".into(), headers.into()]);
            }
        }
        if start.as_f64() != 0.0 {
            match local {
                // The same position as a relative `-ss` on the whole file.
                Some(local) => args.extend([
                    "-seek_timestamp".into(),
                    "1".into(),
                    "-ss".into(),
                    crate::screen::py_float(start.as_f64() + local.start_time).into(),
                ]),
                None => args.extend(["-ss".into(), py_str(start).into()]),
            }
        }
        if let Some(end) = end {
            args.extend(["-t".into(), py_str(difference(end, start)).into()]);
        }
        if protocol == "rtmp" {
            let fields = crate::rtmp_fd::Fields::from_extra(&format.extra);
            args.extend(fields.ffmpeg_args().into_iter().map(OsString::from));
        }
        // `_configuration_args(('_i1', '_i'))` before each input.
        let keys = [format!("ffmpeg_i{}", input + 1), "ffmpeg_i".into()];
        args.extend(configured(settings, &keys).iter().map(OsString::from));
        match local {
            Some(local) => {
                let mut path = OsString::from("file:");
                path.push(&local.path);
                args.extend(["-i".into(), path]);
            }
            None => {
                let url = if piped { "-" } else { format.url.as_str() };
                args.extend(["-i".into(), url.into()]);
            }
        }
    }
    args.extend(["-c".into(), "copy".into()]);
    if !info.requested_formats.is_empty() || protocol == "http_dash_segments" {
        for (index, (_, number)) in formats.iter().enumerate() {
            args.extend(["-map".into(), format!("{index}:{number}").into()]);
        }
    }
    if settings.test {
        args.extend([
            "-fs".into(),
            crate::download::TEST_FILE_SIZE.to_string().into(),
        ]);
    }
    if let Some(timecode) = growing {
        args.extend(crate::growing::output_args(
            timecode,
            crate::growing::ext(info),
        ));
    } else {
        if let Some(timecode) = timecode {
            args.extend(["-timecode".into(), timecode.into()]);
        }
        args.extend(["-f".into(), muxer(settings, info, &protocol).into()]);
    }
    // `_configuration_args(('_o1', '_o', ''))` (with `default`).
    let keys = ["ffmpeg_o1", "ffmpeg_o", "ffmpeg", "default"].map(String::from);
    args.extend(configured(settings, &keys).iter().map(OsString::from));
    // `_ffmpeg_filename_argument`.
    let mut output = OsString::from("file:");
    output.push(temporary);
    args.push(output);
    Ok(args)
}

/// FFmpegSinkFD's connection: writes the stream into FFmpeg's stdin
/// (`real_connection(sink, info_dict)`) until it ends.
pub(crate) type Connection = Box<dyn FnOnce(&mut dyn std::io::Write) + Send>;

/// `ExternalFD.real_download` with `FFmpegFD`: run FFmpeg into the
/// temporary name, then publish it and report one finished status.
#[allow(clippy::too_many_arguments)]
pub(crate) fn download(
    ffmpeg: &Path,
    network: &Network<'_>,
    settings: &Settings,
    info: &VideoInfo,
    local: Option<&[LocalInput]>,
    timecode: Option<&crate::section_timecode::Request>,
    destination: &Path,
    options: &DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    run(
        ffmpeg,
        network,
        settings,
        info,
        local,
        timecode,
        destination,
        options,
        None,
        None,
        callback,
    )
}

/// A growing-file download (`--growing-file`, port extension).
pub(crate) struct Growing {
    /// The `-timecode` of the recording's start.
    pub timecode: String,
    /// The inputs are named pipes the native downloader feeds (`local`):
    /// FFmpeg finishes at their end instead of being asked to quit.
    pub pipes: bool,
}

/// FFmpegFD writing a live stream into a growing QuickTime file at
/// `destination` itself (no temporary name, so an editor importing it keeps
/// a valid path), tagged as recording while FFmpeg runs, with FFmpeg's
/// `-progress` as the reported size and recorded time.
#[allow(clippy::too_many_arguments)]
pub(crate) fn download_growing(
    ffmpeg: &Path,
    network: &Network<'_>,
    settings: &Settings,
    info: &VideoInfo,
    local: Option<&[LocalInput]>,
    growing: &Growing,
    destination: &Path,
    options: &DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    run(
        ffmpeg,
        network,
        settings,
        info,
        local,
        None,
        destination,
        options,
        None,
        Some(growing),
        callback,
    )
}

/// Upstream `FFmpegSinkFD.real_download`: FFmpegFD reading `-` while
/// `connection` feeds its stdin on another thread. The connection's end
/// interrupts the download like upstream's `SIGINT` (a live download is
/// then complete); unlike upstream, FFmpeg is not killed then but reads the
/// closed stdin to its end, so its output is finished.
#[allow(clippy::too_many_arguments)]
pub(crate) fn download_piped(
    ffmpeg: &Path,
    network: &Network<'_>,
    settings: &Settings,
    info: &VideoInfo,
    destination: &Path,
    options: &DownloadOptions,
    connection: Connection,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    run(
        ffmpeg,
        network,
        settings,
        info,
        None,
        None,
        destination,
        options,
        Some(connection),
        None,
        callback,
    )
}

#[allow(clippy::too_many_arguments)]
fn run(
    ffmpeg: &Path,
    network: &Network<'_>,
    settings: &Settings,
    info: &VideoInfo,
    local: Option<&[LocalInput]>,
    timecode: Option<&crate::section_timecode::Request>,
    destination: &Path,
    options: &DownloadOptions,
    connection: Option<Connection>,
    growing: Option<&Growing>,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    crate::download::check_destination(destination, options)?;
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut private = None;
    let temporary: PathBuf = match options.part_files {
        // A growing file is written (and imported) at its final name.
        _ if growing.is_some() => destination.to_owned(),
        PartFiles::NoPart { .. } => destination.to_owned(),
        PartFiles::Part { .. } => {
            let mut name = destination.as_os_str().to_owned();
            name.push(".part");
            name.into()
        }
        PartFiles::Private => {
            let file = crate::download::media_tempfile(".yt-dlp-rs-", ".part", parent)?;
            let path = file.path().to_owned();
            private = Some(file);
            path
        }
    };
    let piped = connection.is_some();
    let timecode = timecode
        .filter(|_| crate::sections::is_partial(info))
        .and_then(|request| {
            let formats = inputs(info);
            let probed: Vec<_> = formats
                .iter()
                .enumerate()
                .map(|(index, (format, _))| crate::section_timecode::Input {
                    info: format,
                    local: local.and_then(|local| local.get(index)),
                    headers: matches!(format.url.scheme(), "http" | "https")
                        .then(|| header_lines(network, format, &format.url)),
                })
                .collect();
            let protocol = formats
                .last()
                .and_then(|(format, _)| format.protocol.clone())
                .unwrap_or_default();
            crate::section_timecode::timecode(
                request,
                info,
                &probed,
                muxer(settings, info, &protocol),
            )
        });
    let args = arguments(
        network,
        settings,
        info,
        local,
        timecode.as_deref(),
        growing.map(|growing| growing.timecode.as_str()),
        &temporary,
        piped,
    )?;
    let progress = |phase, downloaded_bytes| DownloadProgress {
        attempt: 1,
        phase,
        downloaded_bytes,
        total_bytes: None,
        total_bytes_estimate: None,
        stream_index: None,
        stream_count: None,
        fragment_index: None,
        fragment_count: None,
        fragment: None,
        recorded_ms: None,
    };
    if callback(progress(DownloadPhase::Downloading, 0)).is_break() {
        return Err(Error::DownloadCancelled);
    }
    crate::screen::debug_command(ffmpeg, &args);
    let mut command = Command::new(ffmpeg);
    command.args(&args).stdin(Stdio::piped());
    // The growing file exists (and is tagged) before FFmpeg opens it;
    // its truncating open keeps the attribute.
    let mut tag = None;
    if growing.is_some() {
        std::fs::File::create(&temporary)?;
        tag = Some(crate::growing::RecordingTag::set(&temporary));
        command.stdout(Stdio::piped());
    }
    let recorded = Arc::new(crate::growing::Progress::default());
    let mut reported = 0;
    // One status per `-progress` report.
    let report = |reported: &mut u64| {
        let (reports, recorded_ms, bytes) = recorded.snapshot();
        (reports != *reported).then(|| {
            *reported = reports;
            DownloadProgress {
                recorded_ms: Some(recorded_ms),
                ..progress(DownloadPhase::Downloading, bytes)
            }
        })
    };
    if let Some(proxy) = network.proxy.filter(|proxy| !proxy.is_empty()) {
        let proxy = if proxy.contains("://") {
            proxy.to_owned()
        } else {
            format!("http://{proxy}")
        };
        command.env("HTTP_PROXY", &proxy).env("http_proxy", &proxy);
    }
    // Upstream `real_download`: interrupting a live download completes it
    // (FFmpegSinkFD always handles the interrupt its connection raises).
    let interrupt = if growing.is_some() {
        Some(crate::interrupt::Guard::install_with_terminate())
    } else {
        (crate::is_live(info) || piped).then(crate::interrupt::Guard::install)
    };
    let mut child = command.spawn()?;
    let follower = child
        .stdout
        .take()
        .map(|output| Arc::clone(&recorded).follow(output));
    // Upstream keeps FFmpeg's stdin open (a pipe) so it never reads the terminal.
    let mut stdin = child.stdin.take();
    // `on_process_started`: the connection writes FFmpeg's stdin.
    let ended = Arc::new(AtomicBool::new(false));
    if let Some(connection) = connection
        && let Some(mut input) = stdin.take()
    {
        let ended = Arc::clone(&ended);
        std::thread::spawn(move || {
            connection(&mut input);
            drop(input);
            ended.store(true, Ordering::SeqCst);
        });
    }
    let mut interrupted = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        // A library `Interrupt::stop` ends FFmpeg like an interrupt, so the
        // file is finished and kept.
        let stop = crate::control::stopping();
        let signalled = stop
            || interrupt
                .as_ref()
                .is_some_and(crate::interrupt::Guard::interrupted);
        if piped && !interrupted && signalled {
            // A piped FFmpeg is killed (`proc.kill`).
            interrupted = true;
            let _ = child.kill();
        } else if piped && ended.load(Ordering::SeqCst) {
            interrupted = true;
        } else if growing.is_some_and(|growing| growing.pipes) {
            // The pipes' writers stop at the interrupt; FFmpeg reads to their end.
            interrupted |= signalled;
        } else if !piped && !interrupted && signalled {
            // `proc.communicate_or_kill(b'q')`: FFmpeg finishes the file.
            interrupted = true;
            if let Some(mut input) = stdin.take() {
                use std::io::Write;
                let _ = input.write_all(b"q");
            }
        }
        let event = if stop {
            None
        } else if growing.is_some() {
            report(&mut reported)
        } else {
            Some(progress(DownloadPhase::Downloading, 0))
        };
        if event.is_some_and(|event| callback(event).is_break()) {
            let _ = child.kill();
            let _ = child.wait();
            drop(tag);
            // A growing file may already be in use; it stays.
            if growing.is_none() {
                let _ = std::fs::remove_file(&temporary);
            }
            return Err(Error::DownloadCancelled);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // A signal to the whole process group (how apps stop a recording) may
    // end FFmpeg before the loop sees it.
    interrupted |= growing.is_some()
        && interrupt
            .as_ref()
            .is_some_and(crate::interrupt::Guard::interrupted);
    drop(interrupt);
    // The recording has ended either way.
    drop(tag);
    // FFmpeg's final report (`progress=end`).
    if let Some(follower) = follower {
        let _ = follower.join();
        if let Some(event) = report(&mut reported) {
            let _ = callback(event);
        }
    }
    // The connection may end while FFmpeg exits.
    interrupted |= piped && ended.load(Ordering::SeqCst);
    if interrupted && crate::control::stopping() {
        let size = std::fs::metadata(&temporary).map_or(0, |file| file.len());
        if size == 0 {
            if private.is_some() {
                let _ = std::fs::remove_file(&temporary);
            }
            return Err(Error::DownloadCancelled);
        }
        match private {
            Some(file) => crate::download::persist(file, destination, options.overwrite)?,
            None if temporary != destination => std::fs::rename(&temporary, destination)?,
            None => {}
        }
        return Err(Error::DownloadStopped {
            path: destination.to_owned(),
        });
    }
    if interrupted && !crate::is_live(info) {
        // Upstream's `KeyboardInterrupt` propagates for other downloads.
        if private.is_some() {
            let _ = std::fs::remove_file(&temporary);
        }
        return Err(Error::Reported("Interrupted by user".into()));
    }
    if interrupted {
        // `retval = 0`: postprocessing follows as for a finished download.
        if !piped {
            crate::output::encoding::write_message(true, "\n");
        }
        crate::screen::screen("[ffmpeg] Interrupted by user");
    } else if !status.success() {
        if private.is_some() {
            let _ = std::fs::remove_file(&temporary);
        }
        // A growing file keeps what was recorded (unless nothing was).
        if growing.is_some() && std::fs::metadata(&temporary).is_ok_and(|file| file.len() == 0) {
            let _ = std::fs::remove_file(&temporary);
        }
        crate::output::encoding::write_message(true, "\n\n");
        let _ = callback(progress(DownloadPhase::Failed, 0));
        return Err(Error::Reported(format!(
            "ffmpeg exited with code {}",
            status.code().unwrap_or(-1)
        )));
    }
    let size = std::fs::metadata(&temporary)?.len();
    match private {
        Some(file) => {
            crate::download::persist(file, destination, options.overwrite)?;
        }
        None if temporary != destination => std::fs::rename(&temporary, destination)?,
        None => {}
    }
    if callback(progress(DownloadPhase::Completed, size)).is_break() {
        return Err(Error::DownloadCancelled);
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arguments_follow_ffmpeg_fd() {
        let mut info: VideoInfo = serde_json::from_value(json!({"id":"v","title":"V",
            "url":"https://example.test/v.mp4","webpage_url":"https://example.test/page",
            "ext":"mp4","extractor":"html5","extractor_key":"HTML5MediaEmbed","protocol":"https"}))
        .unwrap();
        info.section_start = Some(PyNumber::Int(2));
        info.section_end = Some(PyNumber::Float(4.5));
        let headers = HeaderMap::new();
        let cookies = CookieJar::default();
        let network = Network {
            headers: &headers,
            cookies: &cookies,
            proxy: None,
        };
        let args = arguments(
            &network,
            &Settings::default(),
            &info,
            None,
            None,
            None,
            Path::new("o.mp4.part"),
            false,
        )
        .unwrap();
        let args: Vec<_> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-y",
                "-hide_banner",
                "-headers",
                concat!(
                    "User-Agent: yt-dlp-rs/",
                    env!("CARGO_PKG_VERSION"),
                    "\r\nReferer: https://example.test/page\r\n"
                ),
                "-ss",
                "2",
                "-t",
                "2.5",
                "-i",
                "https://example.test/v.mp4",
                "-c",
                "copy",
                "-f",
                "mp4",
                "file:o.mp4.part"
            ]
        );
        info.ext = "m4a".into();
        info.section_start = None;
        let args = arguments(
            &network,
            &Settings {
                quiet: true,
                ..Default::default()
            },
            &info,
            None,
            None,
            None,
            Path::new("o"),
            false,
        )
        .unwrap();
        let args: Vec<_> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..4], ["-y", "-loglevel", "quiet", "-hide_banner"]);
        assert!(args.windows(2).any(|pair| pair == ["-t", "4.5"]));
        assert!(!args.contains(&"-ss".to_owned()));
        assert!(args.windows(2).any(|pair| pair == ["-f", "ipod"]));
    }

    #[test]
    fn downloader_args_follow_configuration_args_keys() {
        let info: VideoInfo = serde_json::from_value(json!({"id":"v","title":"V",
            "url":"https://example.test/v.mp4","webpage_url":"https://example.test/v.mp4",
            "ext":"mp4","extractor":"generic","extractor_key":"Generic","direct":true,"protocol":"https"}))
        .unwrap();
        let headers = HeaderMap::new();
        let cookies = CookieJar::default();
        let network = Network {
            headers: &headers,
            cookies: &cookies,
            proxy: None,
        };
        let mut settings = Settings::default();
        settings.args.insert("ffmpeg_i".into(), vec!["-re".into()]);
        settings
            .args
            .insert("ffmpeg_i1".into(), vec!["-ss".into(), "1".into()]);
        settings.args.insert("default".into(), vec!["-x".into()]);
        settings.args.insert("ffmpeg".into(), vec!["-y2".into()]);
        let args: Vec<_> = arguments(
            &network,
            &settings,
            &info,
            None,
            None,
            None,
            Path::new("o"),
            false,
        )
        .unwrap()
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
        // `_i1` wins over `_i` before the input; `ffmpeg` over `default` before the output.
        let input = args.iter().position(|arg| arg == "-i").unwrap();
        assert_eq!(&args[input - 2..input], ["-ss", "1"]);
        assert!(!args.contains(&"-re".to_owned()) && !args.contains(&"-x".to_owned()));
        assert_eq!(&args[args.len() - 2..], ["-y2", "file:o"]);
    }

    #[test]
    fn dash_inputs_map_their_manifest_stream_numbers() {
        let format = |id: &str, number: usize, kind: &str| {
            json!({"format_id": id, "url": "https://example.test/manifest.mpd", "ext": "mp4",
                "protocol": "http_dash_segments", "stream_kind": kind, "manifest_stream_number": number})
        };
        let mut info: VideoInfo = serde_json::from_value(json!({"id":"v","title":"V",
            "url":"https://example.test/manifest.mpd","webpage_url":"https://example.test/manifest.mpd",
            "ext":"mp4","extractor":"generic","extractor_key":"Generic","direct":true,
            "protocol":"http_dash_segments","format_id":"a",
            "formats":[format("v", 0, "video_only"), format("a", 1, "audio_only")]}))
        .unwrap();
        info.section_end = Some(PyNumber::Int(4));
        let headers = HeaderMap::new();
        let cookies = CookieJar::default();
        let network = Network {
            headers: &headers,
            cookies: &cookies,
            proxy: None,
        };
        let render = |info: &VideoInfo| -> Vec<String> {
            arguments(
                &network,
                &Settings::default(),
                info,
                None,
                None,
                None,
                Path::new("o"),
                false,
            )
            .unwrap()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
        };
        // Upstream maps a single DASH format's stream too.
        let single = render(&info);
        assert!(
            single.windows(2).any(|pair| pair == ["-map", "0:1"]),
            "{single:?}"
        );
        info.requested_formats = info.formats.clone();
        let pair = render(&info);
        let maps: Vec<_> = pair
            .windows(2)
            .filter(|pair| pair[0] == "-map")
            .map(|pair| pair[1].clone())
            .collect();
        assert_eq!(maps, ["0:0", "1:1"]);
    }
}
