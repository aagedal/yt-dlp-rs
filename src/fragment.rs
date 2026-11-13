//! Sequential assembly of saved, finite fragment plans: all-or-error by
//! default, or skipping unavailable fragments like upstream when enabled.
use crate::{DownloadOptions, DownloadPhase, DownloadProgress, Error, Fragment, Result, VideoInfo};
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, CONTENT_TYPE, RANGE, REFERER},
};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    ops::ControlFlow,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

struct Reporter<'a> {
    callback: &'a mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
    attempt: u16,
    index: u32,
    count: u32,
    /// `--limit-rate`, applied per fragment request like upstream.
    rate_limit: Option<u64>,
    /// The current fragment's HttpFD progress, reported with its data.
    fragment: Option<crate::FragmentBytes>,
    /// A live DASH snapshot (`ctx['live']`).
    live: bool,
}
impl Reporter<'_> {
    fn report(&mut self, phase: DownloadPhase, bytes: u64) -> Result<()> {
        let fragment = self
            .fragment
            .take()
            .filter(|_| phase == DownloadPhase::Downloading)
            .map(|fragment| crate::FragmentBytes {
                live: self.live,
                ..fragment
            });
        if (self.callback)(DownloadProgress {
            attempt: self.attempt,
            downloaded_bytes: bytes,
            total_bytes: None,
            total_bytes_estimate: None,
            phase,
            stream_index: None,
            stream_count: None,
            fragment_index: Some(self.index),
            // Live fragment lists have no count (`0`).
            fragment_count: (self.count > 0).then_some(self.count),
            fragment,
            recorded_ms: None,
        })
        .is_break()
        {
            Err(Error::DownloadCancelled)
        } else {
            Ok(())
        }
    }
    fn wait(&mut self, delay: Duration, bytes: u64) -> Result<()> {
        let start = Instant::now();
        loop {
            self.report(DownloadPhase::WaitingToRetry, bytes)?;
            let remaining = delay.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(remaining.min(Duration::from_millis(100)));
        }
    }
}
struct Failure {
    error: Error,
    retry: Option<Duration>,
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self { error, retry: None }
    }
}
impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error).into()
    }
}
fn body_failure(error: Error) -> Failure {
    Failure {
        // An interrupt ends the download instead.
        retry: (!matches!(error, Error::DownloadCancelled)).then_some(Duration::ZERO),
        error,
    }
}
/// FragmentFD's uncaught `OSError` for its state or part file, which
/// `process_info` reports.
fn unable_to_download(error: &std::io::Error, path: &Path) -> Error {
    Error::Reported(format!(
        "Unable to download video: {}",
        crate::download::os_error(error, path)
    ))
}

/// HttpFD's error for a response without a body: reported, never retried.
const NO_DATA_BLOCKS: &str = "Did not get any data blocks";
fn unsupported(message: &str) -> Error {
    Error::UnsupportedManifest(message.into())
}

pub(crate) fn download(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    concurrency: usize,
    keep_fragments: bool,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    download_with(
        client,
        info,
        destination,
        options,
        concurrency,
        keep_fragments,
        Assembly::Raw,
        callback,
    )
}

/// How fragments are written.
#[derive(Clone, Copy)]
enum Assembly<'a> {
    /// Appended as received (HlsFD, DashSegmentsFD).
    Raw,
    /// `F4mFD`: the FLV header, then each fragment's `mdat` payload.
    F4m(&'a [u8]),
    /// `IsmFD`: the PIFF header (from these `_download_params` and the first
    /// fragment's track ID) before the raw fragments.
    Ism(&'a serde_json::Value),
    /// `MhtmlFD`: the MIME headers and HTML stub, each fragment as an image
    /// part, then the closing boundary (`str` is a fresh download's boundary).
    Mhtml(&'a str),
}

/// Upstream `MhtmlFD`, sequentially: fragment requests use HttpFD's
/// `--retries` (only transient failures) and any other failure ends the
/// download, as upstream's `_download_fragment` raises it.
pub(crate) fn download_mhtml(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    mut options: DownloadOptions,
    keep_fragments: bool,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    options.fragment_retries = options.retries;
    options.skip_unavailable_fragments = None;
    let boundary = crate::mhtml::boundary();
    download_with(
        client,
        info,
        destination,
        options,
        1,
        keep_fragments,
        Assembly::Mhtml(&boundary),
        callback,
    )
}

/// Upstream `IsmFD`'s assembly, sequentially.
pub(crate) fn download_ism(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    keep_fragments: bool,
    params: &serde_json::Value,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    download_with(
        client,
        info,
        destination,
        options,
        1,
        keep_fragments,
        Assembly::Ism(params),
        callback,
    )
}

/// Upstream `F4mFD`'s assembly: the FLV `header` first (unless resuming),
/// then each fragment's `mdat` payload, sequentially.
pub(crate) fn download_f4m(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    keep_fragments: bool,
    header: &[u8],
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    download_with(
        client,
        info,
        destination,
        options,
        1,
        keep_fragments,
        Assembly::F4m(header),
        callback,
    )
}

#[allow(clippy::too_many_arguments)]
fn download_with(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    concurrency: usize,
    keep_fragments: bool,
    assembly: Assembly<'_>,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    crate::download::check_destination(destination, &options)?;
    // The extractor-argument queries and key apply to the requests.
    let adjusted = crate::manifest_args::download_fragments(info).map(|fragments| {
        let mut adjusted = info.clone();
        adjusted.fragments = fragments;
        adjusted
    });
    let info = adjusted.as_ref().unwrap_or(info);
    let external_key = crate::manifest_args::external_key(info)?;
    let initial_keys = || -> HashMap<url::Url, [u8; 16]> {
        external_key
            .map(|key| {
                info.fragments
                    .iter()
                    .filter_map(|fragment| fragment.encryption.as_ref())
                    .map(|encryption| (encryption.key_url.clone(), key))
                    .collect()
            })
            .unwrap_or_default()
    };
    if info.fragments.is_empty() || info.fragments.len() > 10_000 {
        return Err(unsupported("fragment count must be between 1 and 10000"));
    }
    for fragment in &info.fragments {
        if !matches!(fragment.url.scheme(), "http" | "https") {
            return Err(Error::UnsupportedScheme(fragment.url.scheme().into()));
        }
        if let Some(encryption) = &fragment.encryption
            && !matches!(encryption.key_url.scheme(), "http" | "https")
        {
            return Err(Error::UnsupportedScheme(encryption.key_url.scheme().into()));
        }
        if let Some(range) = &fragment.byte_range
            && (range.length == 0 || range.offset.checked_add(range.length).is_none())
        {
            return Err(Error::InvalidRangeResponse);
        }
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    crate::download::create_parent(parent)?;
    // Upstream `_prepare_frag_download`: `FILE.part` (or the destination
    // with --no-part) and `FILE.ytdl` resume state; the library default
    // assembles a private temporary file instead.
    let (upstream, resume) = match options.part_files {
        crate::PartFiles::Private => (false, false),
        crate::PartFiles::Part { resume } | crate::PartFiles::NoPart { resume } => (true, resume),
    };
    // A live DASH snapshot (`ctx['live']`) neither resumes nor keeps state.
    let live = crate::live_snapshot(info);
    let resume = resume && !live;
    let part_path = {
        let mut name = destination.as_os_str().to_owned();
        if matches!(options.part_files, crate::PartFiles::Part { .. }) {
            name.push(".part");
        }
        std::path::PathBuf::from(name)
    };
    let ytdl_path = {
        let mut name = destination.as_os_str().to_owned();
        name.push(".ytdl");
        std::path::PathBuf::from(name)
    };
    // HlsFD keeps an (empty) `extra_state`, IsmFD whether its header is
    // written; DashSegmentsFD and F4mFD have none.
    let hls = info
        .protocol
        .as_deref()
        .is_some_and(|protocol| protocol.starts_with("m3u8"));
    let ism = matches!(assembly, Assembly::Ism(_));
    // MhtmlFD's boundary, and whether its header is written (its
    // `extra_state`, saved once the header is).
    let mut mhtml = match assembly {
        Assembly::Mhtml(boundary) => Some((boundary.to_owned(), false)),
        _ => None,
    };
    let write_state = |index: usize, mhtml: &Option<(String, bool)>| -> Result<()> {
        if live {
            return Ok(());
        }
        let extra = if let Some((boundary, true)) = mhtml {
            format!(
                ", \"extra_state\": {{\"header_written\": true, \"mime_boundary\": {}}}",
                serde_json::Value::from(boundary.as_str())
            )
        } else if ism {
            format!(
                ", \"extra_state\": {{\"ism_track_written\": {}}}",
                if index > 0 { "true" } else { "false" }
            )
        } else if hls {
            ", \"extra_state\": {}".to_owned()
        } else {
            String::new()
        };
        fs::write(
            &ytdl_path,
            format!("{{\"downloader\": {{\"current_fragment\": {{\"index\": {index}}}{extra}}}}}"),
        )
        .map_err(|error| unable_to_download(&error, &ytdl_path))?;
        Ok(())
    };
    let mut start = 0_usize;
    let mut private = None;
    let mut named = None;
    if upstream {
        let mut resume_len = fs::metadata(&part_path).map_or(0, |metadata| metadata.len());
        if resume && ytdl_path.is_file() {
            let saved: Option<serde_json::Value> = fs::read_to_string(&ytdl_path)
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
            let state: Option<usize> = saved
                .as_ref()
                .and_then(|value| value["downloader"]["current_fragment"]["index"].as_u64())
                .map(|index| index as usize);
            let message = match state {
                None => Some(".ytdl file is corrupt"),
                Some(index) if index > 0 && resume_len == 0 => {
                    Some("Inconsistent state of incomplete fragment download")
                }
                Some(index) => {
                    start = index.min(info.fragments.len());
                    None
                }
            };
            if let Some(message) = message {
                crate::screen::warning(&format!("{message}. Restarting from the beginning ..."));
                start = 0;
                resume_len = 0;
                write_state(0, &None)?;
            } else if let Some(state) = &mut mhtml {
                // The saved boundary keeps the parts consistent.
                let extra = saved
                    .as_ref()
                    .map(|value| &value["downloader"]["extra_state"]);
                if let Some(boundary) = extra.and_then(|extra| extra["mime_boundary"].as_str()) {
                    *state = (
                        boundary.to_owned(),
                        extra.is_some_and(|extra| extra["header_written"] == true),
                    );
                }
            }
        } else {
            if !resume {
                resume_len = 0;
            }
            write_state(0, &None)?;
        }
        // Not `append`: an append-only Windows handle cannot `sync_all`
        // ("Access is denied"); resumed writes start at the end instead.
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .truncate(resume_len == 0)
            .open(&part_path)
            .map_err(|error| unable_to_download(&error, &part_path))?;
        file.seek(SeekFrom::Start(resume_len))?;
        named = Some((file, resume_len));
    } else {
        private = Some(crate::download::media_tempfile(
            ".yt-dlp-rs-fragments-",
            ".part",
            parent,
        )?);
    }
    let resumed_bytes = named.as_ref().map_or(0, |(_, length)| *length);
    let mut stored = resumed_bytes;
    let mut keys = initial_keys();
    // Upstream keeps the latest nonzero fragment filetime.
    let mut filetime = None;
    let mut reporter = Reporter {
        callback,
        attempt: 1,
        index: start as u32 + 1,
        count: info.fragments.len() as u32,
        rate_limit: options.rate_limit,
        fragment: None,
        live,
    };
    // Upstream `--keep-fragments`: `TMPFILENAME-FragN` beside the output.
    let keep = keep_fragments.then(|| part_path.as_os_str().to_owned());
    let keep_fragment = |index: usize, bytes: &mut dyn Read| -> Result<()> {
        if let Some(base) = &keep {
            let mut name = base.clone();
            name.push(format!("-Frag{}", index + 1));
            let mut file = fs::File::create(std::path::PathBuf::from(name))?;
            std::io::copy(bytes, &mut file)?;
        }
        Ok(())
    };
    // Completed fragments for `FILE.ytdl` (upstream counts downloads, not
    // positions).
    let mut completed = start;
    // MhtmlFD writes its header first unless the saved state says it did.
    let mhtml_header = mhtml.as_ref().is_some_and(|(_, written)| !written);
    if let Some((_, written)) = &mut mhtml {
        *written = true;
    }
    let output_path = private.as_ref().map_or_else(
        || part_path.clone(),
        |temporary| temporary.path().to_path_buf(),
    );
    // The output's length after the last whole fragment (a resumed part
    // file's to begin with), which a library `Interrupt::stop` publishes.
    let committed = std::cell::Cell::new((resumed_bytes > 0).then_some(resumed_bytes));
    let mut appended = |_: usize| -> Result<()> {
        completed += 1;
        committed.set(
            fs::metadata(&output_path)
                .ok()
                .map(|metadata| metadata.len()),
        );
        if upstream {
            write_state(completed, &mhtml)
        } else {
            Ok(())
        }
    };
    let assembled = (|| -> Result<()> {
        let output: &mut fs::File = match (&mut private, &mut named) {
            (Some(temporary), _) => temporary.as_file_mut(),
            (_, Some((file, _))) => file,
            _ => unreachable!("one output file is open"),
        };
        // `write_flv_header` (and the metadata tag) before any fragment.
        if let Assembly::F4m(header) = assembly
            && stored == 0
        {
            output.write_all(header)?;
            stored = header.len() as u64;
        }
        // MhtmlFD's unflushed header reaches the file with the first
        // appended fragment (a failure before that leaves it empty).
        let mut mhtml_pending = mhtml
            .as_ref()
            .filter(|_| mhtml_header)
            .map(|(boundary, _)| {
                let durations: Vec<Option<f64>> = info
                    .fragments
                    .iter()
                    .map(|fragment| fragment.duration)
                    .collect();
                let stub = crate::mhtml::stub(&durations, boundary, &info.title);
                crate::mhtml::header(&stub, boundary, &info.title, info.webpage_url.as_str())
            });
        // IsmFD's header follows the first fragment's download.
        let mut track_written = start > 0;
        if concurrency > 1 && info.fragments.len() - start > 1 {
            let (written, latest) = download_concurrent(
                client,
                info,
                &options,
                concurrency,
                start,
                parent,
                output,
                &mut reporter,
                &keep_fragment,
                &mut appended,
            )?;
            stored += written;
            filetime = latest;
        } else {
            let budget = SpoolMemory::default();
            for (index, fragment) in info.fragments.iter().enumerate().skip(start) {
                reporter.index = index as u32 + 1;
                let prefix = stored;
                let mut staged = Staged {
                    output: &mut *output,
                    base: prefix,
                    spool: Spool::new(&budget, parent),
                };
                let fetched = fetch_with_retries(
                    client,
                    info,
                    index,
                    fragment,
                    &options,
                    &mut keys,
                    &mut staged,
                    prefix,
                    &mut stored,
                    &mut reporter,
                )?;
                if let Some(modified) = fetched {
                    staged.append()?;
                    // HttpFD's `finished` status for the fragment.
                    reporter.fragment = Some(crate::FragmentBytes {
                        index: index as u32,
                        downloaded: stored - prefix,
                        total: Some(stored - prefix),
                        finished: true,
                        live: false,
                    });
                    reporter.report(DownloadPhase::Downloading, stored)?;
                    filetime = crate::filetime::filetime(modified.as_deref()).or(filetime);
                    if keep.is_some() {
                        let mut file = fs::File::open(&output_path)?;
                        file.seek(SeekFrom::Start(prefix))?;
                        keep_fragment(index, &mut file.take(stored - prefix))?;
                    }
                    if let Assembly::Ism(params) = assembly
                        && !track_written
                    {
                        let mut raw = Vec::new();
                        let mut file = fs::File::open(&output_path)?;
                        file.seek(SeekFrom::Start(prefix))?;
                        file.take(stored - prefix).read_to_end(&mut raw)?;
                        let id = crate::ism_fd::track_id(&raw)?;
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |elapsed| elapsed.as_secs());
                        let header = crate::ism_fd::piff_header(params, id, now)?;
                        output.set_len(prefix)?;
                        output.seek(SeekFrom::Start(prefix))?;
                        output.write_all(&header)?;
                        output.write_all(&raw)?;
                        stored = prefix + (header.len() + raw.len()) as u64;
                        track_written = true;
                    }
                    // F4mFD appends only the fragment's `mdat` box data.
                    if matches!(assembly, Assembly::F4m(_)) {
                        let mut raw = Vec::new();
                        let mut file = fs::File::open(&output_path)?;
                        file.seek(SeekFrom::Start(prefix))?;
                        file.take(stored - prefix).read_to_end(&mut raw)?;
                        let payload = crate::f4m_fd::mdat(&raw)?;
                        output.set_len(prefix)?;
                        output.seek(SeekFrom::Start(prefix))?;
                        output.write_all(payload)?;
                        stored = prefix + payload.len() as u64;
                    }
                    // MhtmlFD wraps each fragment in an image part.
                    if let Some((boundary, _)) = &mhtml {
                        let mut raw = Vec::new();
                        let mut file = fs::File::open(&output_path)?;
                        file.seek(SeekFrom::Start(prefix))?;
                        file.take(stored - prefix).read_to_end(&mut raw)?;
                        let duration = fragment
                            .duration
                            .ok_or_else(|| Error::Reported("KeyError: 'duration'".into()))?;
                        let mut part = mhtml_pending.take().unwrap_or_default();
                        part.extend(crate::mhtml::part(
                            index,
                            boundary,
                            fragment.url.as_str(),
                            duration,
                            &raw,
                        ));
                        output.set_len(prefix)?;
                        output.seek(SeekFrom::Start(prefix))?;
                        output.write_all(&part)?;
                        stored = prefix + part.len() as u64;
                    }
                    appended(index)?;
                }
            }
            if let Some((boundary, _)) = &mhtml {
                let mut trailer = mhtml_pending.take().unwrap_or_default();
                trailer.extend(crate::mhtml::trailer(boundary));
                output.write_all(&trailer)?;
                stored += trailer.len() as u64;
            }
        }
        output.flush()?;
        output.sync_all()?;
        if options.update_time {
            crate::filetime::apply(output, filetime);
        }
        Ok(())
    })();
    if let Err(error) = assembled {
        let length = committed.get().unwrap_or(0);
        let error = crate::download::stop_publish(error, length, destination, || {
            let output: &mut fs::File = match (&mut private, &mut named) {
                (Some(temporary), _) => temporary.as_file_mut(),
                (_, Some((file, _))) => file,
                _ => unreachable!("one output file is open"),
            };
            output.set_len(length)?;
            output.sync_all()?;
            match private.take() {
                Some(temporary) => {
                    crate::download::persist(temporary, destination, options.overwrite)
                }
                None => {
                    drop(named.take());
                    let _ = fs::remove_file(&ytdl_path);
                    if part_path != destination {
                        if !options.overwrite && fs::symlink_metadata(destination).is_ok() {
                            return Err(Error::OutputExists(destination.into()));
                        }
                        fs::rename(&part_path, destination)?;
                    }
                    Ok(())
                }
            }
        });
        // A stop (`--keep-partial-on-stop`) before any data arrived leaves
        // nothing, not an empty part file and its state.
        if matches!(error, Error::DownloadCancelled)
            && crate::control::stopping()
            && length == 0
            && named.take().is_some()
        {
            let _ = fs::remove_file(&ytdl_path);
            if fs::metadata(&part_path).is_ok_and(|metadata| metadata.len() == 0) {
                let _ = fs::remove_file(&part_path);
            }
        }
        return Err(error);
    }
    // `_finish_frag_download`: every fragment skipped leaves nothing.
    if stored == 0 {
        if named.take().is_some() {
            let _ = fs::remove_file(&ytdl_path);
            let _ = fs::remove_file(&part_path);
        }
        return Err(Error::Reported("The downloaded file is empty".into()));
    }
    reporter.report(DownloadPhase::Publishing, stored)?;
    match private {
        Some(temporary) => crate::download::persist(temporary, destination, options.overwrite)?,
        None => {
            // The part file must be closed before it is renamed (Windows).
            drop(named);
            // `_finish_frag_download`: drop the state, then rename the part file.
            let _ = fs::remove_file(&ytdl_path);
            if part_path != destination {
                fs::rename(&part_path, destination)?;
            }
        }
    }
    Ok(stored)
}
/// One generated live fragment (fragment `index`, zero-based) appended to
/// `file` after `*stored` with `--fragment-retries`; `Ok(false)` when it was
/// skipped as unavailable (never the first).
#[allow(clippy::too_many_arguments)]
pub(crate) fn append_live(
    client: &Client,
    info: &VideoInfo,
    index: usize,
    fragment: &Fragment,
    options: &DownloadOptions,
    file: &mut fs::File,
    stored: &mut u64,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<bool> {
    let mut reporter = Reporter {
        callback,
        attempt: 1,
        index: index as u32 + 1,
        count: 0,
        rate_limit: options.rate_limit,
        fragment: None,
        live: false,
    };
    let prefix = *stored;
    let appended = fetch_with_retries(
        client,
        info,
        index,
        fragment,
        options,
        &mut HashMap::new(),
        file,
        prefix,
        stored,
        &mut reporter,
    )?;
    Ok(appended.is_some())
}

/// One fragment with `--fragment-retries` (`Ok(None)` when skipped as
/// unavailable): written to `file` after `prefix`, which a failed attempt
/// truncates back to.
#[allow(clippy::too_many_arguments)]
fn fetch_with_retries(
    client: &Client,
    info: &VideoInfo,
    index: usize,
    fragment: &Fragment,
    options: &DownloadOptions,
    keys: &mut HashMap<url::Url, [u8; 16]>,
    file: &mut dyn Sink,
    prefix: u64,
    stored: &mut u64,
    reporter: &mut Reporter<'_>,
) -> Result<Option<Option<String>>> {
    for attempt in 0..=options.fragment_retries {
        file.truncate(prefix)?;
        *stored = prefix;
        reporter.attempt = u16::try_from(attempt.saturating_add(1)).unwrap_or(u16::MAX);
        reporter.report(DownloadPhase::Requesting, *stored)?;
        let limits = (options.min_filesize, options.max_filesize);
        let failure = match fetch(client, info, fragment, keys, file, stored, reporter, limits) {
            Ok(modified) => return Ok(Some(modified)),
            Err(failure) => failure,
        };
        // Discard every byte from the failed fragment, including bytes
        // reported to observers, before waiting, retrying or skipping.
        file.truncate(prefix)?;
        *stored = prefix;
        // HttpFD reports it (`to_stderr('\n')`, then `report_error`, so the
        // run exits 1) and FragmentFD goes on without the fragment.
        let no_data =
            matches!(&failure.error, Error::Reported(message) if message == NO_DATA_BLOCKS);
        if no_data {
            crate::screen::stderr("\n");
            crate::report_trouble(NO_DATA_BLOCKS);
        }
        // HttpFD declined the fragment (`--min/max-filesize`, unreported by
        // its quiet downloader); like an empty one, it is not retried.
        let declined = no_data || matches!(failure.error, Error::FileSizeLimit(_));
        // Upstream retries every HTTP status error for fragments when
        // unavailable fragments may be skipped.
        let retry = failure.retry.or_else(|| {
            (options.skip_unavailable_fragments.is_some()
                && (matches!(&failure.error, Error::Http(source) if source.status().is_some())
                    || matches!(failure.error, Error::HttpStatus { .. })))
            .then_some(Duration::ZERO)
        });
        if let Some(retry_after) = retry
            && attempt < options.fragment_retries
        {
            let delay = crate::retry::announce_retry(
                &failure.error,
                attempt,
                options.fragment_retries,
                Some(index as u32 + 1),
                options.retry_sleep.fragment,
            )
            .map_or_else(
                || crate::retry::delay(attempt, retry_after),
                |delay| delay.max(retry_after),
            );
            reporter.wait(delay, *stored)?;
            continue;
        }
        // Upstream `skip_unavailable_fragments`: HlsFD and IsmFD skip any
        // fragment, DashSegmentsFD never the first; IsmFD fails on a
        // fragment without data.
        let protocol = info.protocol.as_deref().unwrap_or_default();
        let ism = protocol == "ism";
        let skippable = if declined && ism {
            false
        } else {
            index > 0 || ism || protocol.starts_with("m3u8")
        };
        if options.skip_unavailable_fragments == Some(true)
            && skippable
            && !matches!(failure.error, Error::DownloadCancelled | Error::Io(_))
        {
            crate::screen::screen(&format!(
                "[download] fragment not found; Skipping fragment {} ...",
                index + 1
            ));
            return Ok(None);
        }
        return Err(if retry.is_some() {
            Error::FragmentUnavailable {
                index: index + 1,
                source: Box::new(crate::retry::exhausted(
                    failure.error,
                    options.fragment_retries,
                )),
            }
        } else if declined {
            Error::Reported(format!(
                "fragment {} not found, unable to continue",
                index + 1
            ))
        } else {
            failure.error
        });
    }
    unreachable!("the final attempt returns or errors")
}

/// Where a fragment's bytes go: the output file, or a concurrent worker's
/// [`Spool`].
trait Sink: Write {
    /// Keeps the first `len` bytes and writes after them.
    fn truncate(&mut self, len: u64) -> std::io::Result<()>;
    /// The length the response announces for the bytes about to be written.
    fn expect(&mut self, _len: u64) {}
}

impl Sink for fs::File {
    fn truncate(&mut self, len: u64) -> std::io::Result<()> {
        self.set_len(len)?;
        self.seek(SeekFrom::Start(len)).map(drop)
    }
}

/// Memory the spools of one concurrent download allocate together.
const SPOOL_MEMORY: usize = 32 << 20;

/// The memory the spools of one concurrent download share.
#[derive(Default)]
struct SpoolMemory {
    /// What every spool reserved, at most [`SPOOL_MEMORY`].
    held: AtomicUsize,
    /// Released buffers for the next fragments (at most a quarter of the
    /// budget), so equal-sized fragments do not fragment the heap.
    spare: std::sync::Mutex<Vec<Vec<u8>>>,
}

/// A concurrent worker's fragment until the assembling thread appends it:
/// in memory while the download's spools hold less than [`SPOOL_MEMORY`],
/// then in a private file, so most bytes are written to disk only once.
struct Spool<'a> {
    memory: Vec<u8>,
    file: Option<tempfile::NamedTempFile>,
    len: u64,
    /// This spool's share of `budget.held`.
    reserved: usize,
    budget: &'a SpoolMemory,
    parent: &'a Path,
}

impl<'a> Spool<'a> {
    fn new(budget: &'a SpoolMemory, parent: &'a Path) -> Self {
        Spool {
            memory: Vec::new(),
            file: None,
            len: 0,
            reserved: 0,
            budget,
            parent,
        }
    }

    fn reader(&self) -> std::io::Result<Box<dyn Read + '_>> {
        Ok(match &self.file {
            Some(file) => Box::new(file.reopen()?),
            None => Box::new(self.memory.as_slice()),
        })
    }

    fn release(&mut self) {
        self.budget.held.fetch_sub(self.reserved, Ordering::Relaxed);
        self.reserved = 0;
        let mut memory = std::mem::take(&mut self.memory);
        if memory.capacity() > 0
            && let Ok(mut spare) = self.budget.spare.lock()
            && spare.iter().map(Vec::capacity).sum::<usize>() + memory.capacity()
                <= SPOOL_MEMORY / 4
        {
            memory.clear();
            spare.push(memory);
        }
    }
}

impl Write for Spool<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.file.is_none() {
            // Doubling like `Vec`, but counted before it allocates.
            let needed = self.memory.len() + bytes.len();
            let more = needed
                .saturating_sub(self.reserved)
                .max(if needed > self.reserved {
                    self.reserved
                } else {
                    0
                });
            let held = self.budget.held.fetch_add(more, Ordering::Relaxed);
            if held + more <= SPOOL_MEMORY {
                self.reserved += more;
                self.memory.reserve_exact(self.reserved - self.memory.len());
                self.memory.extend_from_slice(bytes);
                self.len += bytes.len() as u64;
                return Ok(bytes.len());
            }
            self.budget.held.fetch_sub(more, Ordering::Relaxed);
            let mut file = tempfile::Builder::new()
                .prefix(&crate::scratch::prefix(".yt-dlp-rs-fragment-", self.parent))
                .tempfile_in(self.parent)?;
            file.write_all(&self.memory)?;
            self.release();
            self.file = Some(file);
        }
        let written = self.file.as_mut().expect("spilled above").write(bytes)?;
        self.len += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Sink for Spool<'_> {
    fn truncate(&mut self, len: u64) -> std::io::Result<()> {
        match &mut self.file {
            Some(file) => file.as_file_mut().truncate(len)?,
            None => self.memory.truncate(len as usize),
        }
        self.len = len;
        Ok(())
    }

    /// Reserves the announced length at once, so the budget counts the
    /// fragment's size instead of its doubling growth.
    fn expect(&mut self, len: u64) {
        let Some(more) = usize::try_from(len)
            .ok()
            .filter(|len| *len <= SPOOL_MEMORY && self.file.is_none())
            .and_then(|len| (self.memory.len() + len).checked_sub(self.reserved))
            .filter(|more| *more > 0)
        else {
            return;
        };
        if self.budget.held.fetch_add(more, Ordering::Relaxed) + more <= SPOOL_MEMORY {
            self.reserved += more;
            if self.memory.capacity() == 0
                && let Ok(mut spare) = self.budget.spare.lock()
                && let Some(index) = spare
                    .iter()
                    .position(|buffer| buffer.capacity() >= self.reserved)
            {
                self.memory = spare.swap_remove(index);
            }
            self.memory.reserve_exact(self.reserved - self.memory.len());
        } else {
            self.budget.held.fetch_sub(more, Ordering::Relaxed);
        }
    }
}

impl Drop for Spool<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

impl Spool<'_> {
    /// Appends the fragment to `output` whole.
    fn append_to(&self, output: &mut fs::File) -> Result<()> {
        append_whole(output, |output| match &self.file {
            Some(spilled) => std::io::copy(&mut spilled.reopen()?, output).map(drop),
            None => output.write_all(&self.memory),
        })
    }
}

/// Appends with `write`, or (when it fails, e.g. on a full disk) truncates
/// `output` back, so a resumed download never builds on part of a fragment.
fn append_whole(
    output: &mut fs::File,
    write: impl FnOnce(&mut fs::File) -> std::io::Result<()>,
) -> Result<()> {
    let before = output.stream_position()?;
    write(output).map_err(|error| {
        // The write's error matters; restoring the length is best effort.
        let _ = output
            .set_len(before)
            .and_then(|()| output.seek(SeekFrom::Start(before)).map(drop));
        error.into()
    })
}

/// A sequential fragment until it is whole: kept in a [`Spool`], then
/// appended to `output`, so a killed download leaves only whole fragments
/// in the part file (upstream's `-FragN.part` files, mostly in memory).
struct Staged<'a, 'b> {
    output: &'b mut fs::File,
    /// The output's length, where the fragment starts.
    base: u64,
    spool: Spool<'a>,
}

impl Staged<'_, '_> {
    fn append(self) -> Result<()> {
        self.spool.append_to(self.output)
    }
}

impl Write for Staged<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.spool.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Sink for Staged<'_, '_> {
    fn truncate(&mut self, len: u64) -> std::io::Result<()> {
        self.spool.truncate(len.saturating_sub(self.base))
    }

    fn expect(&mut self, len: u64) {
        self.spool.expect(len);
    }
}

/// Holds concurrent workers back while the assembling thread, not the
/// network, is behind: once the next fragment to append has arrived, a
/// fragment starts only while the fragments queued for appending and
/// those in flight still fit in [`SPOOL_MEMORY`] (at the largest size seen
/// so far). Running further ahead would only spill to disk, writing every
/// byte twice. While the next fragment is still in flight (a slow server
/// or a retry), the others keep going like upstream's, spilling if needed.
struct Window {
    state: std::sync::Mutex<WindowState>,
    moved: std::sync::Condvar,
}

struct WindowState {
    /// The next fragment to append.
    cursor: usize,
    /// Downloaded fragments not appended yet, and their bytes.
    arrived: std::collections::HashSet<usize>,
    queued: u64,
    /// Fragments being downloaded.
    fetching: u64,
    /// The largest fragment so far, capped so that one of that size per
    /// worker always fits.
    largest: u64,
    cap: u64,
}

impl Window {
    fn new(cursor: usize, concurrency: usize) -> Self {
        Window {
            state: std::sync::Mutex::new(WindowState {
                cursor,
                arrived: Default::default(),
                queued: 0,
                fetching: 0,
                largest: 0,
                cap: SPOOL_MEMORY as u64 / (concurrency as u64 + 1),
            }),
            moved: std::sync::Condvar::new(),
        }
    }

    /// Waits until fragment `index` may start, and counts it as in flight.
    fn start(&self, index: usize, cancelled: &std::sync::atomic::AtomicBool) {
        let mut state = self.state.lock().unwrap();
        while index != state.cursor
            && !cancelled.load(Ordering::Relaxed)
            && state.arrived.contains(&state.cursor)
            && state.queued + (state.fetching + 1) * state.largest > SPOOL_MEMORY as u64
        {
            state = self
                .moved
                .wait_timeout(state, crate::http::interruptible::POLL)
                .unwrap()
                .0;
        }
        state.fetching += 1;
    }

    /// Fragment `index` ended: downloaded with `len` bytes, or not.
    fn arrived(&self, index: usize, len: Option<u64>) {
        let mut state = self.state.lock().unwrap();
        state.fetching -= 1;
        if let Some(len) = len {
            state.arrived.insert(index);
            state.queued += len;
            state.largest = state.largest.max(len.min(state.cap));
        }
    }

    /// Fragment `index`, with `len` bytes, was appended (or skipped).
    fn appended(&self, index: usize, len: u64) {
        let mut state = self.state.lock().unwrap();
        state.arrived.remove(&index);
        state.queued -= len;
        state.cursor = index + 1;
        drop(state);
        self.moved.notify_all();
    }
}

/// A fragment worker's report to the assembling thread.
enum Event<'a> {
    Progress {
        index: usize,
        bytes: u64,
        /// The fragment's HttpFD progress, when the event has one.
        fragment: Option<crate::FragmentBytes>,
    },
    Done {
        index: usize,
        result: Result<Option<(Spool<'a>, Option<String>)>>,
    },
}

/// Upstream `--concurrent-fragments`: `concurrency` workers fetch fragments
/// (each with its own retries) into [`Spool`]s, appended in order.
/// Returns the stored length and the latest fragment filetime.
#[allow(clippy::too_many_arguments)]
fn download_concurrent(
    client: &Client,
    info: &VideoInfo,
    options: &DownloadOptions,
    concurrency: usize,
    start: usize,
    parent: &Path,
    output: &mut fs::File,
    reporter: &mut Reporter<'_>,
    keep_fragment: &dyn Fn(usize, &mut dyn Read) -> Result<()>,
    appended: &mut dyn FnMut(usize) -> Result<()>,
) -> Result<(u64, Option<i64>)> {
    use std::sync::{Arc, atomic::AtomicBool, mpsc};
    let budget = SpoolMemory::default();
    let budget = &budget;
    let window = Window::new(start, concurrency);
    let window = &window;
    let next = Arc::new(AtomicUsize::new(start));
    let cancelled = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel::<Event>();
    // Workers' screen messages (retries, skipped fragments), written by the
    // assembling thread, which may hold the output streams.
    let (message_sender, messages) = mpsc::channel::<crate::screen::OwnedMessage>();
    let count = info.fragments.len();
    std::thread::scope(|scope| -> Result<(u64, Option<i64>)> {
        for _ in 0..concurrency.min(count - start) {
            let (next, cancelled, sender) = (next.clone(), cancelled.clone(), sender.clone());
            let messages = message_sender.clone();
            let context = crate::control::current();
            scope.spawn(move || {
                let _context = crate::control::resume(context);
                let _forward = crate::screen::forward_to(messages);
                let mut keys: HashMap<url::Url, [u8; 16]> =
                    match crate::manifest_args::external_key(info).ok().flatten() {
                        Some(key) => info
                            .fragments
                            .iter()
                            .filter_map(|fragment| fragment.encryption.as_ref())
                            .map(|encryption| (encryption.key_url.clone(), key))
                            .collect(),
                        None => HashMap::new(),
                    };
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= count {
                        return;
                    }
                    window.start(index, &cancelled);
                    let progress = sender.clone();
                    let flag = cancelled.clone();
                    let mut forward = move |event: DownloadProgress| {
                        let _ = progress.send(Event::Progress {
                            index,
                            bytes: event.downloaded_bytes,
                            fragment: event.fragment,
                        });
                        if flag.load(Ordering::Relaxed) {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        }
                    };
                    let mut worker = Reporter {
                        callback: &mut forward,
                        attempt: 1,
                        index: index as u32 + 1,
                        count: count as u32,
                        rate_limit: options.rate_limit,
                        fragment: None,
                        live: crate::live_snapshot(info),
                    };
                    let result = (|| {
                        let mut file = Spool::new(budget, parent);
                        let mut stored = 0;
                        let outcome = fetch_with_retries(
                            client,
                            info,
                            index,
                            &info.fragments[index],
                            options,
                            &mut keys,
                            &mut file,
                            0,
                            &mut stored,
                            &mut worker,
                        )?;
                        // HttpFD's `finished` status for the fragment.
                        if outcome.is_some() {
                            let _ = sender.send(Event::Progress {
                                index,
                                bytes: stored,
                                fragment: Some(crate::FragmentBytes {
                                    index: index as u32,
                                    downloaded: stored,
                                    total: Some(stored),
                                    finished: true,
                                    live: false,
                                }),
                            });
                        }
                        Ok(outcome.map(|modified| (file, modified)))
                    })();
                    let failed = result.is_err();
                    window.arrived(
                        index,
                        match &result {
                            Ok(file) => Some(file.as_ref().map_or(0, |(file, _)| file.len)),
                            Err(_) => None,
                        },
                    );
                    let _ = sender.send(Event::Done { index, result });
                    if failed {
                        return;
                    }
                }
            });
        }
        drop(sender);
        // Any early return (a failed append) stops the workers too, instead
        // of leaving them to fetch every remaining fragment.
        struct StopWorkers<'a>(&'a AtomicBool, &'a Window);
        impl Drop for StopWorkers<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
                self.1.moved.notify_all();
            }
        }
        let _stop = StopWorkers(&cancelled, window);
        // Assemble in order while reporting the in-flight total.
        let mut finished: HashMap<usize, Option<(Spool<'_>, Option<String>)>> = HashMap::new();
        let mut in_flight: HashMap<usize, u64> = HashMap::new();
        let (mut written, mut done, mut cursor) = (0_u64, start, start);
        let mut latest = None;
        let mut outcome = Ok(());
        for event in receiver {
            messages.try_iter().for_each(|message| message.emit());
            match event {
                Event::Progress {
                    index,
                    bytes,
                    fragment,
                } => {
                    in_flight.insert(index, bytes);
                    // Each fragment's HttpFD progress reaches the hook
                    // (FragmentFD's `frag_progress_hook` from every thread).
                    reporter.fragment = fragment;
                }
                Event::Done { index, result } => match result {
                    Ok(file) => {
                        in_flight.remove(&index);
                        finished.insert(index, file);
                        done += 1;
                    }
                    Err(error) => {
                        cancelled.store(true, Ordering::Relaxed);
                        window.moved.notify_all();
                        if outcome.is_ok() {
                            outcome = Err(error);
                        }
                    }
                },
            }
            while let Some(file) = finished.remove(&cursor) {
                let len = file.as_ref().map_or(0, |(file, _)| file.len);
                if let Some((file, modified)) = file {
                    file.append_to(output)?;
                    written += file.len;
                    keep_fragment(cursor, &mut file.reader()?)?;
                    appended(cursor)?;
                    latest = crate::filetime::filetime(modified.as_deref()).or(latest);
                }
                window.appended(cursor, len);
                cursor += 1;
            }
            if outcome.is_ok() {
                let pending: u64 = finished.values().flatten().map(|(file, _)| file.len).sum();
                reporter.index = (done + 1).min(count) as u32;
                reporter.attempt = 1;
                let total = written + pending + in_flight.values().sum::<u64>();
                if reporter.report(DownloadPhase::Downloading, total).is_err() {
                    cancelled.store(true, Ordering::Relaxed);
                    window.moved.notify_all();
                    outcome = Err(Error::DownloadCancelled);
                }
            }
        }
        messages.try_iter().for_each(|message| message.emit());
        outcome?;
        Ok((written, latest))
    })
}

/// One fragment's transfer; `limits` are `--min-filesize`/`--max-filesize`,
/// which HttpFD also applies to each fragment's declared length.
#[allow(clippy::too_many_arguments)]
fn fetch(
    client: &Client,
    info: &VideoInfo,
    fragment: &Fragment,
    keys: &mut HashMap<url::Url, [u8; 16]>,
    file: &mut dyn Sink,
    stored: &mut u64,
    reporter: &mut Reporter<'_>,
    limits: (Option<u64>, Option<u64>),
) -> std::result::Result<Option<String>, Failure> {
    let decryption = if let Some(encryption) = &fragment.encryption {
        let key = match keys.get(&encryption.key_url) {
            Some(key) => *key,
            None => {
                let key = fetch_key(client, info, &encryption.key_url, *stored, reporter)?;
                keys.insert(encryption.key_url.clone(), key);
                key
            }
        };
        reporter.report(DownloadPhase::Requesting, *stored)?;
        Some(crate::crypto::Decryptor::new(&key, &encryption.iv))
    } else {
        None
    };
    let mut request = client
        .get(fragment.url.clone())
        .header(ACCEPT_ENCODING, "identity");
    if !info.is_direct() {
        request = request.header(REFERER, info.webpage_url.as_str());
    }
    if let Some(ip) = crate::geo::forwarded_for(info) {
        request = request.header("X-Forwarded-For", ip);
    }
    if let Some(range) = &fragment.byte_range {
        request = request.header(
            RANGE,
            format!("bytes={}-{}", range.offset, range.offset + range.length - 1),
        );
    }
    let mut response = crate::http::send_media(request)
        .inspect(|_| crate::screen::after_request())
        .and_then(crate::retry::check_response)
        .map_err(|error| Failure {
            retry: crate::retry::failure_delay(&error),
            error,
        })?;
    // MhtmlFD's storyboard fragments are images.
    let images = info.protocol.as_deref() == Some("mhtml");
    let expected = validate(&response, fragment, false, images)?;
    if let Some(length) = expected
        && (limits.0.is_some_and(|min| length < min) || limits.1.is_some_and(|max| length > max))
    {
        return Err(Error::FileSizeLimit(format!("fragment of {length} bytes")).into());
    }
    if let Some(expected) = expected {
        file.expect(expected);
    }
    let modified = crate::filetime::header(&response);
    reporter.report(DownloadPhase::Downloading, *stored)?;
    if let Some(decryptor) = decryption {
        if expected.is_some_and(|length| length == 0 || length % 16 != 0) {
            return Err(
                unsupported("AES-128 ciphertext length must be a positive multiple of 16").into(),
            );
        }
        return decrypt_response(response, expected, decryptor, file, stored, reporter)
            .map(|()| modified);
    }
    let mut received = 0_u64;
    // HttpFD's blocks; enough leading bytes are buffered to recognize split
    // HTML/manifest responses before anything is written.
    let mut limiter = crate::download::RateLimit::new(reporter.rate_limit);
    let mut blocks = crate::download::Blocks::new();
    let mut pending: Option<Vec<u8>> = Some(Vec::new());
    // Bytes off the wire, some of them held in `pending` before writing.
    let (url, mut read) = (response.url().clone(), 0_u64);
    loop {
        let block = blocks
            .read(&mut response, None)
            .map_err(|error| body_failure(crate::http::body_error(error, &url, read, expected)))?;
        let count = block.len();
        read += count as u64;
        limiter.consume(count);
        let data = match &mut pending {
            Some(buffered) => {
                buffered.extend_from_slice(block);
                if buffered.len() < 512 && count > 0 {
                    blocks.finished(count);
                    continue;
                }
                sniff(&buffered[..buffered.len().min(512)])?;
                std::borrow::Cow::Owned(pending.take().unwrap())
            }
            None if count == 0 => break,
            None => std::borrow::Cow::Borrowed(block),
        };
        if !data.is_empty() {
            reporter.fragment = Some(crate::FragmentBytes {
                index: reporter.index.saturating_sub(1),
                downloaded: received + data.len() as u64,
                total: expected,
                finished: false,
                live: false,
            });
            append(file, &data, stored, &mut received, expected, reporter)?;
        }
        drop(data);
        blocks.finished(count);
        if count == 0 {
            break;
        }
    }
    if let Some(expected) = expected
        && received != expected
    {
        return Err(body_failure(Error::IncompleteDownload {
            expected,
            actual: received,
        }));
    }
    if received == 0 {
        return Err(Error::Reported(NO_DATA_BLOCKS.into()).into());
    }
    Ok(modified)
}
fn append(
    file: &mut dyn Sink,
    bytes: &[u8],
    stored: &mut u64,
    received: &mut u64,
    expected: Option<u64>,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<(), Failure> {
    let next = received
        .checked_add(bytes.len() as u64)
        .ok_or(Error::InvalidRangeResponse)?;
    if expected.is_some_and(|expected| next > expected) {
        return Err(Error::InvalidRangeResponse.into());
    }
    let next_stored = stored
        .checked_add(bytes.len() as u64)
        .ok_or(Error::InvalidRangeResponse)?;
    file.write_all(bytes)?;
    *received = next;
    *stored = next_stored;
    reporter.report(DownloadPhase::Downloading, *stored)?;
    Ok(())
}
fn validate(
    response: &Response,
    fragment: &Fragment,
    key: bool,
    images: bool,
) -> Result<Option<u64>> {
    let encoding: Vec<_> = response
        .headers()
        .get_all(CONTENT_ENCODING)
        .iter()
        .collect();
    if encoding.len() > 1
        || encoding.first().is_some_and(|value| {
            value
                .to_str()
                .map_or(true, |value| !value.eq_ignore_ascii_case("identity"))
        })
    {
        return Err(unsupported("encoded fragment response"));
    }
    let types: Vec<_> = response.headers().get_all(CONTENT_TYPE).iter().collect();
    if types.len() > 1 {
        return Err(unsupported("ambiguous fragment content type"));
    }
    if let Some(value) = types.first() {
        let mime = value
            .to_str()
            .map_err(|_| unsupported("invalid fragment content type"))?
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime.contains("mpegurl") || mime == "application/dash+xml" {
            return Err(unsupported("fragment response is another manifest"));
        }
        if key
            && matches!(
                mime.as_str(),
                "text/html" | "application/xhtml+xml" | "application/xml" | "text/xml"
            )
        {
            return Err(unsupported(
                "HLS AES-128 key response is HTML or a manifest",
            ));
        }
        if !key
            && !(mime.starts_with("video/")
                || mime.starts_with("audio/")
                || (images && mime.starts_with("image/"))
                || matches!(
                    mime.as_str(),
                    "application/octet-stream"
                        | "binary/octet-stream"
                        | "application/mp4"
                        | "application/ogg"
                ))
        {
            return Err(unsupported("fragment response is not media"));
        }
    }
    if let Some(range) = &fragment.byte_range {
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
            || response.headers().get_all(CONTENT_RANGE).iter().count() != 1
        {
            return Err(Error::InvalidRangeResponse);
        }
        let value = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .ok_or(Error::InvalidRangeResponse)?;
        let (span, total) = value
            .strip_prefix("bytes ")
            .and_then(|value| value.split_once('/'))
            .ok_or(Error::InvalidRangeResponse)?;
        let (start, end) = span.split_once('-').ok_or(Error::InvalidRangeResponse)?;
        let number = |s: &str| {
            if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
                s.parse::<u64>().ok()
            } else {
                None
            }
        };
        let end_expected = range.offset + range.length - 1;
        if number(start) != Some(range.offset)
            || number(end) != Some(end_expected)
            || !number(total).is_some_and(|total| total > end_expected)
            || response
                .content_length()
                .is_some_and(|length| length != range.length)
        {
            return Err(Error::InvalidRangeResponse);
        }
        Ok(Some(range.length))
    } else {
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT
            || response.headers().contains_key(CONTENT_RANGE)
        {
            return Err(Error::PartialResponse);
        }
        Ok(response.content_length())
    }
}

fn sniff(prefix: &[u8]) -> Result<()> {
    let text = String::from_utf8_lossy(prefix)
        .trim_start_matches('\u{feff}')
        .trim_start()
        .to_ascii_lowercase();
    if text.starts_with("<!doctype html")
        || text.starts_with("<html")
        || text.starts_with("#extm3u")
        || text.starts_with("<?xml")
        || text.starts_with("<mpd")
    {
        return Err(unsupported("fragment returned HTML or another manifest"));
    }
    Ok(())
}

fn fetch_key(
    client: &Client,
    info: &VideoInfo,
    url: &url::Url,
    stored: u64,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<[u8; 16], Failure> {
    reporter.report(DownloadPhase::Requesting, stored)?;
    let mut request = client.get(url.clone()).header(ACCEPT_ENCODING, "identity");
    if !info.is_direct() {
        request = request.header(REFERER, info.webpage_url.as_str());
    }
    if let Some(ip) = crate::geo::forwarded_for(info) {
        request = request.header("X-Forwarded-For", ip);
    }
    let mut response = crate::http::send_media(request)
        .inspect(|_| crate::screen::after_request())
        .and_then(crate::retry::check_response)
        .map_err(|error| Failure {
            retry: crate::retry::failure_delay(&error),
            error,
        })?;
    let key_fragment = Fragment {
        url: url.clone(),
        byte_range: None,
        encryption: None,
        path: None,
        duration: None,
        fragment_count: None,
    };
    let expected = validate(&response, &key_fragment, true, false)?;
    if expected.is_some_and(|length| length != 16) {
        return Err(unsupported("HLS AES-128 key must contain exactly 16 bytes").into());
    }
    let mut bytes = [0_u8; 17];
    let mut received = 0;
    while received < bytes.len() {
        reporter.report(DownloadPhase::Downloading, stored)?;
        let count = match response.read(&mut bytes[received..]) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let error = crate::http::body_error(error, url, received as u64, expected);
                return Err(body_failure(error));
            }
        };
        received += count;
    }
    if received != 16 {
        return Err(unsupported("HLS AES-128 key must contain exactly 16 bytes").into());
    }
    let mut key = [0; 16];
    key.copy_from_slice(&bytes[..16]);
    Ok(key)
}

fn decrypt_response(
    mut response: Response,
    expected: Option<u64>,
    mut decryptor: crate::crypto::Decryptor,
    file: &mut dyn Sink,
    stored: &mut u64,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<(), Failure> {
    let mut cipher_bytes = 0_u64;
    let mut plain_bytes = 0_u64;
    let mut prefix = Vec::with_capacity(512);
    let mut sniffed = false;
    let mut buffer = [0; 32 * 1024];
    let mut limiter = crate::download::RateLimit::new(reporter.rate_limit);
    let url = response.url().clone();
    loop {
        reporter.report(DownloadPhase::Downloading, *stored)?;
        let count = match response.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let error = crate::http::body_error(error, &url, cipher_bytes, expected);
                return Err(body_failure(error));
            }
        };
        limiter.consume(count);
        cipher_bytes = cipher_bytes
            .checked_add(count as u64)
            .ok_or(Error::InvalidRangeResponse)?;
        if expected.is_some_and(|expected| cipher_bytes > expected) {
            return Err(Error::InvalidRangeResponse.into());
        }
        let plain = decryptor.update(&buffer[..count]);
        write_plain(
            &plain,
            &mut prefix,
            &mut sniffed,
            file,
            stored,
            &mut plain_bytes,
            reporter,
        )?;
    }
    if cipher_bytes == 0 {
        return Err(Error::Reported(NO_DATA_BLOCKS.into()).into());
    }
    if let Some(expected) = expected
        && cipher_bytes != expected
    {
        return Err(body_failure(Error::IncompleteDownload {
            expected,
            actual: cipher_bytes,
        }));
    }
    let final_plain = decryptor.finish()?;
    write_plain(
        &final_plain,
        &mut prefix,
        &mut sniffed,
        file,
        stored,
        &mut plain_bytes,
        reporter,
    )?;
    if !sniffed {
        sniff(&prefix)?;
        append(file, &prefix, stored, &mut plain_bytes, None, reporter)?;
    }
    if plain_bytes == 0 {
        return Err(unsupported("empty media fragment").into());
    }
    Ok(())
}

fn write_plain(
    bytes: &[u8],
    prefix: &mut Vec<u8>,
    sniffed: &mut bool,
    file: &mut dyn Sink,
    stored: &mut u64,
    plain_bytes: &mut u64,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<(), Failure> {
    if *sniffed {
        return append(file, bytes, stored, plain_bytes, None, reporter);
    }
    let take = bytes.len().min(512 - prefix.len());
    prefix.extend_from_slice(&bytes[..take]);
    if prefix.len() == 512 {
        sniff(prefix)?;
        append(file, prefix, stored, plain_bytes, None, reporter)?;
        prefix.clear();
        *sniffed = true;
        append(file, &bytes[take..], stored, plain_bytes, None, reporter)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed append (a full disk) leaves the output as it was, so a
    /// resumed download continues after whole fragments only.
    #[test]
    fn a_failed_append_truncates_the_output_back() {
        let mut output = tempfile::tempfile().unwrap();
        output.write_all(b"whole").unwrap();
        let result = append_whole(&mut output, |output| {
            output.write_all(b"part")?;
            Err(std::io::Error::other("No space left on device"))
        });
        assert!(result.is_err());
        assert_eq!(output.metadata().unwrap().len(), 5);
        output.write_all(b"!").unwrap();
        let mut bytes = Vec::new();
        output.seek(SeekFrom::Start(0)).unwrap();
        output.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"whole!");
    }

    fn contents(spool: &Spool<'_>) -> Vec<u8> {
        let mut bytes = Vec::new();
        spool.reader().unwrap().read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn spool_keeps_fragments_in_memory_within_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let budget = SpoolMemory::default();
        let mut spool = Spool::new(&budget, dir.path());
        spool.write_all(b"abc").unwrap();
        spool.write_all(b"defgh").unwrap();
        assert!(spool.file.is_none());
        assert_eq!(spool.len, 8);
        assert_eq!(budget.held.load(Ordering::Relaxed), spool.reserved);
        assert!(spool.memory.capacity() >= spool.reserved);
        // A failed attempt starts the fragment over.
        spool.truncate(0).unwrap();
        spool.write_all(b"xyz").unwrap();
        assert_eq!(contents(&spool), b"xyz");
        drop(spool);
        assert_eq!(budget.held.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn spool_reserves_an_announced_length_once() {
        let dir = tempfile::tempdir().unwrap();
        let budget = SpoolMemory::default();
        let mut spool = Spool::new(&budget, dir.path());
        spool.expect(1000);
        assert_eq!(budget.held.load(Ordering::Relaxed), 1000);
        for _ in 0..10 {
            spool.write_all(&[7; 100]).unwrap();
        }
        // Writing the announced bytes neither grows nor reallocates.
        assert_eq!(budget.held.load(Ordering::Relaxed), 1000);
        assert_eq!(spool.memory.capacity(), 1000);
        // A retry announces the same length again.
        spool.truncate(0).unwrap();
        spool.expect(1000);
        assert_eq!(budget.held.load(Ordering::Relaxed), 1000);
        // Lengths beyond the budget are left to the writes.
        let mut large = Spool::new(&budget, dir.path());
        large.expect(SPOOL_MEMORY as u64 + 1);
        assert_eq!(large.reserved, 0);
        drop((spool, large));
        assert_eq!(budget.held.load(Ordering::Relaxed), 0);
        // The released buffer serves the next fragment of its size.
        assert_eq!(budget.spare.lock().unwrap().len(), 1);
        let mut next = Spool::new(&budget, dir.path());
        next.expect(1000);
        assert_eq!(next.memory.capacity(), 1000);
        assert!(budget.spare.lock().unwrap().is_empty());
        drop(next);
        assert_eq!(budget.held.load(Ordering::Relaxed), 0);
    }

    /// Whether `window.start(index)` returns (generously timed for loaded
    /// machines; callers check for waiting with their own short wait).
    fn starts(window: &std::sync::Arc<Window>, index: usize) -> bool {
        let (sender, receiver) = std::sync::mpsc::channel();
        let window = window.clone();
        std::thread::spawn(move || {
            window.start(index, &std::sync::atomic::AtomicBool::new(false));
            let _ = sender.send(());
        });
        receiver.recv_timeout(Duration::from_secs(10)).is_ok()
    }

    #[test]
    fn window_runs_ahead_of_a_fragment_still_in_flight() {
        let window = std::sync::Arc::new(Window::new(0, 4));
        let size = 1 << 20;
        assert!(starts(&window, 0));
        // Fragment 0 is slow: far more than the budget may queue behind it.
        for index in 1..100 {
            assert!(starts(&window, index), "fragment {index}");
            window.arrived(index, Some(size));
        }
    }

    #[test]
    fn window_waits_for_appending_once_the_next_fragment_arrived() {
        let window = std::sync::Arc::new(Window::new(0, 4));
        let size = 1 << 20;
        let mut index = 0;
        // Fill the budget behind an arrived fragment 0.
        while (index as u64 + 1) * size <= SPOOL_MEMORY as u64 {
            assert!(starts(&window, index), "fragment {index}");
            window.arrived(index, Some(size));
            index += 1;
        }
        let waiting = {
            let (sender, receiver) = std::sync::mpsc::channel();
            let window = window.clone();
            std::thread::spawn(move || {
                window.start(index, &std::sync::atomic::AtomicBool::new(false));
                let _ = sender.send(());
            });
            receiver
        };
        assert!(waiting.recv_timeout(Duration::from_millis(300)).is_err());
        // Appending makes room.
        window.appended(0, size);
        window.appended(1, size);
        assert!(waiting.recv_timeout(Duration::from_secs(5)).is_ok());
        // The next fragment to append always starts.
        assert!(starts(&window, 2));
    }

    #[test]
    fn spool_moves_to_a_file_when_the_download_reaches_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let budget = SpoolMemory::default();
        let mut first = Spool::new(&budget, dir.path());
        first.write_all(&vec![1; SPOOL_MEMORY - 4]).unwrap();
        let mut second = Spool::new(&budget, dir.path());
        second.write_all(b"ab").unwrap();
        assert!(second.file.is_none());
        second.write_all(b"cdefgh").unwrap();
        assert!(second.file.is_some());
        // Its memory went back to the budget when it moved.
        assert_eq!(budget.held.load(Ordering::Relaxed), first.reserved);
        assert_eq!(second.len, 8);
        assert_eq!(contents(&second), b"abcdefgh");
        second.truncate(0).unwrap();
        second.write_all(b"z").unwrap();
        assert_eq!(contents(&second), b"z");
        drop(first);
        assert_eq!(budget.held.load(Ordering::Relaxed), 0);
        let file = second.file.as_ref().unwrap().path().to_owned();
        drop(second);
        assert!(!file.exists());
    }
}
