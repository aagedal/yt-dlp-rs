mod dash;
pub(crate) mod direct;
mod ebaumsworld;
mod generic_urls;
mod historicfilms;
pub(crate) mod hls;
pub(crate) mod html5;
mod hypem;
mod json_ld;
mod jwplayer;
mod livestreamfails;
mod megaphone;
mod reverbnation;
mod rss;
mod streamable;
mod youtube;

pub use direct::DirectHttp;
pub(crate) use direct::from_response as direct_media_info;
pub use ebaumsworld::{EbaumsWorld, suitable as ebaumsworld_suitable};
pub use historicfilms::{HistoricFilms, suitable as historicfilms_suitable};
pub use hls::set_split_discontinuity as hls_split_discontinuity;
pub use html5::GenericHttp;
pub use hypem::{Hypem, suitable as hypem_suitable};
pub use livestreamfails::{Livestreamfails, suitable as livestreamfails_suitable};
pub use megaphone::{Megaphone, suitable as megaphone_suitable};
pub use reverbnation::{ReverbNation, suitable as reverbnation_suitable};
pub use streamable::{Streamable, suitable as streamable_suitable};
pub use youtube::{Youtube, YoutubeOptions, suitable as youtube_suitable};

use crate::{ExtractResult, Result, VideoInfo};
use reqwest::blocking::Client;
use url::Url;

/// Extractors return None when they do not recognize a URL/response.
/// A result may expose candidate formats; YoutubeDl resolves selection before returning it.
/// Register site extractors before the generic direct-media fallback.
pub trait Extractor: Send + Sync {
    fn key(&self) -> &'static str;
    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>>;

    /// Override to support playlists. Existing video-only extractors need no changes.
    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

/// `[EXTRACTOR] ID: Unable to download WHAT: ERR (caused by REPR)`.
pub(crate) fn request_error_text(
    extractor: &str,
    id: &str,
    what: &str,
    error: &crate::Error,
) -> String {
    let text = crate::upstream_error_text(error);
    let status = match error {
        crate::Error::Http(source) | crate::Error::HttpRetryAfter { source, .. } => source.status(),
        _ => None,
    };
    let cause = match status {
        Some(_) => format!("<{}>", text.replacen("HTTP Error ", "HTTPError ", 1)),
        None => format!("TransportError({})", crate::batch::py_repr(&text)),
    };
    format!("[{extractor}] {id}: Unable to download {what}: {text} (caused by {cause})")
}

/// Upstream `_download_json(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// JSON metadata`, the request, and the parsed body.
pub(crate) fn download_json(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<serde_json::Value> {
    download_json_note(client, url, extractor, id, "Downloading JSON metadata")
}

/// [`download_json`] with its `note=` (the error note stays the default).
pub(crate) fn download_json_note(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
) -> Result<serde_json::Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request(client, url, None).map_err(wrap)?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    serde_json::from_slice(&body).map_err(|error| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// Upstream `_download_webpage(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// webpage`, the request (`--dump-pages`/`--write-pages` like other
/// pages), and the body as UTF-8.
pub(crate) fn download_webpage(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<String> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: Downloading webpage"));
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "webpage",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request(client, url, None).map_err(wrap)?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Upstream `_download_xml(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// XML`, the request, and the body. Parse errors are this port's text
/// (upstream reports expat's).
pub(crate) fn download_xml(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<String> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: Downloading XML"));
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "XML",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request(client, url, None).map_err(wrap)?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    let text = String::from_utf8(body).map_err(|_| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse XML (not UTF-8)"
        ))
    })?;
    roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse XML ({error}); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    Ok(text)
}

/// A non-fatal `_search_regex` miss: `[EXTRACTOR] unable to extract NAME`.
pub(crate) fn search_warning(extractor: &str, name: &str) {
    crate::screen::warning(&format!(
        "[{extractor}] unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ));
}

/// `self._proto_relative_url(url)`: `//host/...` becomes `https://host/...`.
pub(crate) fn proto_relative(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    }
}
