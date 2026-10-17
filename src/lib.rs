mod download;
mod error;
pub mod extractor;
mod info;
pub mod output;

pub use download::DownloadOptions;
pub use error::{Error, Result};
pub use info::{ExtractResult, PlaylistInfo, VideoFormat, VideoInfo};

use extractor::{Extractor, GenericHttp};
use reqwest::blocking::Client;
use std::{path::Path, time::Duration};
use url::Url;

pub struct YoutubeDl {
    client: Client,
    extractors: Vec<Box<dyn Extractor>>,
}

impl YoutubeDl {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("yt-dlp-rs/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .redirect(reqwest::redirect::Policy::limited(10))
            // Preserve an HTML5 page's explicit referrer across media redirects.
            .referer(false)
            .build()?;
        Ok(Self {
            client,
            extractors: vec![Box::new(GenericHttp)],
        })
    }

    /// Insert a site extractor before the generic fallback.
    pub fn register_extractor(&mut self, extractor: Box<dyn Extractor>) {
        self.extractors.insert(0, extractor);
    }

    pub fn extract_info(&self, input: &str) -> Result<VideoInfo> {
        self.extract_info_with_format(input, None)
    }

    /// Extract and select a literal format ID; no selector expressions are supported.
    pub fn extract_info_with_format(&self, input: &str, format: Option<&str>) -> Result<VideoInfo> {
        self.extract_result_with_format(input, format)?.into_video()
    }

    /// Extract a video or an HTML5 playlist, resolving formats for every entry.
    pub fn extract_result(&self, input: &str) -> Result<ExtractResult> {
        self.extract_result_with_format(input, None)
    }

    /// A literal source ID applies independently to every video in a playlist.
    /// Extraction or selection failure in any entry fails the whole result.
    pub fn extract_result_with_format(
        &self,
        input: &str,
        format: Option<&str>,
    ) -> Result<ExtractResult> {
        let url = Url::parse(input)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Error::UnsupportedScheme(url.scheme().into()));
        }
        for extractor in &self.extractors {
            if let Some(mut info) = extractor.extract_result(&self.client, &url)? {
                info.select_format(format)?;
                return Ok(info);
            }
        }
        Err(Error::UnsupportedUrl(input.into()))
    }

    pub fn download(&self, info: &VideoInfo, destination: impl AsRef<Path>) -> Result<u64> {
        self.download_with_options(info, destination, DownloadOptions::default())
    }

    /// Download with bounded retries. Interrupted transfers may resume with a strong ETag and known length.
    pub fn download_with_options(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
    ) -> Result<u64> {
        download::download(&self.client, info, destination.as_ref(), options)
    }
}
