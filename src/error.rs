use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
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
    /// An `--exec` command rejected like upstream `UnsafeExecExpansionError`.
    #[error("{0}")]
    Exec(String),
    /// An error upstream reports as plain `ERROR: MESSAGE` (`report_error`
    /// in a downloader), without the URL/ID prefix.
    #[error("{0}")]
    Reported(String),
    #[error("invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("unsupported URL scheme: {0}; expected http or https")]
    UnsupportedScheme(String),
    #[error(
        "no extractor supports {0}; expected direct HTTP media, a HTML5 media page, or supported HLS/DASH VOD"
    )]
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
    Http(#[from] reqwest::Error),
    /// A transient HTTP status with a supported, bounded Retry-After hint.
    #[error("HTTP request failed: {source}")]
    HttpRetryAfter {
        #[source]
        source: reqwest::Error,
        retry_after: std::time::Duration,
    },
    #[error("extraction response body failed: {0}")]
    ExtractionRead(#[source] std::io::Error),
    #[error("server returned an unsolicited partial response; full media was requested")]
    PartialResponse,
    #[error("server returned an invalid or changed range response")]
    InvalidRangeResponse,
    #[error("download cancelled")]
    DownloadCancelled,
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
