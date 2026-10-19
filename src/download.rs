use crate::{Error, Result, VideoInfo};
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, ETAG, IF_RANGE, RANGE, REFERER},
};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    ops::ControlFlow,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Checkpoints at which a synchronous download observer can cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPhase {
    Requesting,
    Downloading,
    WaitingToRetry,
    Publishing,
    Merging,
    Postprocessing,
}

/// Bytes stored for the current representation, rather than cumulative traffic.
/// A full-response retry or the next selected stream can reset bytes and total.
/// Merging reports summed input bytes; final publication reports output bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    /// One-based attempt number, including the initial request (maximum 256).
    pub attempt: u16,
    pub downloaded_bytes: u64,
    /// Total from validated download response headers, absent when unknown.
    pub total_bytes: Option<u64>,
    pub phase: DownloadPhase,
    /// One-based selected input stream, present only for merged downloads.
    pub stream_index: Option<u8>,
    pub stream_count: Option<u8>,
    /// One-based fragment position in the finite selected playlist.
    pub fragment_index: Option<u32>,
    pub fragment_count: Option<u32>,
}

struct Reporter<'a> {
    callback: &'a mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
    attempt: u16,
    total: Option<u64>,
}

impl Reporter<'_> {
    fn report(&mut self, phase: DownloadPhase, downloaded_bytes: u64) -> Result<()> {
        match (self.callback)(DownloadProgress {
            attempt: self.attempt,
            downloaded_bytes,
            total_bytes: self.total,
            phase,
            stream_index: None,
            stream_count: None,
            fragment_index: None,
            fragment_count: None,
        }) {
            ControlFlow::Continue(()) => Ok(()),
            ControlFlow::Break(()) => Err(Error::DownloadCancelled),
        }
    }

    fn wait(&mut self, duration: Duration, actual: u64) -> Result<()> {
        let start = Instant::now();
        loop {
            self.report(DownloadPhase::WaitingToRetry, actual)?;
            let remaining = duration.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(remaining.min(Duration::from_millis(100)));
        }
    }
}

/// Download retry policy, independent of `ExtractionOptions`.
#[derive(Debug, Clone, Copy)]
pub struct DownloadOptions {
    /// Additional attempts after a transient failure, from zero to 255.
    pub retries: u8,
    /// `--min-filesize`: abort a whole-file media transfer whose declared
    /// length is smaller (upstream HTTP downloader check).
    pub min_filesize: Option<u64>,
    /// `--max-filesize`: abort a whole-file media transfer whose declared
    /// length is larger.
    pub max_filesize: Option<u64>,
    /// `--limit-rate`: maximum bytes per second for each media transfer or
    /// fragment request (upstream `slow_down`).
    pub rate_limit: Option<u64>,
    /// `--mtime`: set media and subtitle modification times from the
    /// response `Last-Modified` header.
    pub update_time: bool,
    /// Where a whole-file media transfer is written before publication.
    pub part_files: PartFiles,
}

/// Temporary-file policy for whole-file media downloads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PartFiles {
    /// A private same-directory temporary file, removed on failure or
    /// cancellation and published without overwriting (library default).
    #[default]
    Private,
    /// Upstream `FILE.part`, kept on failure; with `resume`, an existing one
    /// is continued with a `Range` request (`--continue`).
    Part { resume: bool },
    /// Upstream `--no-part`: write the destination directly.
    NoPart,
}

/// Upstream `FileDownloader.slow_down`: after each read, sleep until the
/// average speed since the request started is at most the limit.
pub(crate) struct RateLimit {
    limit: Option<u64>,
    start: Instant,
    bytes: u64,
}

impl RateLimit {
    pub(crate) fn new(limit: Option<u64>) -> Self {
        Self {
            limit,
            start: Instant::now(),
            bytes: 0,
        }
    }

    pub(crate) fn consume(&mut self, count: usize) {
        let Some(limit) = self.limit.filter(|&limit| limit > 0) else {
            return;
        };
        self.bytes = self.bytes.saturating_add(count as u64);
        let target = Duration::from_secs_f64(self.bytes as f64 / limit as f64);
        let elapsed = self.start.elapsed();
        if !elapsed.is_zero() && target > elapsed {
            std::thread::sleep(target - elapsed);
        }
    }
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            retries: 3,
            min_filesize: None,
            max_filesize: None,
            rate_limit: None,
            update_time: false,
            part_files: PartFiles::Private,
        }
    }
}

struct AttemptFailure {
    error: Error,
    retryable: bool,
    retry_after: std::time::Duration,
}

impl From<Error> for AttemptFailure {
    fn from(error: Error) -> Self {
        Self {
            error,
            retryable: false,
            retry_after: std::time::Duration::ZERO,
        }
    }
}

impl From<std::io::Error> for AttemptFailure {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error).into()
    }
}

impl AttemptFailure {
    fn retry(error: Error) -> Self {
        Self {
            error,
            retryable: true,
            retry_after: std::time::Duration::ZERO,
        }
    }

    fn request(error: Error) -> Self {
        let delay = crate::retry::failure_delay(&error);
        Self {
            error,
            retryable: delay.is_some(),
            retry_after: delay.unwrap_or_default(),
        }
    }
}

// Only strong ETags identify byte-for-byte representations. Last-Modified alone
// cannot reliably guard appends when a resource changes within one second.
struct ResumeState {
    etag: reqwest::header::HeaderValue,
    url: url::Url,
    total: u64,
}

fn identity_encoded(response: &Response) -> bool {
    let values = response.headers().get_all(CONTENT_ENCODING);
    values.iter().all(|value| value == "identity")
}

fn resume_state(response: &Response) -> Option<ResumeState> {
    if response.status() != reqwest::StatusCode::OK || !identity_encoded(response) {
        return None;
    }
    let etag = response.headers().get(ETAG)?;
    let bytes = etag.as_bytes();
    // RFC entity-tag: DQUOTE *etagc DQUOTE, without the weak W/ prefix.
    if bytes.len() < 2
        || bytes[0] != b'"'
        || bytes[bytes.len() - 1] != b'"'
        || !bytes[1..bytes.len() - 1]
            .iter()
            .all(|&byte| byte == 0x21 || (0x23..=0x7e).contains(&byte) || byte >= 0x80)
        || response.headers().get_all(ETAG).iter().count() != 1
    {
        return None;
    }
    Some(ResumeState {
        etag: etag.clone(),
        url: response.url().clone(),
        total: response.content_length()?,
    })
}

/// `parse_http_range(Content-Range)`: the start and the total, if known.
fn content_range(response: &Response) -> Option<(u64, Option<u64>)> {
    let range = response.headers().get(CONTENT_RANGE)?.to_str().ok()?;
    let (span, total) = range.strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = span.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let total = if total.trim() == "*" {
        None
    } else {
        Some(total.trim().parse().ok()?)
    };
    Some((start, total))
}

fn validate_range(response: &Response, state: &ResumeState, offset: u64) -> Result<()> {
    let invalid = || Error::InvalidRangeResponse;
    let range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(invalid)?;
    let (span, total) = range
        .strip_prefix("bytes ")
        .and_then(|value| value.split_once('/'))
        .ok_or_else(invalid)?;
    let (start, end) = span.split_once('-').ok_or_else(invalid)?;
    let number = |value: &str| -> Option<u64> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        value.parse().ok()
    };
    if number(start) != Some(offset)
        || number(end) != state.total.checked_sub(1)
        || number(total) != Some(state.total)
        || response
            .content_length()
            .is_some_and(|length| length != state.total - offset)
        || response.headers().get(ETAG) != Some(&state.etag)
        || response.headers().get_all(ETAG).iter().count() != 1
        || response.headers().get_all(CONTENT_RANGE).iter().count() != 1
        || response.url() != &state.url
        || !identity_encoded(response)
    {
        return Err(invalid());
    }
    Ok(())
}

/// Stream to a same-directory temporary file, then publish without overwriting.
/// Interrupted bodies resume only with a strong ETag and known total length.
/// Partial files are private to this invocation and removed on failure.
pub fn download(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    download_resource(
        client,
        &HttpResource {
            url: &info.url,
            referrer: (!info.direct).then_some(&info.webpage_url),
            validate_media: !info.direct,
            subtitle: false,
            limits: (options.min_filesize, options.max_filesize),
            rate_limit: options.rate_limit,
            update_time: options.update_time,
            part_files: options.part_files,
        },
        destination,
        options,
        callback,
    )
}

pub(crate) fn download_subtitle(
    client: &Client,
    info: &VideoInfo,
    subtitle: &crate::SubtitleFormat,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    if !matches!(subtitle.url.scheme(), "http" | "https") {
        return Err(Error::UnsupportedScheme(subtitle.url.scheme().into()));
    }
    download_resource(
        client,
        &HttpResource {
            url: &subtitle.url,
            referrer: Some(&info.webpage_url),
            validate_media: false,
            subtitle: true,
            limits: (None, None),
            rate_limit: None,
            update_time: options.update_time,
            part_files: PartFiles::Private,
        },
        destination,
        options,
        callback,
    )
}

/// Copy a thumbnail with the whole-file transport (no page referrer, no media
/// MIME check), publishing without overwriting.
pub(crate) fn download_thumbnail(
    client: &Client,
    url: &url::Url,
    destination: &Path,
    options: DownloadOptions,
) -> Result<u64> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::UnsupportedScheme(url.scheme().into()));
    }
    download_resource(
        client,
        &HttpResource {
            url,
            referrer: None,
            validate_media: false,
            subtitle: false,
            limits: (None, None),
            rate_limit: None,
            // Upstream writes thumbnails without the downloader's utime.
            update_time: false,
            part_files: PartFiles::Private,
        },
        destination,
        options,
        &mut |_| ControlFlow::Continue(()),
    )
}

struct HttpResource<'a> {
    url: &'a url::Url,
    referrer: Option<&'a url::Url>,
    validate_media: bool,
    subtitle: bool,
    /// Minimum/maximum declared total length in bytes.
    limits: (Option<u64>, Option<u64>),
    rate_limit: Option<u64>,
    update_time: bool,
    part_files: PartFiles,
}

/// The file a transfer is written to.
enum Target {
    Private(tempfile::NamedTempFile),
    Part { file: fs::File, path: PathBuf },
}

impl Target {
    fn file(&mut self) -> &mut fs::File {
        match self {
            Target::Private(temporary) => temporary.as_file_mut(),
            Target::Part { file, .. } => file,
        }
    }

    fn publish(self, destination: &Path) -> Result<()> {
        match self {
            Target::Private(temporary) => {
                temporary.persist_noclobber(destination).map_err(|e| {
                    if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                        Error::OutputExists(destination.into())
                    } else {
                        Error::Io(e.error)
                    }
                })?;
            }
            Target::Part { path, .. } if path != destination => {
                if fs::symlink_metadata(destination).is_ok() {
                    return Err(Error::OutputExists(destination.into()));
                }
                fs::rename(&path, destination)?;
            }
            Target::Part { .. } => {}
        }
        Ok(())
    }
}

fn download_resource(
    client: &Client,
    resource: &HttpResource<'_>,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    if destination.exists() {
        return Err(Error::OutputExists(destination.into()));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut actual = 0;
    let mut temporary = match resource.part_files {
        PartFiles::Private => Target::Private(
            tempfile::Builder::new()
                .prefix(".yt-dlp-rs-")
                .suffix(".part")
                .tempfile_in(parent)?,
        ),
        mode => {
            let path = if mode == PartFiles::NoPart {
                destination.to_owned()
            } else {
                let mut name = destination.as_os_str().to_owned();
                name.push(".part");
                PathBuf::from(name)
            };
            // Upstream `continuedl`: continue an existing regular part file.
            if mode == (PartFiles::Part { resume: true })
                && let Ok(metadata) = fs::metadata(&path)
                && metadata.is_file()
            {
                actual = metadata.len();
            }
            let mut file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(actual == 0)
                .open(&path)?;
            file.seek(SeekFrom::End(0))?;
            Target::Part { file, path }
        }
    };
    // A continued part file is first requested with a plain upstream Range.
    let mut continued = actual > 0;
    let mut state = None;
    let mut last_modified = None;
    let mut reporter = Reporter {
        callback,
        attempt: 1,
        total: None,
    };
    for attempt in 0..=options.retries {
        reporter.attempt = u16::from(attempt) + 1;
        reporter.report(DownloadPhase::Requesting, actual)?;
        match download_attempt(
            client,
            resource,
            temporary.file(),
            &mut state,
            &mut actual,
            &mut continued,
            &mut last_modified,
            &mut reporter,
        ) {
            Ok(()) => {
                temporary.file().flush()?;
                temporary.file().sync_all()?;
                if resource.update_time {
                    crate::filetime::apply(
                        temporary.file(),
                        crate::filetime::filetime(last_modified.as_deref()),
                    );
                }
                reporter.report(DownloadPhase::Publishing, actual)?;
                temporary.publish(destination)?;
                return Ok(actual);
            }
            Err(failure) if failure.retryable && attempt < options.retries => {
                reporter.wait(crate::retry::delay(attempt, failure.retry_after), actual)?;
            }
            Err(failure) => return Err(failure.error),
        }
    }
    unreachable!("the final attempt returns success or an error")
}

#[allow(clippy::too_many_arguments)]
fn download_attempt(
    client: &Client,
    resource: &HttpResource<'_>,
    temporary: &mut fs::File,
    state: &mut Option<ResumeState>,
    actual: &mut u64,
    continued: &mut bool,
    last_modified: &mut Option<String>,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<(), AttemptFailure> {
    let resume = state
        .as_ref()
        .filter(|state| *actual > 0 && *actual < state.total);
    let loose = std::mem::take(continued) && *actual > 0;
    let build = |range: bool| {
        let mut request = client
            .get(resource.url.clone())
            .header(ACCEPT_ENCODING, "identity");
        if let Some(state) = resume {
            request = request
                .header(RANGE, format!("bytes={actual}-"))
                .header(IF_RANGE, state.etag.clone());
        } else if range {
            request = request.header(RANGE, format!("bytes={actual}-"));
        }
        if let Some(referrer) = resource.referrer {
            request = request.header(REFERER, referrer.as_str());
        }
        request
    };
    if loose {
        crate::screen::screen(&format!("[download] Resuming download at byte {actual}"));
    }
    // Upstream's 416 branch re-sends the same Range, so a complete part
    // file fails with that status; the part file is kept.
    let mut response = build(loose)
        .send()
        .map_err(Error::Http)
        .and_then(crate::retry::check_response)
        .map_err(AttemptFailure::request)?;
    *last_modified = crate::filetime::header(&response);
    if resource.subtitle
        && !response
            .headers()
            .get_all(CONTENT_ENCODING)
            .iter()
            .all(|value| value.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(Error::Subtitle("compressed subtitle responses are unsupported".into()).into());
    }
    let expected = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT && loose {
        // Upstream accepts a Content-Range starting at the part file's end.
        let (start, total) = content_range(&response).ok_or(Error::InvalidRangeResponse)?;
        if start != *actual {
            return Err(Error::InvalidRangeResponse.into());
        }
        total
    } else if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        let resume = resume.ok_or(Error::PartialResponse)?;
        validate_range(&response, resume, *actual)?;
        Some(resume.total)
    } else {
        if loose {
            crate::screen::screen("[download] Unable to resume");
        }
        // A server ignoring Range (or returning a changed representation for
        // If-Range) sends a full response. Never append it to the old bytes.
        temporary.set_len(0)?;
        temporary.seek(SeekFrom::Start(0))?;
        *actual = 0;
        *state = resume_state(&response);
        response.content_length()
    };
    if resource.validate_media
        && crate::extractor::direct_media_info(&response, resource.url).is_none()
    {
        return Err(Error::UnsupportedHtml5(
            "source no longer serves supported direct HTTP media".into(),
        )
        .into());
    }
    if let Some(total) = expected {
        match resource.limits {
            (Some(min), _) if total < min => {
                return Err(Error::FileSizeLimit(format!(
                    "File is smaller than min-filesize ({total} bytes < {min} bytes). Aborting."
                ))
                .into());
            }
            (_, Some(max)) if total > max => {
                return Err(Error::FileSizeLimit(format!(
                    "File is larger than max-filesize ({total} bytes > {max} bytes). Aborting."
                ))
                .into());
            }
            _ => {}
        }
    }
    reporter.total = expected;
    reporter.report(DownloadPhase::Downloading, *actual)?;
    let mut limiter = RateLimit::new(resource.rate_limit);
    let mut buffer = [0; 64 * 1024];
    loop {
        // Separate network reads from filesystem writes: only the former retry.
        let count = match response.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(AttemptFailure::retry(Error::Io(error))),
        };
        let next = actual
            .checked_add(count as u64)
            .ok_or(Error::InvalidRangeResponse)?;
        if expected.is_some_and(|expected| next > expected) {
            return Err(Error::InvalidRangeResponse.into());
        }
        temporary.write_all(&buffer[..count])?;
        *actual = next;
        reporter.report(DownloadPhase::Downloading, *actual)?;
        limiter.consume(count);
    }
    if let Some(expected) = expected
        && *actual != expected
    {
        return Err(AttemptFailure::retry(Error::IncompleteDownload {
            expected,
            actual: *actual,
        }));
    }
    Ok(())
}
