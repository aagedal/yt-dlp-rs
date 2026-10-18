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

    pub(crate) fn select_format_with_merge(
        &mut self,
        requested: Option<&str>,
        prefer_merge: bool,
    ) -> crate::Result<()> {
        // Select every entry before publishing any change to a playlist.
        let mut selected = self.clone();
        match &mut selected {
            Self::Video(info) => info.select_format_with_merge(requested, prefer_merge)?,
            Self::Playlist(info) => {
                for entry in &mut info.entries {
                    entry
                        .select_format_with_merge(requested, prefer_merge)
                        .map_err(|error| match error {
                            crate::Error::FormatSelection(message) => {
                                crate::Error::FormatSelection(format!("{}: {message}", entry.id))
                            }
                            other => other,
                        })?;
                }
            }
        }
        *self = selected;
        Ok(())
    }
}

/// Whether a source can play independently or requires a complementary stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    /// A generic or muxed source; its precise composition may be unknown.
    #[default]
    Standalone,
    AudioOnly,
    VideoOnly,
}

impl StreamKind {
    fn is_standalone(&self) -> bool {
        *self == Self::Standalone
    }
}

/// A probed direct HTTP source. IDs are HTML declaration indices or YouTube itags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFormat {
    pub format_id: String,
    #[serde(default, skip_serializing_if = "StreamKind::is_standalone")]
    pub stream_kind: StreamKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abr: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tbr: Option<f64>,
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
    #[serde(default, skip_serializing_if = "StreamKind::is_standalone")]
    pub stream_kind: StreamKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_formats: Vec<VideoFormat>,
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
    /// Select a supported source or explicit pair without implicit merger preference.
    pub fn select_format(&mut self, requested: Option<&str>) -> crate::Result<()> {
        self.select_format_with_merge(requested, false)
    }

    /// Bounded native selector grammar: atoms, two-stream `+`, and `/` fallback.
    /// Only the implicit default considers `prefer_merge`; explicit `best` remains
    /// an independently playable source. Selection failures preserve all fields.
    pub(crate) fn select_format_with_merge(
        &mut self,
        requested: Option<&str>,
        prefer_merge: bool,
    ) -> crate::Result<()> {
        let expression = requested.unwrap_or(if prefer_merge {
            "bestvideo+bestaudio/best"
        } else {
            "best"
        });
        let branches = parse_selector(expression)?;
        if self.formats.is_empty() {
            return if branches
                .iter()
                .any(|branch| branch.len() == 1 && matches!(branch[0], "best" | "worst"))
                && self.stream_kind != StreamKind::VideoOnly
            {
                self.requested_formats.clear();
                Ok(())
            } else {
                Err(selection_error("this result exposes no formats"))
            };
        }
        for branch in branches {
            let Some(first) = self.select_atom(branch[0]) else {
                continue;
            };
            if branch.len() == 1 {
                let first = first.clone();
                self.apply_single(&first);
                return Ok(());
            }
            let Some(second) = self.select_atom(branch[1]) else {
                continue;
            };
            let (video, audio) = match (first.stream_kind, second.stream_kind) {
                (StreamKind::VideoOnly, StreamKind::AudioOnly) => (first, second),
                (StreamKind::AudioOnly, StreamKind::VideoOnly) => (second, first),
                _ => continue,
            };
            if !matches!(video.ext.as_str(), "mp4" | "webm")
                || !matches!(audio.ext.as_str(), "m4a" | "webm")
            {
                continue;
            }
            let ext = match (video.ext.as_str(), audio.ext.as_str()) {
                ("mp4", "m4a") => "mp4",
                ("webm", "webm") => "webm",
                _ => "mkv",
            };
            let selected = vec![first.clone(), second.clone()];
            let (width, height) = (video.width, video.height);
            self.url = selected[0].url.clone();
            self.format_id = Some(format!(
                "{}+{}",
                selected[0].format_id, selected[1].format_id
            ));
            self.ext = ext.into();
            self.filesize = None;
            self.width = width;
            self.height = height;
            self.stream_kind = StreamKind::Standalone;
            self.requested_formats = selected;
            return Ok(());
        }
        let merge_hint = if self
            .formats
            .iter()
            .any(|f| f.stream_kind == StreamKind::VideoOnly)
        {
            "; separate video/audio streams require an explicit video+audio selection and an ffmpeg merger"
        } else {
            ""
        };
        Err(selection_error(format!(
            "requested format is unavailable; available IDs: {}{merge_hint}",
            self.formats
                .iter()
                .map(|f| f.format_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }

    fn apply_single(&mut self, selected: &VideoFormat) {
        self.url = selected.url.clone();
        self.ext = selected.ext.clone();
        self.filesize = selected.filesize;
        self.width = selected.width;
        self.height = selected.height;
        self.stream_kind = selected.stream_kind;
        self.format_id = Some(selected.format_id.clone());
        self.requested_formats.clear();
    }

    fn select_atom(&self, atom: &str) -> Option<&VideoFormat> {
        let (kind, worst) = match atom {
            "best" | "worst" => {
                let kind = if self
                    .formats
                    .iter()
                    .any(|f| f.stream_kind == StreamKind::Standalone)
                {
                    StreamKind::Standalone
                } else if self
                    .formats
                    .iter()
                    .any(|f| f.stream_kind == StreamKind::VideoOnly)
                {
                    return None;
                } else {
                    StreamKind::AudioOnly
                };
                (kind, atom == "worst")
            }
            "bestaudio" | "worstaudio" => (StreamKind::AudioOnly, atom == "worstaudio"),
            "bestvideo" | "worstvideo" => (StreamKind::VideoOnly, atom == "worstvideo"),
            id => return self.formats.iter().find(|f| f.format_id == id),
        };
        let compare = |a: &&VideoFormat, b: &&VideoFormat| match kind {
            StreamKind::Standalone => resolution(a).cmp(&resolution(b)),
            StreamKind::VideoOnly => resolution(a)
                .cmp(&resolution(b))
                .then_with(|| bitrate(a.tbr).total_cmp(&bitrate(b.tbr))),
            StreamKind::AudioOnly => bitrate(a.abr)
                .total_cmp(&bitrate(b.abr))
                .then_with(|| bitrate(a.tbr).total_cmp(&bitrate(b.tbr))),
        };
        let formats = self.formats.iter().filter(|f| f.stream_kind == kind);
        if worst {
            formats.min_by(compare)
        } else {
            formats.max_by(compare)
        }
    }
}

fn resolution(format: &VideoFormat) -> Option<u32> {
    match (format.width, format.height) {
        (Some(width), Some(height)) => Some(width.min(height)),
        (width, height) => width.or(height),
    }
}

fn bitrate(value: Option<f64>) -> f64 {
    value.filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(-1.0)
}

fn selection_error(message: impl Into<String>) -> crate::Error {
    crate::Error::FormatSelection(message.into())
}

fn parse_selector(expression: &str) -> crate::Result<Vec<Vec<&str>>> {
    expression
        .split('/')
        .map(|branch| {
            let atoms: Vec<_> = branch.split('+').map(str::trim).collect();
            if atoms.len() > 2
                || atoms.iter().any(|atom| {
                    atom.is_empty()
                        || !atom
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
                })
            {
                Err(selection_error("unsupported or malformed format selector"))
            } else {
                Ok(atoms)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn source(
        id: &str,
        kind: StreamKind,
        ext: &str,
        height: Option<u32>,
        abr: Option<f64>,
        tbr: Option<f64>,
    ) -> VideoFormat {
        VideoFormat {
            format_id: id.into(),
            stream_kind: kind,
            abr,
            tbr,
            url: Url::parse(&format!("https://example.com/{id}.{ext}")).unwrap(),
            ext: ext.into(),
            filesize: Some(42),
            width: None,
            height,
        }
    }

    fn info(formats: Vec<VideoFormat>) -> VideoInfo {
        VideoInfo {
            id: "test".into(),
            title: "Test".into(),
            ext: "mp4".into(),
            url: Url::parse("https://example.com/original.mp4").unwrap(),
            webpage_url: Url::parse("https://example.com/watch").unwrap(),
            extractor: "test".into(),
            extractor_key: "Test".into(),
            direct: false,
            stream_kind: StreamKind::Standalone,
            requested_formats: Vec::new(),
            formats,
            format_id: None,
            filesize: None,
            width: None,
            height: None,
        }
    }

    fn adaptive() -> VideoInfo {
        info(vec![
            source(
                "v1",
                StreamKind::VideoOnly,
                "mp4",
                Some(720),
                None,
                Some(1000.0),
            ),
            source(
                "v2",
                StreamKind::VideoOnly,
                "mp4",
                Some(1080),
                None,
                Some(2000.0),
            ),
            source(
                "a1",
                StreamKind::AudioOnly,
                "m4a",
                None,
                Some(128.0),
                Some(130.0),
            ),
            source(
                "a2",
                StreamKind::AudioOnly,
                "m4a",
                None,
                Some(192.0),
                Some(194.0),
            ),
        ])
    }

    fn snapshot(info: &VideoInfo) -> Value {
        serde_json::to_value(info).unwrap()
    }

    #[test]
    fn standalone_serialization_and_legacy_deserialization_are_unchanged() {
        let legacy = json!({
            "id":"test", "title":"Test", "ext":"mp4", "url":"https://example.com/original.mp4",
            "webpage_url":"https://example.com/watch", "extractor":"test", "extractor_key":"Test",
            "direct":false,
            "formats":[{"format_id":"0", "url":"https://example.com/0.mp4", "ext":"mp4", "filesize":42}]
        });
        let decoded: VideoInfo = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(decoded.stream_kind, StreamKind::Standalone);
        assert_eq!(decoded.formats[0].stream_kind, StreamKind::Standalone);
        assert!(decoded.requested_formats.is_empty());
        assert_eq!(snapshot(&decoded), legacy);
        let adaptive = adaptive();
        let encoded = snapshot(&adaptive);
        assert_eq!(encoded["formats"][0]["stream_kind"], "video_only");
        assert_eq!(encoded["formats"][2]["stream_kind"], "audio_only");
        let decoded: VideoInfo = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(snapshot(&decoded), encoded);
    }

    #[test]
    fn best_standalone_preserves_resolution_and_declaration_ties() {
        let mut video = info(vec![
            source(
                "unknown",
                StreamKind::Standalone,
                "mp4",
                None,
                None,
                Some(9000.0),
            ),
            source(
                "first",
                StreamKind::Standalone,
                "mp4",
                Some(720),
                None,
                Some(9000.0),
            ),
            source("last", StreamKind::Standalone, "mp4", Some(720), None, None),
            source(
                "silent",
                StreamKind::VideoOnly,
                "mp4",
                Some(2160),
                None,
                None,
            ),
        ]);
        video.select_format(None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("last"));
        video.select_format(Some("worst")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("unknown"));
        video.formats.remove(0);
        video.select_format(Some("worst")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("first"));
    }

    #[test]
    fn audio_ranks_abr_then_total_bitrate_with_declaration_ties() {
        let mut video = adaptive();
        video.formats.push(source(
            "a3",
            StreamKind::AudioOnly,
            "webm",
            None,
            Some(192.0),
            Some(200.0),
        ));
        video.formats.push(source(
            "a4",
            StreamKind::AudioOnly,
            "webm",
            None,
            Some(192.0),
            Some(200.0),
        ));
        video.select_format(Some("bestaudio")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a4"));
        video.select_format(Some("worstaudio")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a1"));
        video
            .formats
            .retain(|f| f.stream_kind == StreamKind::AudioOnly);
        video.select_format(Some("best")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a4"));
        assert_eq!(video.stream_kind, StreamKind::AudioOnly);
    }

    #[test]
    fn video_ranks_smaller_dimension_then_bitrate() {
        let mut video = adaptive();
        let mut portrait = source(
            "portrait",
            StreamKind::VideoOnly,
            "webm",
            Some(1920),
            None,
            Some(4000.0),
        );
        portrait.width = Some(720);
        video.formats.push(portrait);
        video.formats.push(source(
            "tie1",
            StreamKind::VideoOnly,
            "mp4",
            Some(1080),
            None,
            Some(3000.0),
        ));
        video.formats.push(source(
            "tie2",
            StreamKind::VideoOnly,
            "mp4",
            Some(1080),
            None,
            Some(3000.0),
        ));
        video.select_format(Some("bestvideo")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("tie2"));
        video.select_format(Some("worstvideo")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1"));
    }

    #[test]
    fn reversed_merge_preserves_order_and_video_metadata() {
        let mut video = adaptive();
        video.select_format(Some("a1+v2")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a1+v2"));
        assert_eq!(
            video
                .requested_formats
                .iter()
                .map(|f| f.format_id.as_str())
                .collect::<Vec<_>>(),
            ["a1", "v2"]
        );
        assert_eq!(video.url, video.formats[2].url);
        assert_eq!(video.height, Some(1080));
        assert_eq!(video.filesize, None);
        assert_eq!(video.ext, "mp4");
        assert_eq!(video.stream_kind, StreamKind::Standalone);
        video.select_format(Some("v1")).unwrap();
        assert!(video.requested_formats.is_empty());
        assert_eq!(video.stream_kind, StreamKind::VideoOnly);
        assert_eq!(video.filesize, Some(42));
    }

    #[test]
    fn implicit_merge_requires_preference_and_complete_supported_pair() {
        let mut video = adaptive();
        video.select_format_with_merge(None, true).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2+a2"));
        assert!(video.select_format_with_merge(Some("best"), true).is_err());
        assert!(video.select_format(None).is_err());
        video
            .formats
            .retain(|f| f.stream_kind == StreamKind::VideoOnly);
        let before = snapshot(&video);
        assert!(video.select_format(None).is_err());
        assert!(video.select_format_with_merge(None, true).is_err());
        assert_eq!(snapshot(&video), before);
        video.formats.push(source(
            "complete",
            StreamKind::Standalone,
            "mp4",
            Some(360),
            None,
            None,
        ));
        video.select_format_with_merge(None, true).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("complete"));
        video.formats.push(source(
            "audio",
            StreamKind::AudioOnly,
            "mp3",
            None,
            None,
            None,
        ));
        video.select_format_with_merge(None, true).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("complete"));
    }

    #[test]
    fn merge_default_prefers_complete_pair_while_explicit_best_stays_standalone() {
        let mut video = adaptive();
        video.formats.push(source(
            "muxed",
            StreamKind::Standalone,
            "mp4",
            Some(2160),
            None,
            None,
        ));
        video.select_format(None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("muxed"));
        video.select_format_with_merge(None, true).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2+a2"));
        video.select_format_with_merge(Some("best"), true).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("muxed"));
        assert!(video.requested_formats.is_empty());
    }

    #[test]
    fn merge_container_policy_and_fallback_are_explicit() {
        let mut video = adaptive();
        video.formats[0].ext = "webm".into();
        video.formats[2].ext = "webm".into();
        video.select_format(Some("v1+a1")).unwrap();
        assert_eq!(video.ext, "webm");
        video.select_format(Some("v2+a1")).unwrap();
        assert_eq!(video.ext, "mkv");
        video
            .select_format(Some("missing+a1/v1+missing/a1+v1"))
            .unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a1+v1"));
    }

    #[test]
    fn errors_are_atomic_and_validate_all_fallback_grammar_first() {
        let mut video = adaptive();
        video.select_format(Some("v2+a2")).unwrap();
        let before = snapshot(&video);
        for expression in [
            "v1+missing",
            "v1+v2",
            "a1+a2",
            "v1+",
            "v1+a1+a2",
            "v1/",
            "/v1",
            "v1/(a1)",
            "v1/a1[abr>10]",
            "v1,a1",
            "v1/a1|a2",
            "",
        ] {
            assert!(
                video.select_format(Some(expression)).is_err(),
                "{expression}"
            );
            assert_eq!(snapshot(&video), before, "{expression}");
        }
    }

    #[test]
    fn direct_results_accept_fallback_but_validate_grammar() {
        let mut video = info(Vec::new());
        video.select_format(Some("missing/best")).unwrap();
        let before = snapshot(&video);
        assert!(video.select_format(Some("best/(bad)")).is_err());
        assert_eq!(snapshot(&video), before);
        video.stream_kind = StreamKind::VideoOnly;
        assert!(video.select_format(None).is_err());
    }

    #[test]
    fn failed_playlist_selection_preserves_preceding_entries() {
        let mut first = adaptive();
        first.id = "first".into();
        let mut second = adaptive();
        second.id = "second".into();
        second.formats.retain(|f| f.format_id != "v2");
        let mut playlist = ExtractResult::Playlist(PlaylistInfo {
            id: "list".into(),
            title: "List".into(),
            webpage_url: first.webpage_url.clone(),
            extractor: "test".into(),
            extractor_key: "Test".into(),
            entries: vec![first, second],
        });
        let before = serde_json::to_value(&playlist).unwrap();
        let error = playlist
            .select_format_with_merge(Some("v2+a2"), false)
            .unwrap_err();
        assert!(error.to_string().contains("second"));
        assert_eq!(serde_json::to_value(&playlist).unwrap(), before);
    }
}
