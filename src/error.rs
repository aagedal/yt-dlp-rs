use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("unsupported URL scheme: {0}; expected http or https")]
    UnsupportedScheme(String),
    #[error("no extractor supports {0}; expected direct HTTP media or a HTML5 media page")]
    UnsupportedUrl(String),
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
    #[error("server returned an unsolicited partial response; full media was requested")]
    PartialResponse,
    #[error("server returned an invalid or changed range response")]
    InvalidRangeResponse,
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
