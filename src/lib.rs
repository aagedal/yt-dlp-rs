mod download;
mod error;
pub mod extractor;
mod info;
pub mod output;

pub use error::{Error, Result};
pub use info::VideoInfo;

use extractor::{DirectHttp, Extractor};
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
            .build()?;
        Ok(Self {
            client,
            extractors: vec![Box::new(DirectHttp)],
        })
    }

    /// Insert a site extractor before the generic fallback.
    pub fn register_extractor(&mut self, extractor: Box<dyn Extractor>) {
        self.extractors.insert(0, extractor);
    }

    pub fn extract_info(&self, input: &str) -> Result<VideoInfo> {
        let url = Url::parse(input)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Error::UnsupportedScheme(url.scheme().into()));
        }
        for extractor in &self.extractors {
            if let Some(info) = extractor.extract(&self.client, &url)? {
                return Ok(info);
            }
        }
        Err(Error::UnsupportedUrl(input.into()))
    }

    pub fn download(&self, info: &VideoInfo, destination: impl AsRef<Path>) -> Result<u64> {
        download::download(&self.client, info, destination.as_ref())
    }
}
