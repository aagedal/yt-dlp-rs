use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Retries were exhausted (upstream `RetryManager.report_retry` text,
    /// without the suffix when no retries were allowed).
    #[error(
        "[download] Got error: {text}{}",
        if *retries == 0 { String::new() } else { format!(". Giving up after {retries} retries") }
    )]
    RetriesExhausted {
        text: String,
        retries: u32,
        source: Box<Error>,
    },
    /// A fatal fragment gave up (upstream reports the retry failure, then
    /// `fragment N not found, unable to continue`).
    #[error("{source}")]
    FragmentUnavailable { index: usize, source: Box<Error> },
    /// Upstream `CookieLoadError`: loading the cookie jar failed; it ends the
    /// run.
    #[error("{0}")]
    CookieLoad(String),
    /// An `--exec` command rejected like upstream `UnsafeExecExpansionError`.
    #[error("{0}")]
    Exec(String),
    /// Upstream `_request_webpage`'s `ExtractorError` for the generic page:
    /// `[generic] ID: Unable to download webpage: ERR (caused by REPR)`.
    #[error("{}", crate::extractor::html5::webpage_error_text(id, source))]
    Webpage {
        id: String,
        #[source]
        source: Box<Error>,
    },
    /// An extractor request's `ExtractorError`:
    /// `[EXTRACTOR] ID: Unable to download WHAT: ERR (caused by REPR)`.
    #[error(
        "{}",
        crate::extractor::request_error_text(extractor, id, what, source)
    )]
    ExtractorRequest {
        extractor: &'static str,
        id: String,
        what: &'static str,
        #[source]
        source: Box<Error>,
    },
    /// Upstream raises `DownloadError(MESSAGE)` without reporting it first:
    /// `--load-info-json` then retries the webpage URL, otherwise it is
    /// reported as `ERROR: MESSAGE`.
    #[error("{0}")]
    Raised(String),
    /// An error upstream reports as plain `ERROR: MESSAGE` (`report_error`
    /// in a downloader), without the URL/ID prefix.
    #[error("{0}")]
    Reported(String),
    #[error("invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("unsupported URL scheme: {0}; expected http or https")]
    UnsupportedScheme(String),
    /// GenericIE's `UnsupportedError` (no supported media on the page).
    #[error("Unsupported URL: {0}")]
    UnsupportedUrl(String),
    #[error("unsupported YouTube behavior: {0}")]
    UnsupportedYoutube(String),
    #[error("unsupported manifest: {0}")]
    UnsupportedManifest(String),
    #[error("subtitle operation failed: {0}")]
    Subtitle(String),
    #[error("unsupported HTML5 media: {0}")]
    UnsupportedHtml5(String),
    #[error("playlist result requires extract_result or extract_result_with_format")]
    PlaylistRequiresResult,
    #[error("playlist selection failed: {0}")]
    PlaylistSelection(String),
    #[error("format selection failed: {0}")]
    FormatSelection(String),
    #[error("HTML page exceeds the {0}-byte extraction limit")]
    HtmlTooLarge(u64),
    #[error("cookie operation failed: {0}")]
    Cookies(String),
    #[error("invalid HTTP configuration: {0}")]
    HttpConfiguration(String),
    #[error("HTTP request failed: {0}")]
    Http(HttpError),
    /// Upstream's `HTTPError` for a redirect urllib refuses to follow: a
    /// loop (more than 10 URLs, or one URL 4 times) or another scheme.
    /// Handled wherever a status error is.
    #[error(
        "HTTP Error {status}: {reason}{}",
        if *redirect_loop { " (redirect loop detected)" } else { "" }
    )]
    HttpStatus {
        status: u16,
        reason: String,
        redirect_loop: bool,
    },
    /// A transient HTTP status with a supported, bounded Retry-After hint.
    #[error("HTTP request failed: {source}")]
    HttpRetryAfter {
        #[source]
        source: HttpError,
        retry_after: std::time::Duration,
    },
    /// A transport failure of an impersonated request (upstream's
    /// `TransportError` from `curl_cffi`).
    #[error("HTTP request failed: {0}")]
    Impersonate(String),
    /// A media transfer through the impersonating transport failed before
    /// its response (upstream's `TransportError`, retried like one).
    #[error("{0}")]
    MediaTransport(String),
    #[error("extraction response body failed: {0}")]
    ExtractionRead(#[source] std::io::Error),
    #[error("server returned an unsolicited partial response; full media was requested")]
    PartialResponse,
    #[error("server returned an invalid or changed range response")]
    InvalidRangeResponse,
    #[error("download cancelled")]
    DownloadCancelled,
    /// A library `Interrupt::stop`: the data downloaded so far was published
    /// at `path` (whole fragments only).
    #[error("download stopped; partial file kept at {}", .path.display())]
    DownloadStopped { path: std::path::PathBuf },
    /// A library `Interrupt` ended an extraction before its next request.
    #[error("extraction cancelled")]
    ExtractionCancelled,
    /// A `--min-filesize`/`--max-filesize` limit skipped the transfer; upstream
    /// reports this without an error or archive record.
    #[error("{0}")]
    FileSizeLimit(String),
    #[error("postprocessing failed: {0}")]
    Postprocess(String),
    #[error("match filter failed: {0}")]
    MatchFilter(String),
    #[error("info JSON failed: {0}")]
    InfoJson(String),
    #[error("download archive failed: {0}")]
    Archive(String),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("output already exists: {0}")]
    OutputExists(PathBuf),
    #[error("invalid output template: {0}")]
    InvalidTemplate(String),
    #[error("incomplete download: expected {expected} bytes, received {actual}")]
    IncompleteDownload { expected: u64, actual: u64 },
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// [`Error::Http`] for an HTTP client error.
    pub(crate) fn http(error: reqwest::Error) -> Self {
        Self::Http(HttpError(error))
    }
}

/// An HTTP transport or status error ([`Error::Http`],
/// [`Error::HttpRetryAfter`]). Its `Display` text and `source` chain are
/// the HTTP client's; the client's own error type stays private, so
/// updating the client is not a breaking change.
#[derive(Debug)]
pub struct HttpError(reqwest::Error);

impl HttpError {
    pub(crate) fn new(error: reqwest::Error) -> Self {
        Self(error)
    }

    /// The HTTP client's error.
    pub(crate) fn inner(&self) -> &reqwest::Error {
        &self.0
    }

    /// The response status of a status error.
    pub fn status(&self) -> Option<u16> {
        self.0.status().map(|status| status.as_u16())
    }

    /// The URL of the failed request, when known.
    pub fn url(&self) -> Option<&url::Url> {
        self.0.url()
    }

    /// Whether the request timed out.
    pub fn is_timeout(&self) -> bool {
        self.0.is_timeout()
    }

    /// Whether connecting to the server failed.
    pub fn is_connect(&self) -> bool {
        self.0.is_connect()
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for HttpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}
