use crate::{Error, Result, VideoInfo};
use reqwest::{
    blocking::Client,
    header::{ACCEPT_ENCODING, REFERER},
};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

/// Whole-file download retry policy. Extraction requests are not retried.
#[derive(Debug, Clone, Copy)]
pub struct DownloadOptions {
    /// Additional attempts after a transient failure, from zero to 255.
    pub retries: u8,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self { retries: 3 }
    }
}

struct AttemptFailure {
    error: Error,
    retryable: bool,
}

impl From<Error> for AttemptFailure {
    fn from(error: Error) -> Self {
        Self {
            error,
            retryable: false,
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
        }
    }

    fn request(error: reqwest::Error) -> Self {
        let retryable = error.is_connect()
            || error.is_timeout()
            || error
                .status()
                .is_some_and(|status| matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504));
        Self {
            error: Error::Http(error),
            retryable,
        }
    }
}

/// Stream to a same-directory temporary file, then publish without overwriting.
/// Each retry discards the failed attempt's file and starts from byte zero.
pub fn download(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
) -> Result<u64> {
    for attempt in 0..=options.retries {
        match download_attempt(client, info, destination) {
            Ok(bytes) => return Ok(bytes),
            Err(failure) if failure.retryable && attempt < options.retries => {
                // Bounded exponential backoff: 100, 200, 400, 800, then 1000 ms.
                let delay_ms = (100_u64 << attempt.min(4)).min(1000);
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
            Err(failure) => return Err(failure.error),
        }
    }
    unreachable!("the final attempt returns success or an error")
}

fn download_attempt(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
) -> std::result::Result<u64, AttemptFailure> {
    if destination.exists() {
        return Err(Error::OutputExists(destination.into()).into());
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut request = client
        .get(info.url.clone())
        .header(ACCEPT_ENCODING, "identity");
    if !info.direct {
        request = request.header(REFERER, info.webpage_url.as_str());
    }
    let mut response = request
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(AttemptFailure::request)?;
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(Error::PartialResponse.into());
    }
    if !info.direct && crate::extractor::direct_media_info(&response, &info.url).is_none() {
        return Err(Error::UnsupportedHtml5(
            "source no longer serves supported direct HTTP media".into(),
        )
        .into());
    }
    let expected = response.content_length();
    let mut temporary = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-")
        .suffix(".part")
        .tempfile_in(parent)?;
    let mut actual = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        // Separate network reads from filesystem writes: only the former retry.
        let count = match response.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(AttemptFailure::retry(Error::Io(error))),
        };
        temporary.write_all(&buffer[..count])?;
        actual += count as u64;
    }
    if let Some(expected) = expected
        && actual != expected
    {
        return Err(AttemptFailure::retry(Error::IncompleteDownload {
            expected,
            actual,
        }));
    }
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(destination).map_err(|e| {
        if e.error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::OutputExists(destination.into())
        } else {
            Error::Io(e.error)
        }
    })?;
    Ok(actual)
}
