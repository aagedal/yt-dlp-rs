//! Sequential, all-or-error assembly of saved, finite fragment plans.
use crate::{DownloadOptions, DownloadPhase, DownloadProgress, Error, Fragment, Result, VideoInfo};
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, CONTENT_TYPE, RANGE, REFERER},
};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    ops::ControlFlow,
    path::Path,
    time::{Duration, Instant},
};

struct Reporter<'a> {
    callback: &'a mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
    attempt: u16,
    index: u32,
    count: u32,
}
impl Reporter<'_> {
    fn report(&mut self, phase: DownloadPhase, bytes: u64) -> Result<()> {
        if (self.callback)(DownloadProgress {
            attempt: self.attempt,
            downloaded_bytes: bytes,
            total_bytes: None,
            phase,
            stream_index: None,
            stream_count: None,
            fragment_index: Some(self.index),
            fragment_count: Some(self.count),
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
        error,
        retry: Some(Duration::ZERO),
    }
}
fn unsupported(message: &str) -> Error {
    Error::UnsupportedManifest(message.into())
}

pub(crate) fn download(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    if destination.exists() {
        return Err(Error::OutputExists(destination.into()));
    }
    if info.fragments.is_empty() || info.fragments.len() > 10_000 {
        return Err(unsupported("fragment count must be between 1 and 10000"));
    }
    for fragment in &info.fragments {
        if !matches!(fragment.url.scheme(), "http" | "https") {
            return Err(Error::UnsupportedScheme(fragment.url.scheme().into()));
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
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-fragments-")
        .suffix(".part")
        .tempfile_in(parent)?;
    let mut stored = 0_u64;
    let mut reporter = Reporter {
        callback,
        attempt: 1,
        index: 1,
        count: info.fragments.len() as u32,
    };
    for (index, fragment) in info.fragments.iter().enumerate() {
        reporter.index = index as u32 + 1;
        let prefix = stored;
        for attempt in 0..=options.retries {
            temporary.as_file_mut().set_len(prefix)?;
            temporary.as_file_mut().seek(SeekFrom::Start(prefix))?;
            stored = prefix;
            reporter.attempt = u16::from(attempt) + 1;
            reporter.report(DownloadPhase::Requesting, stored)?;
            match fetch(
                client,
                info,
                fragment,
                temporary.as_file_mut(),
                &mut stored,
                &mut reporter,
            ) {
                Ok(()) => break,
                Err(Failure {
                    error: _,
                    retry: Some(delay),
                }) if attempt < options.retries => {
                    // Discard every byte from the failed fragment, including bytes
                    // reported to observers, before waiting or starting another GET.
                    temporary.as_file_mut().set_len(prefix)?;
                    temporary.as_file_mut().seek(SeekFrom::Start(prefix))?;
                    stored = prefix;
                    reporter.wait(crate::retry::delay(attempt, delay), stored)?;
                }
                Err(failure) => return Err(failure.error),
            }
        }
    }
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    reporter.report(DownloadPhase::Publishing, stored)?;
    temporary.persist_noclobber(destination).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::OutputExists(destination.into())
        } else {
            Error::Io(error.error)
        }
    })?;
    Ok(stored)
}
fn fetch(
    client: &Client,
    info: &VideoInfo,
    fragment: &Fragment,
    file: &mut fs::File,
    stored: &mut u64,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<(), Failure> {
    let mut request = client
        .get(fragment.url.clone())
        .header(ACCEPT_ENCODING, "identity");
    if !info.direct {
        request = request.header(REFERER, info.webpage_url.as_str());
    }
    if let Some(range) = &fragment.byte_range {
        request = request.header(
            RANGE,
            format!("bytes={}-{}", range.offset, range.offset + range.length - 1),
        );
    }
    let mut response = request
        .send()
        .map_err(Error::Http)
        .and_then(crate::retry::check_response)
        .map_err(|error| Failure {
            retry: crate::retry::failure_delay(&error),
            error,
        })?;
    let expected = validate(&response, fragment)?;
    reporter.report(DownloadPhase::Downloading, *stored)?;
    let mut received = 0_u64;
    // Buffer enough leading bytes to recognize split HTML/manifest responses
    // even when a server omits Content-Type or sends one byte per network read.
    let mut prefix = Vec::new();
    response
        .by_ref()
        .take(512)
        .read_to_end(&mut prefix)
        .map_err(|error| body_failure(Error::Io(error)))?;
    let text = String::from_utf8_lossy(&prefix)
        .trim_start_matches('\u{feff}')
        .trim_start()
        .to_ascii_lowercase();
    if text.starts_with("<!doctype html")
        || text.starts_with("<html")
        || text.starts_with("#extm3u")
        || text.starts_with("<?xml")
        || text.starts_with("<mpd")
    {
        return Err(unsupported("fragment returned HTML or another manifest").into());
    }
    append(file, &prefix, stored, &mut received, expected, reporter)?;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = match response.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(body_failure(Error::Io(error))),
        };
        append(
            file,
            &buffer[..count],
            stored,
            &mut received,
            expected,
            reporter,
        )?;
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
        return Err(unsupported("empty media fragment").into());
    }
    Ok(())
}
fn append(
    file: &mut fs::File,
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
fn validate(response: &Response, fragment: &Fragment) -> Result<Option<u64>> {
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
        if !(mime.starts_with("video/")
            || mime.starts_with("audio/")
            || matches!(
                mime.as_str(),
                "application/octet-stream" | "application/mp4" | "application/ogg"
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
