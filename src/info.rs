use serde::{Deserialize, Serialize};
use url::Url;

/// Small, explicitly supported subset of yt-dlp's info dictionary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub ext: String,
    pub url: Url,
    pub webpage_url: Url,
    pub extractor: String,
    pub extractor_key: String,
    pub direct: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,
}
