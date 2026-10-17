use serde::{Deserialize, Serialize};
use url::Url;

/// Fully extracted results; playlists contain videos, never unresolved URLs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "_type", rename_all = "lowercase")]
pub enum ExtractResult {
    Video(VideoInfo),
    Playlist(PlaylistInfo),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfo {
    pub id: String,
    pub title: String,
    pub webpage_url: Url,
    pub extractor: String,
    pub extractor_key: String,
    pub entries: Vec<VideoInfo>,
}

impl ExtractResult {
    /// Preserve the single-video API's explicit boundary instead of dropping entries.
    pub fn into_video(self) -> crate::Result<VideoInfo> {
        match self {
            Self::Video(info) => Ok(info),
            Self::Playlist(_) => Err(crate::Error::PlaylistRequiresResult),
        }
    }

    pub fn into_entries(self) -> Vec<VideoInfo> {
        match self {
            Self::Video(info) => vec![info],
            Self::Playlist(info) => info.entries,
        }
    }

    pub(crate) fn select_format(&mut self, requested: Option<&str>) -> crate::Result<()> {
        match self {
            Self::Video(info) => info.select_format(requested),
            Self::Playlist(info) => {
                for entry in &mut info.entries {
                    entry
                        .select_format(requested)
                        .map_err(|error| match error {
                            crate::Error::FormatSelection(message) => {
                                crate::Error::FormatSelection(format!("{}: {message}", entry.id))
                            }
                            other => other,
                        })?;
                }
                Ok(())
            }
        }
    }
}

/// A probed progressive HTTP source. IDs are zero-based HTML declaration indices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFormat {
    pub format_id: String,
    pub url: Url,
    pub ext: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<VideoFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

impl VideoInfo {
    /// Select a literal ID, or rank progressive sources for default/best/worst.
    /// Resolution is the smaller declared dimension (or the only known one).
    /// Unknown resolution sorts below known resolution; ties use declaration order.
    /// This is a bounded native policy, not upstream's full format sorter.
    pub fn select_format(&mut self, requested: Option<&str>) -> crate::Result<()> {
        if self.formats.is_empty() {
            return match requested {
                None | Some("best" | "worst") => Ok(()),
                Some(id) => Err(crate::Error::FormatSelection(format!(
                    "format {id:?} is unavailable; this result exposes no formats"
                ))),
            };
        }
        let available = self
            .formats
            .iter()
            .map(|f| f.format_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let resolution = |format: &VideoFormat| match (format.width, format.height) {
            (Some(width), Some(height)) => Some(width.min(height)),
            (width, height) => width.or(height),
        };
        let selected = match requested {
            None | Some("best") => self.formats.iter().max_by_key(|f| resolution(f)),
            Some("worst") => self.formats.iter().min_by_key(|f| resolution(f)),
            Some(id) => self.formats.iter().find(|f| f.format_id == id),
        }
        .ok_or_else(|| {
            crate::Error::FormatSelection(format!(
                "requested format is unavailable; available IDs: {available}"
            ))
        })?;
        self.url = selected.url.clone();
        self.ext = selected.ext.clone();
        self.filesize = selected.filesize;
        self.width = selected.width;
        self.height = selected.height;
        self.format_id = Some(selected.format_id.clone());
        Ok(())
    }
}
