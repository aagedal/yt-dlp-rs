mod direct;

pub use direct::DirectHttp;

use crate::{Result, VideoInfo};
use reqwest::blocking::Client;
use url::Url;

/// Extractors return None when they do not recognize a URL/response.
/// Register site extractors before the generic direct-media fallback.
pub trait Extractor: Send + Sync {
    fn key(&self) -> &'static str;
    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>>;
}
