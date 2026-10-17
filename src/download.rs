use crate::{Error, Result, VideoInfo};
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, ETAG, IF_RANGE, RANGE, REFERER},
};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

/// Download retry policy, independent of `ExtractionOptions`.
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
        let retryable = crate::retry::transient_request(&error);
        Self {
            error: Error::Http(error),
            retryable,
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
) -> Result<u64> {
    if destination.exists() {
        return Err(Error::OutputExists(destination.into()));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-")
        .suffix(".part")
        .tempfile_in(parent)?;
    let mut state = None;
    let mut actual = 0;
    for attempt in 0..=options.retries {
        match download_attempt(
            client,
            info,
            temporary.as_file_mut(),
            &mut state,
            &mut actual,
        ) {
            Ok(()) => {
                temporary.flush()?;
                temporary.as_file().sync_all()?;
                temporary.persist_noclobber(destination).map_err(|e| {
                    if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                        Error::OutputExists(destination.into())
                    } else {
                        Error::Io(e.error)
                    }
                })?;
                return Ok(actual);
            }
            Err(failure) if failure.retryable && attempt < options.retries => {
                crate::retry::backoff(attempt);
            }
            Err(failure) => return Err(failure.error),
        }
    }
    unreachable!("the final attempt returns success or an error")
}

fn download_attempt(
    client: &Client,
    info: &VideoInfo,
    temporary: &mut fs::File,
    state: &mut Option<ResumeState>,
    actual: &mut u64,
) -> std::result::Result<(), AttemptFailure> {
    let resume = state
        .as_ref()
        .filter(|state| *actual > 0 && *actual < state.total);
    let mut request = client
        .get(info.url.clone())
        .header(ACCEPT_ENCODING, "identity");
    if let Some(state) = resume {
        request = request
            .header(RANGE, format!("bytes={actual}-"))
            .header(IF_RANGE, state.etag.clone());
    }
    if !info.direct {
        request = request.header(REFERER, info.webpage_url.as_str());
    }
    let mut response = request
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(AttemptFailure::request)?;
    let expected = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        let resume = resume.ok_or(Error::PartialResponse)?;
        validate_range(&response, resume, *actual)?;
        Some(resume.total)
    } else {
        // A server ignoring Range (or returning a changed representation for
        // If-Range) sends a full response. Never append it to the old bytes.
        temporary.set_len(0)?;
        temporary.seek(SeekFrom::Start(0))?;
        *actual = 0;
        *state = resume_state(&response);
        response.content_length()
    };
    if !info.direct && crate::extractor::direct_media_info(&response, &info.url).is_none() {
        return Err(Error::UnsupportedHtml5(
            "source no longer serves supported direct HTTP media".into(),
        )
        .into());
    }
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
