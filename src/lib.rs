mod archive;
mod audio;
mod audio_download;
mod cookies;
mod crypto;
mod dash;
mod download;
mod embed_subtitles;
mod error;
pub mod extractor;
mod fragment;
mod hls;
mod http;
mod info;
mod merge;
pub mod output;
mod playlist;
mod postprocess;
mod retry;
mod selector;
mod subtitle_convert;
mod subtitles;
mod video;

pub use archive::{DownloadArchive, archive_id};
pub use audio::{AudioDownload, AudioOptions, audio_original_path, audio_output_path};
pub use cookies::CookieJar;
pub use download::{DownloadOptions, DownloadPhase, DownloadProgress};
pub use embed_subtitles::{SubtitleEmbedding, SubtitleFile};
pub use error::{Error, Result};
pub use extractor::YoutubeOptions;
pub use http::HttpOptions;
pub use info::{
    ByteRange, ExtractResult, Fragment, HlsAes128, PlaylistInfo, StreamKind, VideoFormat, VideoInfo,
};
pub use output::{render_automatic_captions, render_subtitles};
pub use playlist::PlaylistSelection;
pub use postprocess::{PostprocessOptions, ProcessedDownload};
pub use subtitle_convert::{ConvertedSubtitle, SubtitleConversion};
pub use subtitles::{
    RequestedSubtitles, SubtitleFormat, SubtitleMap, SubtitleOptions, SubtitleSelection, Subtitles,
    subtitle_paths,
};
pub use video::{VideoMode, VideoOptions};

use extractor::{Extractor, GenericHttp, Youtube};
use reqwest::blocking::Client;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
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
    cookies: Arc<CookieJar>,
    extractors: Vec<Box<dyn Extractor>>,
    ffmpeg: Option<PathBuf>,
    ffprobe: Option<PathBuf>,
}

impl YoutubeDl {
    pub fn new() -> Result<Self> {
        Self::with_youtube_options(YoutubeOptions::default())
    }

    /// Configure YouTube extraction. The runtime path is inspected only when a
    /// YouTube player challenge needs solving; construction does not launch it.
    pub fn with_youtube_options(options: YoutubeOptions) -> Result<Self> {
        Self::with_options(options, HttpOptions::default())
    }

    /// Configure global HTTP defaults together with YouTube extraction.
    /// Per-request extractor and transport headers override these defaults.
    pub fn with_options(youtube: YoutubeOptions, http: HttpOptions) -> Result<Self> {
        let headers = http.header_map()?;
        let proxy = http.proxy_config()?;
        let cookies = http.cookies.unwrap_or_default();
        let builder = Client::builder()
            .user_agent(concat!("yt-dlp-rs/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers.clone())
            .cookie_provider(cookies.clone())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .redirect(reqwest::redirect::Policy::limited(10))
            // Preserve an HTML5 page's explicit referrer across media redirects.
            .referer(false);
        let client = proxy.apply(builder)?.build()?;
        Ok(Self {
            client,
            ffmpeg: None,
            ffprobe: None,
            cookies: cookies.clone(),
            extractors: vec![
                Box::new(Youtube::with_http_options(
                    youtube,
                    headers,
                    cookies.clone(),
                    proxy,
                )),
                Box::new(GenericHttp),
            ],
        })
    }

    /// Return shared scoped cookies for explicit library persistence or reuse.
    pub fn cookie_jar(&self) -> Arc<CookieJar> {
        self.cookies.clone()
    }

    /// Configure an explicit ffmpeg executable for two-stream copy merging.
    /// The path is inspected only for a merged download, never a single stream.
    /// Default selection prefers a video/audio pair when this option is present.
    pub fn with_ffmpeg(mut self, executable: impl Into<PathBuf>) -> Self {
        self.ffmpeg = Some(executable.into());
        self
    }

    /// Configure an explicit ffprobe executable for audio codec inspection.
    /// Without this override, audio processing uses ffprobe beside explicit ffmpeg.
    pub fn with_ffprobe(mut self, executable: impl Into<PathBuf>) -> Self {
        self.ffprobe = Some(executable.into());
        self
    }

    /// Download and extract audio, publishing the result without overwriting.
    pub fn download_audio_with_options(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        audio: &AudioOptions,
    ) -> Result<AudioDownload> {
        self.download_audio_with_progress(info, destination, options, audio, |_| {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Observe/cancel transfer and audio processing. Only final publication emits
    /// Publishing; a processing failure can retain the completed source download.
    pub fn download_audio_with_progress(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        audio: &AudioOptions,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<AudioDownload> {
        audio_download::download(
            self,
            info,
            destination.as_ref(),
            options,
            audio,
            &mut callback,
        )
    }

    /// Download, then extract audio and/or remux/recode local media in order.
    pub fn download_processed_with_options(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        processors: &PostprocessOptions,
    ) -> Result<ProcessedDownload> {
        self.download_processed_with_progress(info, destination, options, processors, |_| {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Observe/cancel downloads and local processors before final publication.
    /// A processing failure preserves its latest completed input; keep-video also
    /// retains earlier stages. Multiple retained/final publications can partially succeed.
    pub fn download_processed_with_progress(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        processors: &PostprocessOptions,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<ProcessedDownload> {
        postprocess::download(
            self,
            info,
            destination.as_ref(),
            options,
            processors,
            &mut callback,
        )
    }

    /// Convert a local subtitle sidecar, preserving its source on failure.
    pub fn convert_subtitle_with_options(
        &self,
        source: impl AsRef<Path>,
        source_ext: &str,
        options: &SubtitleConversion,
    ) -> Result<ConvertedSubtitle> {
        self.convert_subtitle_with_progress(source, source_ext, options, |_| {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Observe/cancel subtitle processing before no-clobber final publication.
    pub fn convert_subtitle_with_progress(
        &self,
        source: impl AsRef<Path>,
        source_ext: &str,
        options: &SubtitleConversion,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<ConvertedSubtitle> {
        subtitle_convert::convert(self, source.as_ref(), source_ext, options, &mut callback)
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
    /// Copy a selected HTTP subtitle resource using the native whole-file transport.
    /// Caption bytes are not parsed or converted; the page referrer is preserved.
    pub fn download_subtitle_with_options(
        &self,
        info: &VideoInfo,
        subtitle: &SubtitleFormat,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
    ) -> Result<u64> {
        self.download_subtitle_with_progress(info, subtitle, destination, options, |_| {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Observe/cancel a subtitle transfer at the same checkpoints as whole-file media.
    pub fn download_subtitle_with_progress(
        &self,
        info: &VideoInfo,
        subtitle: &SubtitleFormat,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<u64> {
        download::download_subtitle(
            &self.client,
            info,
            subtitle,
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
