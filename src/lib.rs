mod archive;
mod audio;
mod audio_download;
pub mod batch;
pub mod codecs;
pub mod config;
pub mod container;
mod cookies;
mod crypto;
mod dash;
pub mod dates;
mod download;
mod embed_metadata;
mod embed_subtitles;
mod error;
pub mod extractor;
mod ffmpeg_fd;
mod filetime;
mod fixup;
pub mod format_sort;
mod fragment;
mod hls;
mod html;
mod http;
mod info;
mod info_json;
mod match_filter;
mod merge;
pub mod metadata_parser;
mod modify_chapters;
pub mod output;
mod playlist;
mod postprocess;
pub mod pp_args;
mod retry;
pub mod screen;
pub mod sections;
mod selector;
pub mod sponsorblock;
mod subtitle_convert;
mod subtitles;
mod thumbnail_process;
mod video;
pub mod xattr;

pub use archive::{DownloadArchive, archive_id};
pub use audio::{AudioDownload, AudioOptions, audio_original_path, audio_output_path};
pub use cookies::CookieJar;
pub use download::{DownloadOptions, DownloadPhase, DownloadProgress, MergeFiles, PartFiles};
pub use embed_subtitles::{SubtitleEmbedding, SubtitleFile};
pub use error::{Error, Result};
pub use extractor::YoutubeOptions;
pub use format_sort::FormatSorter;
pub use http::HttpOptions;
pub use info::{
    ByteRange, Chapter, DownloaderOptions, ExtractResult, Fragment, HlsAes128, Metadata,
    PlaylistInfo, PyNumber, StreamKind, Thumbnail, VideoFormat, VideoInfo,
};
pub use info_json::{
    InfoJsonWrite, info_json_path, load_info_json, playlist_info_json, playlist_info_json_with,
    to_python_json, video_info_json, video_info_json_with, write_info_json, write_info_json_with,
};
pub use match_filter::{EntrySelection, MatchFilters, MatchOutcome};
pub use output::{render_automatic_captions, render_subtitles};
pub use playlist::{PlaylistCounts, PlaylistSelection};
pub use postprocess::{InfoJsonEmbed, ModifyChapters, PostprocessOptions, ProcessedDownload};
pub use retry::{RetrySleep, SleepExpression};
pub use subtitle_convert::{ConvertedSubtitle, SubtitleConversion};
pub use subtitles::{
    RequestedSubtitles, SubtitleFormat, SubtitleMap, SubtitleOptions, SubtitleSelection, Subtitles,
    subtitle_paths,
};
pub use thumbnail_process::{ThumbnailConversion, fixup_webp};
pub use video::{VideoMode, VideoOptions};

use extractor::{Extractor, GenericHttp, Youtube};
use reqwest::blocking::Client;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
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
    merge_output_format: Option<Vec<String>>,
    format_sort: FormatSortOptions,
    prefer_best: bool,
    /// Global request headers and proxy for FFmpeg network inputs.
    headers: reqwest::header::HeaderMap,
    proxy: Option<String>,
    ffmpeg_settings: ffmpeg_fd::Settings,
    /// `--http-chunk-size` and `--test`.
    http_chunk_size: Option<u64>,
    test: bool,
    /// `--use-extractors` (upstream extractor names in matching order) and
    /// `--force-generic-extractor`.
    allowed_extractors: Option<Vec<&'static str>>,
    force_generic: bool,
    /// `--default-search`.
    default_search: Option<String>,
    check_formats: CheckFormats,
    /// `-N`/`--concurrent-fragments`.
    concurrent_fragments: usize,
    /// `--keep-fragments`.
    keep_fragments: bool,
    /// `--enable-file-urls`.
    enable_file_urls: bool,
    /// `--downloader [PROTO:]NAME` choices by simplified protocol (`default`
    /// for the rest): `true` selects FFmpegFD, `false` the native downloaders.
    downloaders: std::collections::HashMap<String, bool>,
}

/// A transport error as urllib words it: `[Errno N] TEXT` for an OS error.
fn urllib_error(error: &reqwest::Error) -> String {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(io) = current.downcast_ref::<std::io::Error>()
            && let Some(code) = io.raw_os_error()
        {
            let text = io.to_string();
            let text = text
                .strip_suffix(&format!(" (os error {code})"))
                .unwrap_or(&text);
            return format!("[Errno {code}] {text}");
        }
        source = current.source();
    }
    error.to_string()
}

/// HlsFD's `Total fragments` for `info` after `refresh_hls`: the whole
/// playlist's fragments even when only a discontinuity section downloads.
pub fn total_fragments(info: &VideoInfo) -> usize {
    FRAGMENT_TOTAL.with(|total| match &*total.borrow() {
        Some((url, count)) if *url == info.url => *count,
        _ => info.fragments.len(),
    })
}

thread_local! {
    /// The refetched playlist URL and its fragment count (section formats).
    static FRAGMENT_TOTAL: std::cell::RefCell<Option<(Url, usize)>> = const { std::cell::RefCell::new(None) };
    /// The URL whose download `announce_download` already reported.
    static ANNOUNCED: std::cell::RefCell<Option<Url>> = const { std::cell::RefCell::new(None) };
}

/// Upstream `dl`'s `Invoking … downloader on "URL"[, "URL"]` debug line;
/// `data:` URLs are abbreviated.
fn report_invoking(name: &str, urls: &[&Url]) {
    if !screen::verbose() {
        return;
    }
    let urls: Vec<String> = urls
        .iter()
        .map(|url| {
            let url = url.as_str();
            if url.starts_with("data:") {
                format!("{},<data>", url.split(',').next().unwrap_or(url))
            } else {
                url.to_owned()
            }
        })
        .collect();
    screen::debug(&format!(
        "Invoking {name} downloader on \"{}\"",
        urls.join("\", \"")
    ));
}

/// Upstream `shorten_protocol_name(proto, simplify=True)`.
fn simplify_protocol(protocol: &str) -> &str {
    match protocol {
        "https" => "http",
        "ftps" => "ftp",
        "m3u8" | "m3u8_native" => "m3u8",
        "http_dash_segments" | "http_dash_segments_generator" => "dash",
        "rtmp_ffmpeg" => "rtmp",
        other => other,
    }
}

/// Upstream `--fixup POLICY` and `--hls-use-mpegts`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixupOptions {
    /// `never`, `ignore`, `warn`, `detect_or_warn` (default) or `force`.
    pub policy: Option<String>,
    pub hls_use_mpegts: bool,
}

/// Upstream `check_formats`: `--check-formats` (`selected`),
/// `--check-all-formats` (`True`) and `--no-check-formats` (`False`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckFormats {
    /// Only formats that need testing (none in the represented extractors).
    #[default]
    Default,
    Selected,
    All,
    Never,
}

/// Upstream `dl(temp, format, test=True)`: a quiet `_TEST_FILE_SIZE` HTTP
/// download, or the first fragment of a fragmented format.
fn test_format(client: &Client, info: &VideoInfo, format: &VideoFormat) -> bool {
    let mut single = info.with_selected_format(format);
    single.fragments.truncate(1);
    let Ok(directory) = tempfile::tempdir() else {
        return false;
    };
    let target = directory.path().join("format.tmp");
    // The test parameters carry no retry counts, so nothing is retried.
    let options = DownloadOptions {
        part_files: PartFiles::NoPart { resume: false },
        skip_unavailable_fragments: Some(false),
        retries: 0,
        fragment_retries: 0,
        ..Default::default()
    };
    let mut quiet = |_| std::ops::ControlFlow::Continue(());
    let result = if single.fragments.is_empty() {
        download::download(
            client,
            &single,
            &target,
            options,
            Some(download::Chunking {
                size: download::TEST_FILE_SIZE,
                test: true,
            }),
            &mut quiet,
        )
    } else {
        fragment::download(client, &single, &target, options, 1, false, &mut quiet)
    };
    // Errors are still reported by the quiet test downloader.
    if let Err(error) = &result {
        screen::stderr(&format!("ERROR: {error}"));
    }
    result.is_ok()
}

/// Upstream `parse_duration`: whole durations are `int`, others `float`.
pub fn parse_duration(text: &str) -> Option<PyNumber> {
    match_filter::parse_duration_number(text)
}

/// Upstream extractor names this port knows, in `gen_extractor_classes`
/// order (`end` is the unsupported-URL fallback).
const EXTRACTOR_NAMES: [&str; 4] = ["html5", "youtube", "generic", "end"];

/// Upstream `orderedSet_from_options(names, {'all', 'default'}, use_regex=True)`.
fn ordered_extractors(
    options: &[String],
    start: Vec<&'static str>,
) -> std::result::Result<Vec<&'static str>, String> {
    let mut requested = start;
    for option in options {
        let (discard, value) = match option.strip_prefix('-') {
            Some(value) => (true, value),
            None => (false, option.as_str()),
        };
        let alias: Option<&[&str]> = match value {
            "all" => Some(&EXTRACTOR_NAMES),
            "default" => Some(&EXTRACTOR_NAMES[..3]),
            _ => None,
        };
        if let Some(names) = alias {
            let flipped: Vec<String> = names
                .iter()
                .map(|name| {
                    if discard {
                        format!("-{name}")
                    } else {
                        (*name).to_owned()
                    }
                })
                .collect();
            requested = ordered_extractors(&flipped, requested)?;
            continue;
        }
        let regex = regex::RegexBuilder::new(&format!("^(?:{value})$"))
            .case_insensitive(true)
            .build()
            .map_err(|_| format!("Wrong regex for allowed_extractors: {value}"))?;
        for name in EXTRACTOR_NAMES.iter().filter(|name| regex.is_match(name)) {
            if discard {
                requested.retain(|item| item != name);
            } else {
                requested.push(name);
            }
        }
    }
    let mut seen = Vec::new();
    requested.retain(|name| {
        let new = !seen.contains(name);
        seen.push(*name);
        new
    });
    Ok(requested)
}

/// Python `urlparse(url).scheme` is nonempty.
fn has_scheme(input: &str) -> bool {
    input.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Upstream `-S`/`--format-sort`, `--format-sort-force` and
/// `--prefer-free-formats`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormatSortOptions {
    pub fields: Vec<String>,
    pub force: bool,
    pub prefer_free: bool,
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
        let proxy_url = http.proxy.clone();
        let proxy = http.transport_config()?;
        let cookies = http.cookies.unwrap_or_default();
        let builder = Client::builder()
            .user_agent(concat!("yt-dlp-rs/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers.clone())
            .cookie_provider(cookies.clone())
            .redirect(reqwest::redirect::Policy::limited(10))
            // Preserve an HTML5 page's explicit referrer across media redirects.
            .referer(false);
        let client = proxy.apply(builder)?.build()?;
        Ok(Self {
            client,
            ffmpeg: None,
            merge_output_format: None,
            format_sort: FormatSortOptions::default(),
            prefer_best: false,
            headers: headers.clone(),
            proxy: proxy_url,
            ffmpeg_settings: ffmpeg_fd::Settings::default(),
            http_chunk_size: None,
            test: false,
            allowed_extractors: None,
            force_generic: false,
            default_search: None,
            check_formats: CheckFormats::Default,
            concurrent_fragments: 1,
            keep_fragments: false,
            enable_file_urls: false,
            downloaders: Default::default(),
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

    /// Upstream `FFmpegFixupM3u8PP`: when `path` holds MPEG-TS, print
    /// `[FixupM3u8] Fixing MPEG-TS in MP4 container of "PATH"` and remux it to
    /// MP4 in place (`aac_adtstoasc` when ffprobe reports AAC audio).
    /// Returns whether a fixup ran; requires a configured ffmpeg.
    pub fn fixup_m3u8(&self, path: &Path) -> Result<bool> {
        self.fixup_m3u8_as(path, path)
    }

    /// [`Self::fixup_m3u8`] naming `shown` (the destination) in its message.
    fn fixup_m3u8_as(&self, path: &Path, shown: &Path) -> Result<bool> {
        if crate::container::sniff(path) != Some("ts") {
            return Ok(false);
        }
        let ffmpeg = self
            .ffmpeg
            .as_ref()
            .ok_or_else(|| Error::Postprocess("fixup requires ffmpeg".into()))?;
        screen::screen(&format!(
            "[FixupM3u8] Fixing MPEG-TS in MP4 container of \"{}\"",
            shown.display()
        ));
        let probe = self.ffprobe.clone().or_else(|| {
            ffmpeg
                .parent()
                .map(|dir| dir.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX)))
        });
        let aac = probe.is_some_and(|probe| {
            audio::ffmpeg::probe(&probe, path, "ts", &mut || {
                std::ops::ControlFlow::Continue(())
            })
            .is_ok_and(|codec| codec == "aac")
        });
        fixup::remux_to_mp4("FixupM3u8", ffmpeg, path, "ts", aac, &mut || {
            std::ops::ControlFlow::Continue(())
        })?;
        Ok(true)
    }

    /// Upstream `FFmpegFixupM4aPP`: print `[FixupM4a] Correcting container of
    /// "PATH"` and remux a DASH m4a download to a plain MP4 container.
    pub fn fixup_m4a(&self, path: &Path) -> Result<()> {
        self.fixup_m4a_as(path, path)
    }

    fn fixup_m4a_as(&self, path: &Path, shown: &Path) -> Result<()> {
        let ffmpeg = self
            .ffmpeg
            .as_ref()
            .ok_or_else(|| Error::Postprocess("fixup requires ffmpeg".into()))?;
        screen::screen(&format!(
            "[FixupM4a] Correcting container of \"{}\"",
            shown.display()
        ));
        let container = crate::container::sniff(path).unwrap_or("m4a");
        fixup::remux_to_mp4("FixupM4a", ffmpeg, path, container, false, &mut || {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Upstream `HlsFD.real_download`: `[hlsnative] Downloading m3u8
    /// manifest` refetches the media playlist, whose fragments are then
    /// downloaded. `None` for other downloads.
    pub fn refresh_hls(&self, info: &VideoInfo) -> Result<Option<VideoInfo>> {
        if info.fragments.is_empty()
            || !info.protocol.as_deref().unwrap_or("").starts_with("m3u8")
            || self.uses_ffmpeg_fd(info)
        {
            return Ok(None);
        }
        screen::screen("[hlsnative] Downloading m3u8 manifest");
        let referer = (!info.is_direct()).then_some(&info.webpage_url);
        let response = crate::extractor::direct::probe(&self.client, &info.url, referer)?;
        let (_, playlist) = crate::extractor::hls::read_playlist(response)?;
        let hls::Playlist::Media {
            fragments,
            discontinuity,
            ..
        } = playlist
        else {
            return Err(Error::UnsupportedManifest(
                "the refetched HLS manifest is not a media playlist".into(),
            ));
        };
        // HlsFD keeps only the format's discontinuity section.
        let index = info
            .extra
            .get("format_index")
            .and_then(serde_json::Value::as_u64)
            .and_then(|index| u32::try_from(index).ok());
        let mut refreshed = info.clone();
        // HlsFD's `Total fragments` counts every media URI of the playlist.
        FRAGMENT_TOTAL
            .with(|total| total.replace(index.map(|_| (info.url.clone(), fragments.len()))));
        refreshed.fragments = hls::section(fragments, &discontinuity, index);
        Ok(Some(refreshed))
    }

    /// Upstream `YoutubeDL.fixup()` for an unmerged download at `path`
    /// (shown as `shown`): `FixupM4a` for `m4a_dash` containers and
    /// `FixupM3u8` for native HLS MP4 downloads, under `--fixup` policies.
    pub fn fixup(
        &self,
        info: &VideoInfo,
        path: &Path,
        shown: &Path,
        options: &FixupOptions,
    ) -> Result<()> {
        let policy = options.policy.as_deref().unwrap_or("detect_or_warn");
        // FFmpegFD downloads and merges are never fixed up.
        if !info.requested_formats.is_empty()
            || self.uses_ffmpeg_fd(info)
            || matches!(policy, "never" | "ignore")
        {
            return Ok(());
        }
        let protocol = info.protocol.as_deref().unwrap_or("");
        let m3u8 = protocol.starts_with("m3u8") && !options.hls_use_mpegts;
        let m4a = info.ext == "m4a" && info.container.as_deref() == Some("m4a_dash");
        for (needed, message, m3u8_fixup) in [
            (
                m4a,
                "writing DASH m4a. Only some players support this container",
                false,
            ),
            (
                m3u8,
                "Possible MPEG-TS in MP4 container or malformed AAC timestamps",
                true,
            ),
        ] {
            if !needed {
                continue;
            }
            if policy == "warn" {
                screen::warning(&format!("{}: {message}", info.id));
            } else if self.ffmpeg.is_none() {
                screen::warning(&format!(
                    "{}: {message}. Install ffmpeg to fix this automatically",
                    info.id
                ));
            } else if m3u8_fixup {
                if matches!(info.ext.as_str(), "mp4" | "m4a") {
                    self.fixup_m3u8_as(path, shown)?;
                }
            } else {
                self.fixup_m4a_as(path, shown)?;
            }
        }
        Ok(())
    }

    /// Upstream `FFmpegSplitChaptersPP`: cut `input` into one file per
    /// chapter (`destinations` pairs each chapter with its prepared path).
    /// With `force_keyframes` (`--force-keyframes-at-cuts`), a re-encoded copy
    /// with keyframes at the chapter starts is split instead.
    pub fn split_chapters(
        &self,
        input: &Path,
        chapters: &[(Chapter, PathBuf)],
        force_keyframes: bool,
    ) -> Result<()> {
        let ffmpeg = self.ffmpeg.as_ref().ok_or_else(|| {
            Error::Postprocess(
                "ffmpeg not found. Please install or provide the path using --ffmpeg-location"
                    .into(),
            )
        })?;
        // Upstream deletes the keyframes copy afterwards, without a message.
        let mut workspace = None;
        let mut input = input.to_path_buf();
        if force_keyframes && chapters.len() > 1 {
            screen::screen(&format!(
                "[SplitChapters] Re-encoding \"{}\" with appropriate keyframes",
                input.display()
            ));
            let parent = input
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let directory = tempfile::Builder::new()
                .prefix(".yt-dlp-rs-keyframes-")
                .tempdir_in(parent)?;
            let ext = input
                .extension()
                .map(|ext| ext.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let container = crate::container::sniff(&input).unwrap_or(&ext).to_owned();
            let keyframes = directory.path().join(format!("keyframes.{ext}"));
            let starts: Vec<PyNumber> = chapters
                .iter()
                .map(|(chapter, _)| chapter.start_time.unwrap_or(PyNumber::Int(0)))
                .collect();
            modify_chapters::force_keyframes(
                ffmpeg,
                "SplitChapters",
                &input,
                &container,
                &ext,
                &keyframes,
                &starts,
            )?;
            input = keyframes;
            workspace = Some(directory);
        }
        let input = input.as_path();
        screen::screen(&format!(
            "[SplitChapters] Splitting video by chapters; {} chapters found",
            chapters.len()
        ));
        let py_str = |value: PyNumber| match value {
            PyNumber::Int(value) => value.to_string(),
            PyNumber::Float(value) => screen::py_float(value),
        };
        for (index, (chapter, destination)) in chapters.iter().enumerate() {
            let (Some(start), Some(end)) = (chapter.start_time, chapter.end_time) else {
                return Err(Error::Postprocess(
                    "chapter end times need the video duration".into(),
                ));
            };
            let duration = match (end, start) {
                (PyNumber::Int(end), PyNumber::Int(start)) => PyNumber::Int(end - start),
                (end, start) => PyNumber::Float(end.as_f64() - start.as_f64()),
            };
            screen::screen(&format!(
                "[SplitChapters] Chapter {:03}; Destination: {}",
                index + 1,
                destination.display()
            ));
            let ext = destination
                .extension()
                .map(|ext| ext.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            fixup::cut(
                ffmpeg,
                input,
                destination,
                &py_str(start),
                &py_str(duration),
                embed_metadata::muxer(&ext),
                &mut || std::ops::ControlFlow::Continue(()),
            )?;
        }
        drop(workspace);
        Ok(())
    }

    /// Upstream `SponsorBlockPP.run` (an `after_filter` processor): fetch the
    /// video's segments in `categories` from `api` and store them as
    /// `sponsorblock_chapters`. `retries` follows `--extractor-retries`.
    pub fn fetch_sponsorblock(
        &self,
        info: &mut VideoInfo,
        categories: &[String],
        api: &str,
        retries: u32,
    ) -> Result<()> {
        let say = |text: &str| screen::screen(&format!("[SponsorBlock] {text}"));
        if !sponsorblock::supported(info) {
            say(&format!(
                "SponsorBlock is not supported for {}",
                info.extractor_key
            ));
            return Ok(());
        }
        say("Fetching SponsorBlock segments");
        let url = sponsorblock::query_url(api, &info.id, categories);
        screen::debug(&format!("SponsorBlock query: {url}"));
        // `_download_json` with `RetryManager`: 404 means no segments.
        let mut attempt = 0;
        let body: Option<serde_json::Value> = loop {
            attempt += 1;
            let result = self
                .client
                .get(&url)
                .send()
                .map_err(|error| urllib_error(&error))
                .and_then(|response| {
                    let status = response.status();
                    if status == reqwest::StatusCode::NOT_FOUND {
                        return Ok(None);
                    }
                    if !status.is_success() {
                        return Err(format!(
                            "HTTP Error {}: {}",
                            status.as_u16(),
                            status.canonical_reason().unwrap_or("")
                        ));
                    }
                    let bytes = response.bytes().map_err(|error| error.to_string())?;
                    serde_json::from_slice::<serde_json::Value>(&bytes)
                        .map(Some)
                        .map_err(|error| error.to_string())
                });
            match result {
                Ok(body) => break body,
                Err(error) => {
                    let error = format!("Unable to communicate with SponsorBlock API: {error}");
                    if attempt > retries {
                        return Err(Error::Postprocess(error));
                    }
                    screen::warning(&format!("{error}. Retrying ({attempt}/{retries})..."));
                }
            }
        };
        let segments: Vec<serde_json::Value> = body
            .as_ref()
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .find(|entry| entry["videoID"].as_str() == Some(info.id.as_str()))
            .and_then(|entry| entry["segments"].as_array().cloned())
            .unwrap_or_default();
        let (chapters, mismatched) = sponsorblock::chapters(&segments, info.metadata.duration)?;
        if mismatched {
            screen::warning(
                "Some SponsorBlock segments are from a video of different duration, maybe from an old version of this video",
            );
        }
        if chapters.is_empty() {
            say("No matching segments were found in the SponsorBlock database");
        } else {
            say(&format!(
                "Found {} segments in the SponsorBlock database",
                chapters.len()
            ));
        }
        info.extra.insert(
            "sponsorblock_chapters".into(),
            chapters
                .iter()
                .map(sponsorblock::SponsorChapter::to_json)
                .collect(),
        );
        Ok(())
    }

    /// Upstream `FFmpegConcatPP.concat_files` for `--concat-playlist`:
    /// several files with identical streams are concatenated into `output`.
    pub fn concat_files(&self, inputs: &[PathBuf], output: &Path) -> Result<()> {
        let ffmpeg = self.ffmpeg.as_ref().ok_or_else(|| {
            Error::Postprocess(
                "ffmpeg not found. Please install or provide the path using --ffmpeg-location"
                    .into(),
            )
        })?;
        let ffprobe = self
            .ffprobe
            .clone()
            .unwrap_or_else(|| ffmpeg.with_file_name("ffprobe"));
        modify_chapters::concat_files(ffmpeg, &ffprobe, inputs, output)
    }

    /// Whether an ffmpeg executable is configured (upstream `merger.available`).
    /// Upstream `dl`'s `--verbose` line naming the downloader, for callers
    /// that announce a download before starting it; the next
    /// `download_with_progress` of the same info does not repeat it.
    pub fn announce_download(&self, info: &VideoInfo) {
        self.report_invoking(info);
        ANNOUNCED.with(|announced| announced.replace(Some(info.url.clone())));
    }

    fn report_invoking(&self, info: &VideoInfo) {
        let ffmpeg_fd = self.uses_ffmpeg_fd(info);
        if !(ffmpeg_fd || info.requested_formats.is_empty()) {
            return;
        }
        let name = if ffmpeg_fd {
            "ffmpeg"
        } else if info.fragments.is_empty() {
            "http"
        } else if info.protocol.as_deref() == Some("http_dash_segments") {
            "dashsegments"
        } else {
            "hlsnative"
        };
        let urls: Vec<&Url> = if info.requested_formats.is_empty() {
            vec![&info.url]
        } else {
            info.requested_formats
                .iter()
                .map(|format| &format.url)
                .collect()
        };
        report_invoking(name, &urls);
    }

    /// The configured ffmpeg executable.
    pub fn ffmpeg_path(&self) -> Option<&Path> {
        self.ffmpeg.as_deref()
    }

    pub fn has_ffmpeg(&self) -> bool {
        self.ffmpeg.is_some()
    }

    /// Configure an explicit ffprobe executable for audio codec inspection.
    /// Without this override, audio processing uses ffprobe beside explicit ffmpeg.
    pub fn with_ffprobe(mut self, executable: impl Into<PathBuf>) -> Self {
        self.ffprobe = Some(executable.into());
        self
    }

    /// Upstream's `prefer_best` default (`best/bestvideo+bestaudio`, no
    /// ffmpeg warning), used for `-o -` output.
    /// Upstream `quiet` for FFmpeg downloads (`-loglevel quiet`).
    pub fn with_external_quiet(mut self, quiet: bool) -> Self {
        self.ffmpeg_settings.quiet = quiet;
        self
    }

    /// `--http-chunk-size`: chunked HTTP downloads (overrides a format's
    /// `downloader_options`).
    pub fn with_http_chunk_size(mut self, size: Option<u64>) -> Self {
        self.http_chunk_size = size;
        self
    }

    /// `--use-extractors NAMES`: upstream names (`html5`, `youtube`,
    /// `generic`, `end`, regexes, `all`, `default`, `-` exclusions) select and
    /// order the extractors.
    pub fn with_allowed_extractors(
        mut self,
        names: &[String],
    ) -> std::result::Result<Self, String> {
        self.allowed_extractors = if names.is_empty() {
            None
        } else {
            Some(ordered_extractors(names, Vec::new())?)
        };
        Ok(self)
    }

    /// `--force-generic-extractor`: only the generic extractor is used.
    pub fn with_force_generic_extractor(mut self, force: bool) -> Self {
        self.force_generic = force;
        self
    }

    /// `-N`/`--concurrent-fragments`: fragments downloaded in parallel.
    pub fn with_concurrent_fragments(mut self, count: usize) -> Self {
        self.concurrent_fragments = count.max(1);
        self
    }

    /// `--enable-file-urls`: extract and download local `file:` media.
    pub fn with_file_urls(mut self, enable: bool) -> Self {
        self.enable_file_urls = enable;
        self
    }

    /// `--keep-fragments`: keep `TMPFILENAME-FragN` fragment files.
    pub fn with_keep_fragments(mut self, keep: bool) -> Self {
        self.keep_fragments = keep;
        self
    }

    /// `--downloader`: `ffmpeg` (FFmpegFD) or `native` per simplified
    /// protocol key (`http`, `ftp`, `m3u8`, `dash`, `rtmp`, `default`).
    pub fn with_downloader(mut self, protocol: &str, ffmpeg: bool) -> Self {
        self.downloaders.insert(protocol.to_owned(), ffmpeg);
        self
    }

    /// `--downloader-args NAME:ARGS` for FFmpegFD (`ffmpeg`, `ffmpeg_o`,
    /// `ffmpeg_oN`, `ffmpeg_i`, `ffmpeg_iN`, `default`).
    pub fn with_downloader_args(mut self, key: &str, args: Vec<String>) -> Self {
        self.ffmpeg_settings.args.insert(key.to_lowercase(), args);
        self
    }

    /// Upstream `get_suitable_downloader(...) == FFmpegFD`: partial downloads,
    /// or every selected protocol assigned to `ffmpeg` by `--downloader`.
    pub fn uses_ffmpeg_fd(&self, info: &VideoInfo) -> bool {
        if sections::is_partial(info) {
            return true;
        }
        // `FFmpegFD.can_download` needs ffmpeg; otherwise the native
        // downloaders are used.
        if self.ffmpeg.is_none() {
            return false;
        }
        let protocols: Vec<String> = if info.requested_formats.is_empty() {
            vec![
                info.protocol
                    .clone()
                    .unwrap_or_else(|| info.url.scheme().to_owned()),
            ]
        } else {
            info.requested_formats
                .iter()
                .map(VideoFormat::determine_protocol)
                .collect()
        };
        protocols.iter().all(|protocol| {
            self.downloaders
                .get(simplify_protocol(protocol))
                .or_else(|| self.downloaders.get("default"))
                .copied()
                .unwrap_or(false)
        })
    }

    /// Upstream `check_formats`.
    pub fn with_check_formats(mut self, check: CheckFormats) -> Self {
        self.check_formats = check;
        self
    }

    /// `--check-all-formats`: test every format, best first, before listing
    /// and selection, dropping those that fail (`process_video_result`).
    pub fn check_all_formats(&self, info: &mut VideoInfo) {
        if self.check_formats != CheckFormats::All {
            return;
        }
        let mut working = Vec::new();
        for format in info.formats.iter().rev() {
            screen::screen(&format!("[info] Testing format {}", format.format_id));
            if test_format(&self.client, info, format) {
                working.push(format.clone());
            } else {
                screen::screen(&format!(
                    "[info] Unable to download format {}. Skipping...",
                    format.format_id
                ));
            }
        }
        working.reverse();
        info.formats = working;
    }

    /// `--default-search PREFIX` for inputs without a URL scheme.
    pub fn with_default_search(mut self, prefix: Option<String>) -> Self {
        self.default_search = prefix;
        self
    }

    /// `--test`: download only the first 10241 bytes of HTTP media.
    pub fn with_test(mut self, test: bool) -> Self {
        self.test = test;
        self
    }

    /// `--hls-use-mpegts` for FFmpeg HLS downloads.
    pub fn with_hls_use_mpegts(mut self, mpegts: bool) -> Self {
        self.ffmpeg_settings.hls_use_mpegts = mpegts;
        self
    }

    pub fn with_prefer_best_format(mut self, prefer_best: bool) -> Self {
        self.prefer_best = prefer_best;
        self
    }

    /// Sort formats like upstream with user fields and flags.
    pub fn with_format_sort(mut self, options: FormatSortOptions) -> Self {
        self.format_sort = options;
        self
    }

    /// Upstream `process_video_result` format processing for every video of
    /// an extracted or loaded result: common fields, thumbnails, per-format
    /// fills, `FormatSorter` order (worst first) and format ID cleanup.
    pub fn process_result(&self, result: &mut ExtractResult) -> Result<()> {
        let entries = match result {
            ExtractResult::Video(video) => std::slice::from_mut(video),
            ExtractResult::Playlist(playlist) => playlist.entries.as_mut_slice(),
        };
        for entry in entries {
            entry.fulltitle = Some(entry.title.clone());
            entry.metadata.fill_common_fields();
            entry.metadata.fill_chapters();
            entry.metadata.sanitize_thumbnails();
            info::fill_format_fields(&mut entry.formats);
            let extractor: Vec<&str> = entry
                .format_sort_fields
                .iter()
                .map(String::as_str)
                .collect();
            let sorter = format_sort::FormatSorter::new(
                &self.format_sort.fields,
                &extractor,
                self.format_sort.force,
                self.format_sort.prefer_free,
            )
            .map_err(|error| {
                Error::FormatSelection(format!(
                    "Invalid format sort string \"{}\" given by extractor",
                    error.0
                ))
            })?;
            // Upstream sorts (and reports the order) only with formats.
            if !entry.formats.is_empty() && screen::verbose() {
                for line in sorter.verbose_info() {
                    screen::debug(&line);
                }
            }
            info::sort_formats(&mut entry.formats, &sorter);
            info::sanitize_format_ids(&mut entry.formats);
            info::fill_format_descriptions(&mut entry.formats);
        }
        Ok(())
    }

    /// Upstream `--merge-output-format`: ordered containers that a merge may
    /// use, passed to `get_compatible_ext` as preferences.
    pub fn with_merge_output_format(mut self, formats: Vec<String>) -> Self {
        self.merge_output_format = Some(formats);
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
        // Upstream `_default_format_spec` `prefer_best` (stdout output).
        let format = format.or(self.prefer_best.then_some("best/bestvideo+bestaudio"));
        result.select_format_with_merge(
            format,
            self.ffmpeg.is_some(),
            self.merge_output_format.as_deref(),
        )
    }

    /// Select formats for one video or playlist entry (upstream
    /// `process_video_result`), leaving it unchanged on failure.
    pub fn select_video(&self, info: &mut VideoInfo, format: Option<&str>) -> Result<()> {
        if format.is_none() && self.prefer_best {
            screen::debug("Default format spec: best/bestvideo+bestaudio");
        }
        let format = format.or(self.prefer_best.then_some("best/bestvideo+bestaudio"));
        if self.check_formats != CheckFormats::Selected {
            return info.select_format_with_merge(
                format,
                self.ffmpeg.is_some(),
                self.merge_output_format.as_deref(),
            );
        }
        // Upstream `_check_formats` for `selected`: each candidate is tested
        // once, in preference order, as selection reaches it.
        let client = self.client.clone();
        let snapshot = info.clone();
        let mut tested: std::collections::HashMap<String, bool> = Default::default();
        let check = Box::new(move |format: &VideoFormat| {
            *tested.entry(format.format_id.clone()).or_insert_with(|| {
                screen::screen(&format!("[info] Testing format {}", format.format_id));
                let works = test_format(&client, &snapshot, format);
                if !works {
                    screen::warning(&format!(
                        "Unable to download format {}. Skipping...",
                        format.format_id
                    ));
                }
                works
            })
        });
        info::with_format_check(check, || {
            info.select_format_with_merge(
                format,
                self.ffmpeg.is_some(),
                self.merge_output_format.as_deref(),
            )
        })
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
        if !has_scheme(input) {
            return self.extract_unqualified(input, options);
        }
        let url = Url::parse(input)?;
        if url.scheme() == "file" {
            return self.extract_file(&url);
        }
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
                    retry::backoff(u32::from(attempt), delay);
                }
            }
        }
        unreachable!("the final attempt returns success or an error")
    }

    /// The extractor names in matching order (`None`: the registry order).
    fn extractor_order(&self) -> Option<Vec<&'static str>> {
        if self.force_generic {
            return Some(vec!["generic"]);
        }
        self.allowed_extractors.clone()
    }

    fn generic_allowed(&self) -> bool {
        self.extractor_order()
            .is_none_or(|order| order.contains(&"generic"))
    }

    fn no_suitable(&self, url: &str) -> Error {
        let key = if self.force_generic { " (Generic)" } else { "" };
        Error::Reported(format!("No suitable extractor{key} found for URL {url}"))
    }

    /// Upstream generic extraction of a `file:` URL, refused unless
    /// `--enable-file-urls` (urllib's file handler then reads it).
    fn extract_file(&self, url: &Url) -> Result<ExtractResult> {
        if !self.generic_allowed() {
            return Err(self.no_suitable(url.as_str()));
        }
        screen::set_extractor("generic");
        screen::screen(&format!(
            "[generic] Extracting URL: {}",
            screen::extracting_url(url.as_str())
        ));
        screen::screen(&format!(
            "[generic] {}: Downloading webpage",
            screen::generic_id(url.as_str())
        ));
        if !self.enable_file_urls {
            return Err(Error::Reported(
                "file:// URLs are disabled by default in yt-dlp for security reasons. Use --enable-file-urls to enable at your own risk."
                    .into(),
            ));
        }
        let mut result = ExtractResult::Video(extractor::direct::from_file(url)?);
        self.process_result(&mut result)?;
        Ok(result)
    }

    /// Upstream `GenericIE._real_extract` for an input without a scheme.
    fn extract_unqualified(
        &self,
        input: &str,
        options: ExtractionOptions,
    ) -> Result<ExtractResult> {
        if !self.generic_allowed() {
            return Err(self.no_suitable(input));
        }
        screen::screen(&format!(
            "[generic] Extracting URL: {}",
            screen::extracting_url(input)
        ));
        let search = self.default_search.as_deref().unwrap_or("fixup_error");
        static DOMAIN: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"^[^\s/]+\.[^\s/]+/").unwrap());
        if matches!(search, "auto" | "auto_warning" | "fixup_error") && DOMAIN.is_match(input) {
            screen::warning("[generic] The url doesn't specify the protocol, trying with https");
            return self
                .extract_result_unselected_with_options(&format!("https://{input}"), options);
        }
        let quoted = output::traverse::py_repr_str(input);
        if matches!(search, "error" | "fixup_error") {
            return Err(Error::Reported(format!(
                "[generic] {quoted} is not a valid URL"
            )));
        }
        Err(Error::Reported(format!(
            "[generic] {quoted}: search extractors ({search}) are not supported natively"
        )))
    }

    fn extract_once(&self, url: &Url) -> Result<ExtractResult> {
        let order = self.extractor_order();
        let extractors: Vec<&Box<dyn Extractor>> = match &order {
            None => self.extractors.iter().collect(),
            Some(order) => {
                let mut selected = Vec::new();
                for name in order {
                    if *name == "end" {
                        break;
                    }
                    selected.extend(
                        self.extractors
                            .iter()
                            .filter(|extractor| extractor.key().eq_ignore_ascii_case(name)),
                    );
                }
                selected
            }
        };
        for extractor in extractors {
            if let Some(mut info) = extractor.extract_result(&self.client, url)? {
                self.process_result(&mut info)?;
                return Ok(info);
            }
        }
        // Upstream `UnsupportedURLIE` (`end`) or no matching extractor.
        match &order {
            None => return Err(Error::UnsupportedUrl(url.to_string())),
            Some(order) if order.contains(&"end") => {
                screen::screen(&format!(
                    "[UnsupportedURL] Extracting URL: {}",
                    screen::extracting_url(url.as_str())
                ));
                return Err(Error::Reported(format!("Unsupported URL: {url}")));
            }
            Some(_) => {}
        }
        Err(self.no_suitable(url.as_str()))
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
        // Upstream `get_suitable_downloader`: partial downloads and
        // `--downloader ffmpeg` use FFmpegFD, which also merges selected
        // formats directly.
        let ffmpeg_fd = self.uses_ffmpeg_fd(info);
        let announced = ANNOUNCED.with(|announced| announced.take());
        if announced.as_ref() != Some(&info.url) {
            self.report_invoking(info);
        }
        if ffmpeg_fd {
            let ffmpeg = self.ffmpeg.as_deref().ok_or_else(|| {
                Error::Reported(
                    "You have requested downloading the video partially, but ffmpeg is not installed. Aborting"
                        .into(),
                )
            })?;
            let network = ffmpeg_fd::Network {
                headers: &self.headers,
                cookies: &self.cookies,
                proxy: self.proxy.as_deref(),
            };
            return ffmpeg_fd::download(
                ffmpeg,
                &network,
                &self.ffmpeg_settings,
                info,
                destination.as_ref(),
                &options,
                &mut callback,
            );
        }
        if !info.requested_formats.is_empty() {
            return self.download_merged(info, destination.as_ref(), options, &mut callback);
        }
        if !info.fragments.is_empty() {
            return fragment::download(
                &self.client,
                info,
                destination.as_ref(),
                options,
                self.concurrent_fragments,
                self.keep_fragments,
                &mut callback,
            );
        }
        // Upstream `HttpFD`: `_TEST_FILE_SIZE` for `--test`, else the
        // `http_chunk_size` parameter, else the format's downloader options.
        let chunk = if self.test {
            Some(download::Chunking {
                size: download::TEST_FILE_SIZE,
                test: true,
            })
        } else {
            self.http_chunk_size
                .or_else(|| info.downloader_options.as_ref()?.http_chunk_size)
                .filter(|size| *size > 0)
                .map(|size| download::Chunking { size, test: false })
        };
        if info.url.scheme() == "file" {
            return download::download_file(info, destination.as_ref(), options, &mut callback);
        }
        download::download(
            &self.client,
            info,
            destination.as_ref(),
            options,
            chunk,
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
        report_invoking("http", &[&subtitle.url]);
        download::download_subtitle(
            &self.client,
            info,
            subtitle,
            destination.as_ref(),
            options,
            &mut callback,
        )
    }

    /// Convert a local thumbnail image with the explicit FFmpeg (upstream
    /// `FFmpegThumbnailsConvertorPP.convert_thumbnail`).
    pub fn convert_thumbnail(&self, input: &Path, output: &Path, target: &str) -> Result<()> {
        let ffmpeg = self.ffmpeg.as_ref().ok_or_else(|| {
            Error::Postprocess(
                "thumbnail conversion requires an explicit --ffmpeg-location PATH".into(),
            )
        })?;
        thumbnail_process::convert(ffmpeg, input, output, target, &mut || {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Copy a thumbnail URL to `destination` without overwriting it.
    pub fn download_thumbnail(
        &self,
        url: &str,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
    ) -> Result<u64> {
        download::download_thumbnail(
            &self.client,
            &Url::parse(url)?,
            destination.as_ref(),
            options,
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
        download::check_destination(destination, &options)?;
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
        if !info::mergeable(&streams[video], &streams[audio]) {
            return Err(Error::Postprocess("unsupported stream containers".into()));
        }
        // The selected container (upstream `get_compatible_ext`).
        let output_ext = info.ext.as_str();
        let upstream_files = matches!(options.merge_files, download::MergeFiles::Upstream { .. });
        let executable = match &self.ffmpeg {
            Some(executable) => Some(executable),
            // Upstream downloads the formats without merging (after warning).
            None if upstream_files => None,
            None => {
                return Err(Error::Postprocess(
                    "separate streams require --ffmpeg-location PATH or YoutubeDl::with_ffmpeg"
                        .into(),
                ));
            }
        };
        let executable = executable
            .map(|executable| executable.canonicalize())
            .transpose()
            .map_err(|error| {
                Error::Postprocess(format!("invalid ffmpeg executable path: {error}"))
            })?;
        if executable
            .as_ref()
            .is_some_and(|executable| !executable.is_file())
        {
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
        let upstream = match options.merge_files {
            download::MergeFiles::Private => None,
            download::MergeFiles::Upstream { keep } => Some(keep),
        };
        let mut paths = Vec::new();
        let mut stored = 0u64;
        for (index, stream) in streams.iter().enumerate() {
            let path = match upstream {
                None => workspace
                    .path()
                    .join(format!("input-{index}.{}", stream.ext)),
                Some(_) => stream_path(destination, &info.ext, stream),
            };
            if upstream.is_some() && path.exists() {
                // Upstream FileDownloader.download reuses a complete file.
                screen::screen(&format!(
                    "[download] {} has already been downloaded",
                    path.display()
                ));
                let bytes = fs::metadata(&path)?.len();
                stored = stored.saturating_add(bytes);
                paths.push(path);
                if callback(DownloadProgress {
                    attempt: 1,
                    downloaded_bytes: bytes,
                    total_bytes: Some(bytes),
                    phase: DownloadPhase::Publishing,
                    stream_index: Some(index as u8 + 1),
                    stream_count: Some(2),
                    fragment_index: None,
                    fragment_count: None,
                })
                .is_break()
                {
                    return Err(Error::DownloadCancelled);
                }
                continue;
            }
            let mut part = info.clone();
            part.fragments = stream.fragments.clone();
            part.requested_formats.clear();
            part.formats.clear();
            part.url = stream.url.clone();
            part.ext = stream.ext.clone();
            part.protocol = Some(stream.determine_protocol());
            part.stream_kind = stream.stream_kind;
            part.format_id = Some(stream.format_id.clone());
            match stream.format_index {
                Some(index) => {
                    part.extra.insert("format_index".into(), index.into());
                }
                None => {
                    part.extra.remove("format_index");
                }
            }
            part.filesize = stream.filesize;
            part.width = stream.width;
            part.height = stream.height;
            if upstream.is_some() {
                self.announce_download(&part);
                // Upstream `HlsFD` refetches each stream's media playlist.
                if let Some(refreshed) = self.refresh_hls(&part)? {
                    part = refreshed;
                }
                if !part.fragments.is_empty() {
                    let downloader = match part.protocol.as_deref() {
                        Some("http_dash_segments") => "dashsegments",
                        _ => "hlsnative",
                    };
                    screen::screen(&format!(
                        "[{downloader}] Total fragments: {}",
                        total_fragments(&part)
                    ));
                }
                screen::screen(&format!("[download] Destination: {}", path.display()));
            }
            let bytes = self.download_with_progress(
                &part,
                &path,
                DownloadOptions {
                    part_files: if upstream.is_some() {
                        options.part_files
                    } else {
                        PartFiles::Private
                    },
                    ..options
                },
                |mut progress: DownloadProgress| {
                    // Private inputs are not published; upstream-style ones are.
                    if progress.phase == DownloadPhase::Publishing && upstream.is_none() {
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
        let Some(executable) = executable else {
            // No merger: the format files are the result.
            return Ok(stored);
        };
        if upstream.is_some() {
            screen::screen(&format!(
                "[Merger] Merging formats into \"{}\"",
                destination.display()
            ));
        }
        let output = tempfile::Builder::new()
            .prefix("merged-")
            .suffix(".part")
            .tempfile_in(workspace.path())?
            .into_temp_path();
        let oldest = filetime::oldest(paths.iter().map(PathBuf::as_path));
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
        filetime::inherit(&output, oldest);
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
        download::persist_path(output, destination, options.overwrite)?;
        if upstream == Some(false) {
            for path in &paths {
                screen::screen(&format!(
                    "Deleting original file {} (pass -k to keep)",
                    path.display()
                ));
                fs::remove_file(path)?;
            }
        }
        Ok(bytes)
    }
}

/// Upstream's per-format merge input name: `correct_ext` of the destination
/// for the stream's extension, then `prepend_extension` with `fFORMAT_ID`.
fn stream_path(destination: &Path, merged_ext: &str, stream: &VideoFormat) -> PathBuf {
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && (ext == merged_ext || ext == stream.ext) => {
            stem.to_owned()
        }
        _ => name,
    };
    destination.with_file_name(format!("{stem}.f{}.{}", stream.format_id, stream.ext))
}
