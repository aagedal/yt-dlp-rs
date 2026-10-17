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
}

impl VideoInfo {
    /// Resolve a literal format ID. Multiple sources require an explicit choice;
    /// this port does not yet implement upstream's automatic format ranking.
    pub fn select_format(&mut self, requested: Option<&str>) -> crate::Result<()> {
        if self.formats.is_empty() {
            return match requested {
                None => Ok(()),
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
        let selected = match requested {
            Some(id) => self.formats.iter().find(|f| f.format_id == id),
            None if self.formats.len() == 1 => self.formats.first(),
            None => {
                return Err(crate::Error::FormatSelection(format!(
                    "multiple sources require --format ID; available IDs: {available}"
                )));
            }
        }
        .ok_or_else(|| {
            crate::Error::FormatSelection(format!(
                "requested format is unavailable; available IDs: {available}"
            ))
        })?;
        self.url = selected.url.clone();
        self.ext = selected.ext.clone();
        self.filesize = selected.filesize;
        self.format_id = Some(selected.format_id.clone());
        Ok(())
    }
}
