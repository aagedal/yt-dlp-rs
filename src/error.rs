use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
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
    #[error("unsupported HTML5 media: {0}")]
    UnsupportedHtml5(String),
    #[error("playlist result requires extract_result or extract_result_with_format")]
    PlaylistRequiresResult,
    #[error("format selection failed: {0}")]
    FormatSelection(String),
    #[error("HTML page exceeds the {0}-byte extraction limit")]
    HtmlTooLarge(u64),
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
    #[error("postprocessing failed: {0}")]
    Postprocess(String),
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
