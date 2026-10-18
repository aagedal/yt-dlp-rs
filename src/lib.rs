mod archive;
mod crypto;
mod dash;
mod download;
mod error;
pub mod extractor;
mod fragment;
mod hls;
mod info;
mod merge;
pub mod output;
mod playlist;
mod retry;
mod selector;

pub use archive::{DownloadArchive, archive_id};
pub use download::{DownloadOptions, DownloadPhase, DownloadProgress};
pub use error::{Error, Result};
pub use extractor::YoutubeOptions;
pub use info::{
    ByteRange, ExtractResult, Fragment, HlsAes128, PlaylistInfo, StreamKind, VideoFormat, VideoInfo,
};
pub use playlist::PlaylistSelection;

use extractor::{Extractor, GenericHttp, Youtube};
use reqwest::blocking::Client;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

/// Validate the supported selector grammar without extraction or availability checks.
pub fn validate_format_selector(expression: &str) -> Result<()> {
    selector::parse(expression).map(|_| ())
}

/// Whole-extraction retry policy, independent of download retries.
/// Defaults to no retries to preserve existing library behavior.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractionOptions {
    /// Additional attempts after transient HTTP or HTML body read failures.
    pub retries: u8,
}

pub struct YoutubeDl {
    client: Client,
    extractors: Vec<Box<dyn Extractor>>,
    ffmpeg: Option<PathBuf>,
}

impl YoutubeDl {
    pub fn new() -> Result<Self> {
        Self::with_youtube_options(YoutubeOptions::default())
    }

    /// Configure YouTube extraction. The runtime path is inspected only when a
    /// YouTube player challenge needs solving; construction does not launch it.
    pub fn with_youtube_options(options: YoutubeOptions) -> Result<Self> {
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
            ffmpeg: None,
            extractors: vec![
                Box::new(Youtube::with_options(options)),
                Box::new(GenericHttp),
            ],
        })
    }

    /// Configure an explicit ffmpeg executable for two-stream copy merging.
    /// The path is inspected only for a merged download, never a single stream.
    /// Default selection prefers a video/audio pair when this option is present.
    pub fn with_ffmpeg(mut self, executable: impl Into<PathBuf>) -> Self {
        self.ffmpeg = Some(executable.into());
        self
    }

    /// Insert a site extractor before the generic fallback.
    pub fn register_extractor(&mut self, extractor: Box<dyn Extractor>) {
        self.extractors.insert(0, extractor);
    }

    pub fn extract_info(&self, input: &str) -> Result<VideoInfo> {
        self.extract_info_with_format(input, None)
    }

    /// Extract and select a supported single source, two-stream pair, or fallback.
    /// Full upstream selector grammar and ranking are unsupported.
    pub fn extract_info_with_format(&self, input: &str, format: Option<&str>) -> Result<VideoInfo> {
        self.extract_result_with_format(input, format)?.into_video()
    }

    /// Extract a video or an HTML5 playlist, resolving formats for every entry.
    pub fn extract_result(&self, input: &str) -> Result<ExtractResult> {
        self.extract_result_with_format(input, None)
    }

    /// The supported selector applies independently to each entry.
    /// Extraction or selection failure in any entry fails the whole result.
    pub fn extract_result_with_format(
        &self,
        input: &str,
        format: Option<&str>,
    ) -> Result<ExtractResult> {
        self.extract_result_with_options(input, format, ExtractionOptions::default())
    }

    /// Retry the entire extractor registry after transient network failures.
    /// Earlier page reads and source probes repeat; no partial result is returned.
    /// Format selection and unsupported behavior are never retried.
    pub fn extract_result_with_options(
        &self,
        input: &str,
        format: Option<&str>,
        options: ExtractionOptions,
    ) -> Result<ExtractResult> {
        let mut info = self.extract_result_unselected_with_options(input, options)?;
        self.select_result(&mut info, format)?;
        Ok(info)
    }

    /// Apply a requested or configured default selector atomically to an extracted result.
    pub fn select_result(&self, result: &mut ExtractResult, format: Option<&str>) -> Result<()> {
        result.select_format_with_merge(format, self.ffmpeg.is_some())
    }

    /// Extract all supported alternatives without applying library format selection.
    /// Useful for inspection, including separate-only presentations without a merger.
    /// Metadata retains the extractor's representative fields; select before downloading.
    pub fn extract_result_unselected(&self, input: &str) -> Result<ExtractResult> {
        self.extract_result_unselected_with_options(input, ExtractionOptions::default())
    }

    /// Retry whole extraction independently of format selection and output rendering.
    /// All playlist entries must extract successfully before a result is returned.
    pub fn extract_result_unselected_with_options(
        &self,
        input: &str,
        options: ExtractionOptions,
    ) -> Result<ExtractResult> {
        let url = Url::parse(input)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Error::UnsupportedScheme(url.scheme().into()));
        }
        for attempt in 0..=options.retries {
            match self.extract_once(&url) {
                Ok(info) => return Ok(info),
                Err(error) => {
                    let Some(delay) = retry::failure_delay(&error) else {
                        return Err(error);
                    };
                    if attempt == options.retries {
                        return Err(error);
                    }
                    retry::backoff(attempt, delay);
                }
            }
        }
        unreachable!("the final attempt returns success or an error")
    }

    fn extract_once(&self, url: &Url) -> Result<ExtractResult> {
        for extractor in &self.extractors {
            if let Some(info) = extractor.extract_result(&self.client, url)? {
                return Ok(info);
            }
        }
        Err(Error::UnsupportedUrl(url.to_string()))
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
        self.download_with_progress(info, destination, options, |_| {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Observe download checkpoints; return `ControlFlow::Break(())` to cancel.
    /// Cancellation removes the private temporary file and never retries or publishes.
    /// Callbacks run synchronously before requests, after validated headers and each
    /// written chunk, during retry waits (at intervals of at most 100 ms of sleep),
    /// and immediately before publication. `Publishing` is not a success event;
    /// only an `Ok` return confirms publication. Blocking requests, reads, and disk
    /// operations cannot be interrupted; existing request timeouts still apply.
    /// Byte counts may reset on whole-file/fragment retries or selected stream changes.
    /// No callback follows publication.
    pub fn download_with_progress(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<u64> {
        if !info.requested_formats.is_empty() {
            return self.download_merged(info, destination.as_ref(), options, &mut callback);
        }
        if !info.fragments.is_empty() {
            return fragment::download(
                &self.client,
                info,
                destination.as_ref(),
                options,
                &mut callback,
            );
        }
        download::download(
            &self.client,
            info,
            destination.as_ref(),
            options,
            &mut callback,
        )
    }
    fn download_merged(
        &self,
        info: &VideoInfo,
        destination: &Path,
        options: DownloadOptions,
        callback: &mut dyn FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<u64> {
        use std::{fs, ops::ControlFlow};
        if destination.exists() {
            return Err(Error::OutputExists(destination.into()));
        }
        let streams = &info.requested_formats;
        if streams.len() != 2 {
            return Err(Error::Postprocess(
                "exactly two selected streams are required".into(),
            ));
        }
        let video = streams
            .iter()
            .position(|stream| stream.stream_kind == StreamKind::VideoOnly);
        let audio = streams
            .iter()
            .position(|stream| stream.stream_kind == StreamKind::AudioOnly);
        let (Some(video), Some(audio)) = (video, audio) else {
            return Err(Error::Postprocess(
                "merging requires one video-only and one audio-only stream".into(),
            ));
        };
        let video_ext = streams[video].ext.as_str();
        let audio_ext = streams[audio].ext.as_str();
        let output_ext = info::merge_extension(&streams[video], &streams[audio])
            .ok_or_else(|| Error::Postprocess("unsupported stream containers".into()))?;
        if info.ext != output_ext {
            return Err(Error::Postprocess(
                "merge output container disagrees with selected streams".into(),
            ));
        }
        let executable = self.ffmpeg.as_ref().ok_or_else(|| {
            Error::Postprocess(
                "separate streams require --ffmpeg-location PATH or YoutubeDl::with_ffmpeg".into(),
            )
        })?;
        let executable = executable.canonicalize().map_err(|error| {
            Error::Postprocess(format!("invalid ffmpeg executable path: {error}"))
        })?;
        if !executable.is_file() {
            return Err(Error::Postprocess(
                "ffmpeg executable path is not a file".into(),
            ));
        }
        for stream in streams {
            if !matches!(stream.url.scheme(), "http" | "https") {
                return Err(Error::UnsupportedScheme(stream.url.scheme().into()));
            }
        }
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let workspace = tempfile::Builder::new()
            .prefix(".yt-dlp-rs-merge-")
            .tempdir_in(parent)?;
        let mut paths = Vec::new();
        let mut stored = 0u64;
        for (index, stream) in streams.iter().enumerate() {
            let path = workspace
                .path()
                .join(format!("input-{index}.{}", stream.ext));
            let mut part = info.clone();
            part.fragments = stream.fragments.clone();
            part.requested_formats.clear();
            part.formats.clear();
            part.url = stream.url.clone();
            part.ext = stream.ext.clone();
            part.stream_kind = stream.stream_kind;
            part.format_id = Some(stream.format_id.clone());
            part.filesize = stream.filesize;
            part.width = stream.width;
            part.height = stream.height;
            let bytes = self.download_with_progress(
                &part,
                &path,
                options,
                |mut progress: DownloadProgress| {
                    if progress.phase == DownloadPhase::Publishing {
                        return ControlFlow::Continue(());
                    }
                    progress.stream_index = Some(index as u8 + 1);
                    progress.stream_count = Some(2);
                    callback(progress)
                },
            )?;
            stored = stored
                .checked_add(bytes)
                .ok_or_else(|| Error::Postprocess("combined stream byte count overflow".into()))?;
            paths.push(path);
        }
        let output = tempfile::Builder::new()
            .prefix("merged-")
            .suffix(".part")
            .tempfile_in(workspace.path())?
            .into_temp_path();
        merge::merge(
            &executable,
            &paths[video],
            video_ext,
            &paths[audio],
            audio_ext,
            &output,
            output_ext,
            &mut || {
                callback(DownloadProgress {
                    attempt: 1,
                    downloaded_bytes: stored,
                    total_bytes: Some(stored),
                    phase: DownloadPhase::Merging,
                    stream_index: None,
                    stream_count: Some(2),
                    fragment_index: None,
                    fragment_count: None,
                })
            },
        )?;
        let bytes = fs::metadata(&output)?.len();
        fs::OpenOptions::new()
            .write(true)
            .open(&output)?
            .sync_all()?;
        if callback(DownloadProgress {
            attempt: 1,
            downloaded_bytes: bytes,
            total_bytes: Some(bytes),
            phase: DownloadPhase::Publishing,
            stream_index: None,
            stream_count: Some(2),
            fragment_index: None,
            fragment_count: None,
        })
        .is_break()
        {
            return Err(Error::DownloadCancelled);
        }
        output.persist_noclobber(destination).map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::OutputExists(destination.into())
            } else {
                Error::Io(error.error)
            }
        })?;
        Ok(bytes)
    }
}
