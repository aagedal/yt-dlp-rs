mod dash;
pub(crate) mod direct;
pub(crate) mod hls;
pub(crate) mod html5;
mod youtube;

pub use direct::DirectHttp;
pub(crate) use direct::from_response as direct_media_info;
pub use hls::set_split_discontinuity as hls_split_discontinuity;
pub use html5::GenericHttp;
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
