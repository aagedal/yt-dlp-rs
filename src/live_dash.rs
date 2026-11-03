//! Upstream `DashSegmentsFD` for `http_dash_segments_generator` formats
//! (YouTube `--live-from-start`): each format's fragments come from its
//! lazy generator and are appended as they are downloaded, the formats in
//! parallel (`download_and_append_fragments_multiple`). Like upstream's live
//! fragment downloads there is no `.ytdl` state file, and an interrupt
//! (SIGINT, or a library `Interrupt::stop`) ends the lists and completes
//! the files instead of failing.
//!
//! Port fix: upstream lets the formats drift apart (the small audio
//! fragments race minutes ahead of the video), so a stopped recording
//! merges far more audio than video. Here a stop ends the file formats at
//! their last common fragment, and none gets more than [`AHEAD`] fragments
//! ahead (which the stop would discard).
use crate::{
    DownloadOptions, DownloadPhase, DownloadProgress, Error, PartFiles, Result, VideoInfo,
    extractor::LiveFragments,
};
use reqwest::blocking::Client;
use std::{
    fs,
    io::Read,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

/// How many fragments a format downloaded to a file may get ahead of the
/// slowest other one: enough that one format's slow fragments do not hold
/// the others back, few enough that a stop discards little. The formats'
/// lists share sequence numbers (both start at the stream's first, or the
/// same 120-hour cutoff).
const AHEAD: usize = 10;

/// A lazy live fragment list: the next fragment, `None` once the stream
/// ends. `stop` is polled while waiting (an interrupt ends the list).
pub(crate) trait LiveSource: Send {
    fn next(&mut self, stop: &dyn Fn() -> bool) -> Result<Option<crate::Fragment>>;
}

impl LiveSource for LiveFragments {
    fn next(&mut self, stop: &dyn Fn() -> bool) -> Result<Option<crate::Fragment>> {
        LiveFragments::next(self, stop)
    }
}

/// One format's download.
pub(crate) struct Job {
    /// The result with this format applied (request headers, referrer).
    pub info: VideoInfo,
    pub destination: PathBuf,
    pub fragments: Box<dyn LiveSource>,
    /// `destination` is a named pipe FFmpeg reads (`--growing-file`).
    pub pipe: bool,
}

/// The whole fragments each file format has downloaded, `None` for a
/// format that has ended (or feeds a pipe, which FFmpeg's reads pace).
struct Pace {
    done: Mutex<Vec<Option<usize>>>,
    changed: Condvar,
}

impl Pace {
    /// Wait until `job` may download fragment `index`: no other running
    /// format is more than [`AHEAD`] fragments behind it.
    fn wait(&self, job: usize, index: usize, halted: &dyn Fn() -> bool) {
        let mut done = self.done.lock().unwrap_or_else(|e| e.into_inner());
        while !halted()
            && done.iter().enumerate().any(|(other, count)| {
                other != job && count.is_some_and(|count| count + AHEAD < index)
            })
        {
            done = self
                .changed
                .wait_timeout(done, Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn set(&self, job: usize, count: Option<usize>) {
        self.done.lock().unwrap_or_else(|e| e.into_inner())[job] = count;
        self.changed.notify_all();
    }
}

/// Marks a format ended however its download returns.
struct Leave<'a>(&'a Pace, usize);

impl Drop for Leave<'_> {
    fn drop(&mut self) {
        self.0.set(self.1, None);
    }
}

/// A format's download before it is published.
enum Recorded {
    /// Fed to FFmpeg: the bytes written.
    Fed(u64),
    File {
        output: Output,
        /// The file's size after each fragment.
        ends: Vec<u64>,
    },
}

/// Where a format's fragments were appended.
enum Output {
    Private(tempfile::NamedTempFile),
    /// The `.part` file, or the destination itself with `--no-part`.
    Named(fs::File, PathBuf),
}

/// A worker's report to the calling thread, which may hold the output
/// streams: its downloaded byte count or a screen message.
enum Event {
    Progress { job: usize, bytes: u64 },
    Message(crate::screen::OwnedMessage),
}

/// Download the jobs in parallel; returns each one's size. `streams`
/// numbers the progress (`stream_index`) of a merge's formats.
pub(crate) fn download(
    client: &Client,
    jobs: Vec<Job>,
    options: DownloadOptions,
    streams: bool,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<Vec<u64>> {
    let _interrupt = crate::interrupt::Guard::install();
    let stop = AtomicBool::new(false);
    let count = jobs.len();
    let (sender, receiver) = mpsc::channel::<Event>();
    let destinations: Vec<PathBuf> = jobs.iter().map(|job| job.destination.clone()).collect();
    let pace = Pace {
        done: Mutex::new(jobs.iter().map(|job| (!job.pipe).then_some(0)).collect()),
        changed: Condvar::new(),
    };
    let results: Vec<Result<Recorded>> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .into_iter()
            .enumerate()
            .map(|(index, job)| {
                let (sender, stop, pace) = (sender.clone(), &stop, &pace);
                let context = crate::control::current();
                scope.spawn(move || {
                    let _context = crate::control::resume(context);
                    let messages = sender.clone();
                    crate::screen::forward(Some(Box::new(move |message| {
                        let _ = messages.send(Event::Message(message));
                    })));
                    let pace = Leave(pace, index);
                    let result = run(client, job, &options, stop, &pace, &mut |bytes| {
                        let _ = sender.send(Event::Progress { job: index, bytes });
                    });
                    drop(pace);
                    crate::screen::forward(None);
                    if result.is_err() {
                        stop.store(true, Ordering::SeqCst);
                    }
                    result
                })
            })
            .collect();
        drop(sender);
        let mut bytes = vec![0_u64; count];
        for event in receiver {
            let (job, downloaded) = match event {
                Event::Progress { job, bytes } => (job, bytes),
                Event::Message(message) => {
                    message.emit();
                    continue;
                }
            };
            bytes[job] = downloaded;
            let progress = DownloadProgress {
                attempt: 1,
                downloaded_bytes: bytes.iter().sum(),
                total_bytes: None,
                total_bytes_estimate: None,
                phase: DownloadPhase::Downloading,
                stream_index: streams.then_some(1),
                stream_count: streams.then_some(count as u8),
                fragment_index: None,
                fragment_count: None,
                fragment: None,
                recorded_ms: None,
            };
            if callback(progress).is_break() {
                stop.store(true, Ordering::SeqCst);
            }
        }
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(Error::Reported("a fragment worker panicked".into())))
            })
            .collect()
    });
    let recorded = results.into_iter().collect::<Result<Vec<_>>>()?;
    // A stop ends the file formats at their last common fragment.
    let common = recorded
        .iter()
        .filter_map(|recorded| match recorded {
            Recorded::File { ends, .. } => Some(ends.len()),
            Recorded::Fed(_) => None,
        })
        .min()
        .filter(|_| ending());
    let mut sizes = Vec::with_capacity(count);
    for (index, (recorded, destination)) in recorded.into_iter().zip(destinations).enumerate() {
        let size = match recorded {
            Recorded::Fed(size) => size,
            Recorded::File { output, ends } => {
                let whole = common.unwrap_or(ends.len());
                let size = whole.checked_sub(1).map_or(0, |last| ends[last]);
                publish(output, size, &destination, &options)?
            }
        };
        sizes.push(size);
        // `_finish_frag_download`'s `finished` status for each format.
        let finished = DownloadProgress {
            attempt: 1,
            downloaded_bytes: size,
            total_bytes: Some(size),
            total_bytes_estimate: None,
            phase: DownloadPhase::Publishing,
            stream_index: streams.then_some(index as u8 + 1),
            stream_count: streams.then_some(count as u8),
            fragment_index: None,
            fragment_count: None,
            fragment: None,
            recorded_ms: None,
        };
        // The recording has ended; only a cancel discards it.
        if callback(finished).is_break() && !ending() {
            return Err(Error::DownloadCancelled);
        }
        debug_assert!(destination.exists());
    }
    Ok(sizes)
}

/// Whether the recording ends and is kept: a signal, or a library
/// `Interrupt::stop` (a cancel discards it).
fn ending() -> bool {
    crate::interrupt::interrupted() || crate::control::stop_requested()
}

/// `_prepare_frag_download` (no `.ytdl` file for a live download), the
/// fragments, then `_finish_frag_download`.
fn run(
    client: &Client,
    mut job: Job,
    options: &DownloadOptions,
    stop: &AtomicBool,
    pace: &Leave<'_>,
    progress: &mut dyn FnMut(u64),
) -> Result<Recorded> {
    if job.pipe {
        return feed(client, job, options, stop, progress).map(Recorded::Fed);
    }
    let Leave(pace, slot) = *pace;
    let destination = job.destination.as_path();
    crate::download::check_destination(destination, options)?;
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let (mut private, part_path) = match options.part_files {
        PartFiles::Private => (
            Some(
                tempfile::Builder::new()
                    .prefix(&crate::scratch::prefix(".yt-dlp-rs-live-", parent))
                    .suffix(".part")
                    .tempfile_in(parent)?,
            ),
            None,
        ),
        PartFiles::Part { .. } => {
            let mut name = destination.as_os_str().to_owned();
            name.push(".part");
            (None, Some(PathBuf::from(name)))
        }
        PartFiles::NoPart { .. } => (None, Some(destination.to_path_buf())),
    };
    let mut named = match &part_path {
        Some(path) => Some(fs::File::create(path)?),
        None => None,
    };
    let halted = || stop.load(Ordering::SeqCst) || ending();
    let mut stored = 0_u64;
    let mut ends = Vec::new();
    {
        let file = match (&mut private, &mut named) {
            (Some(temporary), _) => temporary.as_file_mut(),
            (_, Some(file)) => file,
            _ => unreachable!("one output file is open"),
        };
        loop {
            let index = ends.len();
            pace.wait(slot, index, &halted);
            let Some(fragment) = job.fragments.next(&halted)? else {
                break;
            };
            let mut forward = |event: DownloadProgress| {
                progress(event.downloaded_bytes);
                if halted() {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            match crate::fragment::append_live(
                client,
                &job.info,
                index,
                &fragment,
                options,
                file,
                &mut stored,
                &mut forward,
            ) {
                Ok(_) => progress(stored),
                // An interrupted fragment is dropped; the rest is kept.
                Err(Error::DownloadCancelled) if ending() => break,
                Err(error) => return Err(error),
            }
            ends.push(stored);
            pace.set(slot, Some(ends.len()));
        }
        if stop.load(Ordering::SeqCst) && !ending() {
            return Err(Error::DownloadCancelled);
        }
    }
    let output = match (private, named, part_path) {
        (Some(temporary), _, _) => Output::Private(temporary),
        (None, Some(file), Some(path)) => Output::Named(file, path),
        _ => unreachable!("one output file is open"),
    };
    Ok(Recorded::File { output, ends })
}

/// `_finish_frag_download`: the first `size` bytes of `output` at the
/// destination.
fn publish(
    output: Output,
    size: u64,
    destination: &Path,
    options: &DownloadOptions,
) -> Result<u64> {
    let file = match &output {
        Output::Private(temporary) => temporary.as_file(),
        Output::Named(file, _) => file,
    };
    file.set_len(size)?;
    file.sync_all()?;
    if size == 0 {
        if let Output::Named(file, path) = output {
            drop(file);
            let _ = fs::remove_file(path);
        }
        // A library stop before any whole fragment is a cancel.
        if crate::control::stop_requested() {
            return Err(Error::DownloadCancelled);
        }
        return Err(Error::Reported("The downloaded file is empty".into()));
    }
    match output {
        Output::Private(temporary) => {
            crate::download::persist(temporary, destination, options.overwrite)?
        }
        Output::Named(file, path) => {
            drop(file);
            if path != destination {
                fs::rename(&path, destination)?;
            }
        }
    }
    Ok(size)
}

/// `--growing-file` (port extension): each fragment is downloaded into a
/// scratch file (a failed attempt is truncated away) and then written whole
/// to the named pipe FFmpeg reads. The pipe closes at the end, or when
/// interrupted, which lets FFmpeg finish the file.
fn feed(
    client: &Client,
    mut job: Job,
    options: &DownloadOptions,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<u64> {
    // Blocks until FFmpeg opens this input.
    let mut pipe = fs::OpenOptions::new().write(true).open(&job.destination)?;
    let parent = job
        .destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut scratch = tempfile::tempfile_in(parent)?;
    let halted = || stop.load(Ordering::SeqCst) || ending();
    let mut stored = 0_u64;
    let mut index = 0_usize;
    while let Some(fragment) = job.fragments.next(&halted)? {
        let mut forward = |event: DownloadProgress| {
            progress(stored + event.downloaded_bytes);
            if halted() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let mut size = 0_u64;
        match crate::fragment::append_live(
            client,
            &job.info,
            index,
            &fragment,
            options,
            &mut scratch,
            &mut size,
            &mut forward,
        ) {
            Ok(_) => {}
            Err(Error::DownloadCancelled) if ending() => break,
            Err(error) => return Err(error),
        }
        use std::io::{Seek, SeekFrom};
        scratch.seek(SeekFrom::Start(0))?;
        match std::io::copy(&mut (&mut scratch).take(size), &mut pipe) {
            Ok(_) => {}
            // FFmpeg has stopped reading (an interrupt, or it ended); its
            // own result decides the download's.
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => break,
            Err(error) => return Err(error.into()),
        }
        stored += size;
        progress(stored);
        index += 1;
    }
    if stop.load(Ordering::SeqCst) && !ending() {
        return Err(Error::DownloadCancelled);
    }
    Ok(stored)
}
