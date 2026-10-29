//! Upstream `DashSegmentsFD` for `http_dash_segments_generator` formats
//! (YouTube `--live-from-start`): each format's fragments come from its
//! lazy generator and are appended as they are downloaded, the formats in
//! parallel (`download_and_append_fragments_multiple`). Like upstream's live
//! fragment downloads there is no `.ytdl` state file, and an interrupt
//! (SIGINT) ends the lists and completes the files instead of failing.
use crate::{
    DownloadOptions, DownloadPhase, DownloadProgress, Error, PartFiles, Result, VideoInfo,
    extractor::LiveFragments,
};
use reqwest::blocking::Client;
use std::{
    fs,
    io::{Read, Write},
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

/// One format's download.
pub(crate) struct Job {
    /// The result with this format applied (request headers, referrer).
    pub info: VideoInfo,
    pub destination: PathBuf,
    pub fragments: LiveFragments,
    /// `destination` is a named pipe FFmpeg reads (`--growing-file`).
    pub pipe: bool,
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
    let results: Vec<Result<u64>> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .into_iter()
            .enumerate()
            .map(|(index, job)| {
                let (sender, stop) = (sender.clone(), &stop);
                let context = crate::control::current();
                scope.spawn(move || {
                    let _context = crate::control::resume(context);
                    let messages = sender.clone();
                    crate::screen::forward(Some(Box::new(move |message| {
                        let _ = messages.send(Event::Message(message));
                    })));
                    let result = run(client, job, &options, stop, &mut |bytes| {
                        let _ = sender.send(Event::Progress { job: index, bytes });
                    });
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
    let mut sizes = Vec::with_capacity(count);
    for (index, (result, destination)) in results.into_iter().zip(destinations).enumerate() {
        let size = result?;
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
        if callback(finished).is_break() {
            return Err(Error::DownloadCancelled);
        }
        debug_assert!(destination.exists());
    }
    Ok(sizes)
}

/// `_prepare_frag_download` (no `.ytdl` file for a live download), the
/// fragments, then `_finish_frag_download`.
fn run(
    client: &Client,
    mut job: Job,
    options: &DownloadOptions,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<u64> {
    if job.pipe {
        return feed(client, job, options, stop, progress);
    }
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
                    .prefix(".yt-dlp-rs-live-")
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
    let halted = || stop.load(Ordering::SeqCst) || crate::interrupt::interrupted();
    let mut stored = 0_u64;
    let mut index = 0_usize;
    {
        let file = match (&mut private, &mut named) {
            (Some(temporary), _) => temporary.as_file_mut(),
            (_, Some(file)) => file,
            _ => unreachable!("one output file is open"),
        };
        while let Some(fragment) = job.fragments.next(&halted)? {
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
                Err(Error::DownloadCancelled) if crate::interrupt::interrupted() => break,
                Err(error) => return Err(error),
            }
            index += 1;
        }
        if stop.load(Ordering::SeqCst) && !crate::interrupt::interrupted() {
            return Err(Error::DownloadCancelled);
        }
        file.flush()?;
        file.sync_all()?;
    }
    if stored == 0 {
        drop(named);
        if let Some(path) = &part_path {
            let _ = fs::remove_file(path);
        }
        return Err(Error::Reported("The downloaded file is empty".into()));
    }
    match (private, part_path) {
        (Some(temporary), _) => {
            crate::download::persist(temporary, destination, options.overwrite)?
        }
        (None, Some(path)) => {
            drop(named);
            if path != destination {
                fs::rename(&path, destination)?;
            }
        }
        _ => {}
    }
    Ok(stored)
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
    let halted = || stop.load(Ordering::SeqCst) || crate::interrupt::interrupted();
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
            Err(Error::DownloadCancelled) if crate::interrupt::interrupted() => break,
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
    if stop.load(Ordering::SeqCst) && !crate::interrupt::interrupted() {
        return Err(Error::DownloadCancelled);
    }
    Ok(stored)
}
