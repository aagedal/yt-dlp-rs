mod dash;
pub(crate) mod direct;
mod generic_urls;
pub(crate) mod hls;
pub(crate) mod html5;
mod json_ld;
mod jwplayer;
mod rss;
mod streamable;
mod youtube;

pub use direct::DirectHttp;
pub(crate) use direct::from_response as direct_media_info;
pub use hls::set_split_discontinuity as hls_split_discontinuity;
pub use html5::GenericHttp;
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
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: Downloading JSON metadata"));
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

/// `self._proto_relative_url(url)`: `//host/...` becomes `https://host/...`.
pub(crate) fn proto_relative(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    }
}
