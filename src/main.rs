use clap::{CommandFactory, FromArgMatches, Parser};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    ops::ControlFlow,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use yt_dlp_rs::{
    AudioOptions, CookieJar, DownloadArchive, DownloadOptions, DownloadPhase, ExtractResult,
    ExtractionOptions, HttpOptions, InfoJsonWrite, MatchFilters, MatchOutcome, PlaylistCounts,
    PlaylistSelection, PostprocessOptions, SubtitleConversion, SubtitleEmbedding, SubtitleFile,
    SubtitleOptions, VideoMode, VideoOptions, YoutubeDl, YoutubeOptions,
    output::{OutputTemplates, PrintTemplate, PrintToFile, TemplateOptions, render_formats},
    playlist_info_json, render_automatic_captions, render_subtitles, subtitle_paths,
    validate_format_selector, video_info_json, write_info_json_with,
};

/// `-q`: suppress informational `[...]` screen lines (upstream `to_screen`).
static QUIET: AtomicBool = AtomicBool::new(false);
/// `--no-warnings`: suppress `WARNING:` lines.
static NO_WARNINGS: AtomicBool = AtomicBool::new(false);

macro_rules! screen {
    ($($arg:tt)*) => {
        if !QUIET.load(Ordering::Relaxed) {
            eprintln!($($arg)*);
        }
    };
}

macro_rules! warning {
    ($($arg:tt)*) => {
        if !NO_WARNINGS.load(Ordering::Relaxed) {
            eprintln!($($arg)*);
        }
    };
}

#[derive(Parser)]
#[command(
    group(clap::ArgGroup::new("processing").args(["extract_audio", "remux_video", "recode_video", "convert_subs", "embed_subs"]).multiple(true)),
    version,
    about = "Native Rust yt-dlp adaptation (direct HTTP, HTML5 and HLS/DASH VOD)"
)]
struct Args {
    #[arg(required_unless_present = "batch_file")]
    urls: Vec<String>,
    /// Read URLs from FILE ("-" for stdin), one per line; "#", ";" and "]" lines are comments
    #[arg(short = 'a', long, value_name = "FILE", overrides_with_all = ["batch_file", "no_batch_file"])]
    batch_file: Option<String>,
    /// Do not read URLs from a batch file (default)
    #[arg(long, overrides_with = "batch_file")]
    no_batch_file: bool,
    /// Print one JSON info object per video (including playlist entries) without downloading
    #[arg(short = 'j', long)]
    dump_json: bool,
    /// Print one JSON object per URL; playlists include their selected entries (implies simulation)
    #[arg(short = 'J', long)]
    dump_single_json: bool,
    /// Activate quiet mode: suppress informational messages
    #[arg(short = 'q', long, overrides_with_all = ["quiet", "no_quiet"])]
    quiet: bool,
    /// Deactivate quiet mode (default unless printing or JSON output imply it)
    #[arg(long, overrides_with_all = ["quiet", "no_quiet"])]
    no_quiet: bool,
    /// Ignore warnings
    #[arg(long)]
    no_warnings: bool,
    /// Extract metadata without writing media
    #[arg(short = 's', long, overrides_with_all = ["simulate", "no_simulate"])]
    simulate: bool,
    /// Download even when printing, JSON or listing options would simulate
    #[arg(long, overrides_with_all = ["simulate", "no_simulate"])]
    no_simulate: bool,
    /// Print a video field list or output template per video, optionally prefixed with video: (implies simulation; repeatable)
    #[arg(
        short = 'O',
        long = "print",
        value_name = "[WHEN:]TEMPLATE",
        allow_hyphen_values = true
    )]
    print: Vec<String>,
    /// Append a video-stage print template to a file named by an output template (repeatable; does not imply simulation)
    #[arg(
        long,
        num_args = 2,
        value_names = ["[WHEN:]TEMPLATE", "FILE"],
        allow_hyphen_values = true
    )]
    print_to_file: Vec<String>,
    #[arg(short = 'g', long, hide = true)]
    get_url: bool,
    #[arg(short = 'e', long, hide = true)]
    get_title: bool,
    #[arg(long, hide = true)]
    get_id: bool,
    #[arg(long, hide = true)]
    get_filename: bool,
    /// List available formats (selection still applies with -j or -s; no downloads)
    #[arg(short = 'F', long = "list-formats")]
    list_formats: bool,
    /// List available subtitle languages/formats without downloading
    #[arg(long)]
    list_subs: bool,
    /// Write selected subtitle sidecars before downloading media
    #[arg(long, alias = "write-srt", overrides_with = "no_write_subs")]
    write_subs: bool,
    /// Disable subtitle sidecar writing
    #[arg(long, alias = "no-write-srt", overrides_with = "write_subs")]
    no_write_subs: bool,
    /// Write selected automatically generated caption sidecars
    #[arg(
        long,
        alias = "write-automatic-subs",
        overrides_with = "no_write_auto_subs"
    )]
    write_auto_subs: bool,
    /// Disable automatic caption sidecar writing
    #[arg(
        long,
        alias = "no-write-automatic-subs",
        overrides_with = "write_auto_subs"
    )]
    no_write_auto_subs: bool,
    /// Comma-separated literal subtitle languages, all and -exclusions (no regex)
    #[arg(
        long,
        alias = "srt-langs",
        value_name = "LANGS",
        allow_hyphen_values = true
    )]
    sub_langs: Vec<String>,
    /// Ordered subtitle extension preferences separated by / (default best)
    #[arg(long, default_value = "best", value_name = "FORMAT")]
    sub_format: String,
    /// Embed selected subtitles after audio/video processing (implies manual writing unless auto writing is enabled)
    #[arg(long, overrides_with = "no_embed_subs")]
    embed_subs: bool,
    /// Disable subtitle embedding
    #[arg(long, overrides_with = "embed_subs")]
    no_embed_subs: bool,
    /// Convert written subtitles to srt, vtt, ass or lrc; none disables conversion
    #[arg(long, aliases = ["convert-sub", "convert-subtitles"], value_name = "FORMAT", overrides_with = "convert_subs")]
    convert_subs: Option<String>,
    /// Skip media downloads while allowing requested subtitle and info JSON sidecars
    #[arg(long, alias = "no-download")]
    skip_download: bool,
    /// Write cleaned video and playlist metadata to .info.json files (existing files are kept)
    #[arg(long, overrides_with = "no_write_info_json")]
    write_info_json: bool,
    /// Disable info JSON writing
    #[arg(long, overrides_with = "write_info_json")]
    no_write_info_json: bool,
    /// Format ID or indexed/grouped selector with filters, + pairs and / fallbacks
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
    /// Retry transient download failures this many times (0 disables retries)
    #[arg(long, default_value_t = 3)]
    retries: u8,
    /// Retry the entire extraction after transient network failures (0 disables retries)
    #[arg(long, default_value_t = 0)]
    extractor_retries: u8,
    /// Seconds to wait for connections and each read/write before giving up
    #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
    socket_timeout: Option<f64>,
    /// Client-side IP address to bind to
    #[arg(long, value_name = "IP", overrides_with_all = ["source_address", "force_ipv4", "force_ipv6"])]
    source_address: Option<String>,
    /// Make all connections via IPv4
    #[arg(short = '4', long, overrides_with_all = ["source_address", "force_ipv4", "force_ipv6"])]
    force_ipv4: bool,
    /// Make all connections via IPv6
    #[arg(short = '6', long, overrides_with_all = ["source_address", "force_ipv4", "force_ipv6"])]
    force_ipv6: bool,
    /// Suppress HTTPS certificate validation
    #[arg(long)]
    no_check_certificates: bool,
    /// HTTP/HTTPS/SOCKS proxy URL; an empty value forces direct connections
    #[arg(long, value_name = "URL", overrides_with = "proxy")]
    proxy: Option<String>,
    /// Read and save scoped cookies in Netscape format (including metadata modes)
    #[arg(long, value_name = "FILE", overrides_with_all = ["no_cookies", "cookies"])]
    cookies: Option<PathBuf>,
    /// Disable cookie-file loading and saving (in-memory response cookies remain enabled)
    #[arg(long, overrides_with = "cookies")]
    no_cookies: bool,
    /// Custom HTTP header FIELD:VALUE (repeatable; last name wins)
    #[arg(long, alias = "add-header", value_name = "FIELD:VALUE")]
    add_headers: Vec<String>,
    /// Override the HTTP User-Agent
    #[arg(long, value_name = "UA", overrides_with = "user_agent")]
    user_agent: Option<String>,
    /// Default HTTP Referer (extractor request headers can override it)
    #[arg(long, value_name = "URL", overrides_with = "referer")]
    referer: Option<String>,
    /// Explicit path to Deno 2.9.7 for YouTube player challenges (no runtime discovery or installation)
    #[arg(long, value_name = "PATH")]
    youtube_js_runtime: Option<PathBuf>,
    /// Extract audio with local ffmpeg/ffprobe after downloading
    #[arg(short = 'x', long)]
    extract_audio: bool,
    /// Audio target or ordered SOURCE>TARGET/TARGET rules
    #[arg(long, default_value = "best", value_name = "FORMAT")]
    audio_format: String,
    /// VBR quality 0..10 or bitrate such as 128K
    #[arg(
        long,
        default_value = "5",
        value_name = "QUALITY",
        allow_hyphen_values = true
    )]
    audio_quality: String,
    /// Remux media to FORMAT or ordered SOURCE>TARGET/TARGET rules (stream copy)
    #[arg(long, value_name = "FORMAT", overrides_with = "remux_video")]
    remux_video: Option<String>,
    /// Re-encode media to FORMAT or ordered SOURCE>TARGET/TARGET rules
    #[arg(long, value_name = "FORMAT", overrides_with = "recode_video")]
    recode_video: Option<String>,
    /// Keep downloaded source and completed processing intermediates
    #[arg(
        short = 'k',
        long,
        requires = "processing",
        overrides_with = "no_keep_video"
    )]
    keep_video: bool,
    /// Remove processing sources after success (default)
    #[arg(long, overrides_with = "keep_video")]
    no_keep_video: bool,
    /// Explicit ffprobe executable for audio inspection (otherwise beside ffmpeg)
    #[arg(long, value_name = "PATH")]
    ffprobe_location: Option<PathBuf>,
    /// Explicit ffmpeg executable for local stream merging and audio conversion
    #[arg(long, value_name = "PATH")]
    ffmpeg_location: Option<PathBuf>,
    /// Skip recorded extractor/video IDs and record only completed downloads
    #[arg(long, value_name = "FILE", overrides_with_all = ["download_archive", "no_download_archive"])]
    download_archive: Option<PathBuf>,
    /// Do not use an archive file (default)
    #[arg(long, overrides_with_all = ["download_archive", "no_download_archive"])]
    no_download_archive: bool,
    /// Stop when encountering a video that is in the download archive
    #[arg(long, overrides_with_all = ["break_on_existing", "no_break_on_existing"])]
    break_on_existing: bool,
    /// Do not stop at archived videos (default)
    #[arg(long, overrides_with_all = ["break_on_existing", "no_break_on_existing"])]
    no_break_on_existing: bool,
    /// Make --max-downloads and --break-on-existing reset per input URL
    #[arg(long, overrides_with_all = ["break_per_input", "no_break_per_input"])]
    break_per_input: bool,
    /// Let --max-downloads and --break-on-existing stop the whole queue (default)
    #[arg(long, overrides_with_all = ["break_per_input", "no_break_per_input"])]
    no_break_per_input: bool,
    /// Record archive entries even when simulating or skipping downloads
    #[arg(long, aliases = ["force-write-download-archive", "force-download-archive"])]
    force_write_archive: bool,
    /// Comma-separated playlist indices or inclusive START:STOP:STEP ranges
    #[arg(
        short = 'I',
        long,
        value_name = "ITEM_SPEC",
        allow_hyphen_values = true
    )]
    playlist_items: Option<String>,
    /// First playlist item (one-based, default 1; overridden by --playlist-items)
    #[arg(long, default_value_t = 1, allow_hyphen_values = true)]
    playlist_start: i64,
    /// Last playlist item, inclusive (-1 means through the end)
    #[arg(long, allow_hyphen_values = true)]
    playlist_end: Option<i64>,
    /// Reverse the selected playlist item order
    #[arg(long)]
    playlist_reverse: bool,
    /// Placeholder for unavailable output-template metadata
    #[arg(long, default_value = "NA", value_name = "TEXT")]
    output_na_placeholder: String,
    /// Filename template: represented metadata with s/d/i/u/f/F conversions and %%
    #[arg(short, long, value_name = "[TYPES:]TEMPLATE")]
    output: Vec<String>,
    /// Do not overwrite any files
    #[arg(short = 'w', long, overrides_with_all = ["no_overwrites", "force_overwrites", "no_force_overwrites"])]
    no_overwrites: bool,
    /// Overwrite all video and metadata files
    #[arg(long, alias = "yes-overwrites", overrides_with_all = ["no_overwrites", "force_overwrites", "no_force_overwrites"])]
    force_overwrites: bool,
    /// Do not overwrite the video, but overwrite related files (default)
    #[arg(long, overrides_with_all = ["no_overwrites", "force_overwrites", "no_force_overwrites"])]
    no_force_overwrites: bool,
    /// Generic video filter: FIELD comparisons/presence joined by &; repeat for OR
    #[arg(long, value_name = "FILTER", allow_hyphen_values = true)]
    match_filters: Vec<String>,
    /// Discard earlier --match-filters
    #[arg(long, action = clap::ArgAction::Count)]
    no_match_filters: u8,
    /// Like --match-filters, but stop the download process when a video is rejected
    #[arg(long, value_name = "FILTER", allow_hyphen_values = true)]
    break_match_filters: Vec<String>,
    /// Discard earlier --break-match-filters
    #[arg(long, action = clap::ArgAction::Count)]
    no_break_match_filters: u8,
    /// Abort after processing NUMBER videos (0 means no limit)
    #[arg(long, value_name = "NUMBER", allow_hyphen_values = true)]
    max_downloads: Option<i64>,
    /// Maximum download rate in bytes per second, e.g. 50K or 4.2M
    #[arg(short = 'r', long, aliases = ["rate-limit"], value_name = "RATE")]
    limit_rate: Option<String>,
    /// Seconds to sleep before each download (the minimum with --max-sleep-interval)
    #[arg(
        long,
        alias = "min-sleep-interval",
        value_name = "SECONDS",
        allow_hyphen_values = true
    )]
    sleep_interval: Option<f64>,
    /// Maximum seconds to sleep before each download; requires --min-sleep-interval
    #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
    max_sleep_interval: Option<f64>,
    /// Seconds to sleep before each subtitle download
    #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
    sleep_subtitles: Option<f64>,
    /// Skip media downloads smaller than SIZE, e.g. 50k or 44.6M
    #[arg(long, value_name = "SIZE")]
    min_filesize: Option<String>,
    /// Skip media downloads larger than SIZE, e.g. 50k or 44.6M
    #[arg(long, value_name = "SIZE")]
    max_filesize: Option<String>,
    /// Output directories: [TYPES:]PATH with home (default) or an output type such as subtitle/infojson
    #[arg(short = 'P', long, value_name = "[TYPES:]PATH")]
    paths: Vec<String>,
    /// Restrict filenames to ASCII and avoid "&" and spaces
    #[arg(long, overrides_with = "no_restrict_filenames")]
    restrict_filenames: bool,
    /// Allow Unicode characters, "&" and spaces in filenames (default)
    #[arg(long, overrides_with = "restrict_filenames")]
    no_restrict_filenames: bool,
    /// Force Windows-compatible filenames
    #[arg(long, overrides_with = "no_windows_filenames")]
    windows_filenames: bool,
    /// Sanitize filenames only minimally (outside Windows)
    #[arg(long, overrides_with = "windows_filenames")]
    no_windows_filenames: bool,
    /// Limit the filename length (excluding extension) to LENGTH characters
    #[arg(
        long,
        alias = "trim-file-names",
        value_name = "LENGTH",
        default_value_t = 0,
        allow_hyphen_values = true
    )]
    trim_filenames: i64,
}

/// Upstream `simulate`/`quiet` resolution (`__init__` and `YoutubeDL.__init__`).
#[derive(Debug, PartialEq, Eq)]
struct Simulation {
    simulate: bool,
    /// `simulate == 'list_only'`: listing without format selection.
    list_only: bool,
    quiet: bool,
}

impl Args {
    fn simulation(&self) -> Simulation {
        // "any getting" and video-stage print templates imply quiet simulation.
        let getting = self.dump_json
            || self.dump_single_json
            || !self.print.is_empty()
            || self.get_title
            || self.get_id
            || self.get_url
            || self.get_filename;
        let listing = self.list_formats || self.list_subs;
        let explicit = if self.simulate {
            Some(true)
        } else if self.no_simulate {
            Some(false)
        } else {
            None
        };
        let quiet = if self.quiet {
            Some(true)
        } else if self.no_quiet {
            Some(false)
        } else {
            None
        };
        Simulation {
            simulate: explicit.unwrap_or(getting || listing),
            list_only: explicit.is_none() && !getting && listing,
            quiet: quiet.unwrap_or(getting),
        }
    }
}

/// Append one UTF-8 line, creating parent directories like upstream.
fn append_line(path: &std::path::Path, line: &str) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(format!("{line}\n").as_bytes())
}

/// The media filename (`prepare_filename(info)`); the default template is never empty.
fn prepare_media(
    templates: &OutputTemplates,
    info: &yt_dlp_rs::VideoInfo,
    options: &TemplateOptions,
) -> yt_dlp_rs::Result<PathBuf> {
    Ok(templates
        .prepare(info, None, options)?
        .expect("default output template is non-empty"))
}

/// Upstream warns when an absolute media template makes `--paths` irrelevant.
fn path_is_absolute_template(templates: &OutputTemplates) -> bool {
    std::path::Path::new(&yt_dlp_rs::batch::expand_path_text(templates.get(None))).is_absolute()
}

/// Whether a regular file exists; other existing entries (including dangling
/// symlinks, which upstream would write through) are refused natively.
fn path_exists(path: &std::path::Path) -> yt_dlp_rs::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) if std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) => Ok(true),
        Ok(_) => Err(yt_dlp_rs::Error::OutputExists(path.into())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// The `-J` object for one URL: the `-j` video object, or the playlist with
/// `_type`, selected counts/indices and every selected entry.
fn single_json(
    result: &ExtractResult,
    counts: Option<PlaylistCounts>,
) -> serde_json::Result<serde_json::Value> {
    match result {
        ExtractResult::Video(info) => serde_json::to_value(info),
        ExtractResult::Playlist(playlist) => {
            let mut value = serde_json::to_value(playlist)?;
            let object = value
                .as_object_mut()
                .expect("playlist serializes to an object");
            let entries = object.remove("entries");
            object.insert("_type".into(), "playlist".into());
            if let Some(counts) = counts {
                object.insert("playlist_count".into(), counts.playlist_count.into());
            }
            let indices: Vec<_> = playlist
                .entries
                .iter()
                .map(|info| info.playlist_index)
                .collect();
            object.insert("requested_entries".into(), serde_json::to_value(indices)?);
            object.insert("entries".into(), entries.unwrap_or_default());
            Ok(value)
        }
    }
}

/// Python `str(float)` for ordinary values (`1.0`, `0.5`, `nan`, `inf`).
fn py_float(value: f64) -> String {
    if value.is_nan() {
        "nan".into()
    } else if value.is_infinite() {
        if value > 0.0 { "inf" } else { "-inf" }.into()
    } else if value.fract() == 0.0 && value.abs() < 1e16 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

/// Upstream download/subtitle sleep intervals (`__init__` validation and
/// `FileDownloader.download`).
struct SleepPolicy {
    min: f64,
    max: f64,
    subtitles: f64,
}

impl SleepPolicy {
    fn from_cli(
        min: Option<f64>,
        max: Option<f64>,
        subtitles: Option<f64>,
    ) -> Result<Self, String> {
        for (name, value) in [
            ("subtitles sleep interval", subtitles),
            ("sleep interval", min),
            ("max sleep interval", max),
        ] {
            if let Some(value) = value
                && !(value >= 0.0 && value.is_finite())
            {
                return Err(format!(
                    "{name} \"{}\" must be positive or 0",
                    py_float(value)
                ));
            }
        }
        let max = match (min, max) {
            (None, Some(_)) => {
                return Err(
                    "min sleep interval must be specified; use --min-sleep-interval".into(),
                );
            }
            (Some(min), Some(max)) if max < min => {
                return Err(format!(
                    "max sleep interval \"{}\" must be must be greater than or equal to min sleep interval \"{}\"",
                    py_float(max),
                    py_float(min)
                ));
            }
            (min, max) => max.or(min).unwrap_or(0.0),
        };
        Ok(Self {
            min: min.unwrap_or(0.0),
            max,
            subtitles: subtitles.unwrap_or(0.0),
        })
    }

    fn pause(seconds: f64) {
        if seconds > 0.0 {
            screen!("[download] Sleeping {seconds:.2} seconds ...");
            std::thread::sleep(Duration::from_secs_f64(seconds));
        }
    }

    /// `random.uniform(min, max or min)`.
    fn media(&self) {
        let max = if self.max == 0.0 { self.min } else { self.max };
        let unit = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos()) as f64
            / 1e9;
        Self::pause(self.min + (max - self.min) * unit);
    }

    fn subtitle(&self) {
        Self::pause(self.subtitles);
    }
}

/// Upstream `parse_bytes`: `NUMBER[.DIGITS][ ]UNIT` with binary K..Y units,
/// rounded half to even. Non-ASCII digits are not accepted natively.
fn parse_bytes(value: &str) -> Option<u64> {
    let upper = value.to_uppercase();
    let digits = upper
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(upper.len());
    if digits == 0 {
        return None;
    }
    let mut end = digits;
    if let Some(fraction) = upper[end..].strip_prefix('.') {
        let length = fraction
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(fraction.len());
        if length == 0 {
            return None;
        }
        end += 1 + length;
    }
    let number: f64 = upper[..end].parse().ok()?;
    let rest = &upper[end..];
    let unit = rest.trim_start_matches(yt_dlp_rs::batch::is_py_space);
    let power = match unit {
        // `\b` after an empty unit cannot follow consumed whitespace.
        "" if unit.len() == rest.len() => 0,
        "" => return None,
        unit => "KMGTPEZY".find(unit).filter(|_| unit.len() == 1)? as i32 + 1,
    };
    let bytes = (number * 1024f64.powi(power)).round_ties_even();
    (bytes.is_finite() && bytes <= u64::MAX as f64).then_some(bytes as u64)
}

fn size_option(name: &str, value: Option<&str>) -> Result<Option<u64>, String> {
    value
        .map(|value| parse_bytes(value).ok_or_else(|| format!("invalid {name} \"{value}\" given")))
        .transpose()
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let matches = Args::command().get_matches();
    let mut args = Args::from_arg_matches(&matches)?;
    // `--no-match-filters` resets values given before it (optparse store_const).
    let after_reset = |values: &str, reset: &str| -> Vec<String> {
        // Defaulted flags also report an index, so require a command-line source.
        let last_reset = (matches.value_source(reset)
            == Some(clap::parser::ValueSource::CommandLine))
        .then(|| matches.indices_of(reset).and_then(|indices| indices.max()))
        .flatten()
        .unwrap_or(0);
        matches
            .indices_of(values)
            .into_iter()
            .flatten()
            .zip(matches.get_many::<String>(values).into_iter().flatten())
            .filter(|(index, _)| *index > last_reset)
            .map(|(_, value)| value.clone())
            .collect()
    };
    let match_filters = MatchFilters::parse(
        &after_reset("match_filters", "no_match_filters"),
        &after_reset("break_match_filters", "no_break_match_filters"),
    )?;
    QUIET.store(args.simulation().quiet, Ordering::Relaxed);
    NO_WARNINGS.store(args.no_warnings, Ordering::Relaxed);
    if args.no_download_archive {
        args.download_archive = None;
    }
    // Like upstream get_urls: batch URLs precede positional URLs, all stripped.
    let mut urls = match args.batch_file.as_deref().filter(|_| !args.no_batch_file) {
        Some(spec) => {
            // Upstream checks only an explicit -q here, before implied quiet.
            if spec == "-" && !args.quiet {
                eprintln!("Reading URLs from STDIN - EOF (Ctrl+D) to end:");
            }
            yt_dlp_rs::batch::load_batch_file(spec)
                .map_err(|_| format!("batch file {spec} could not be read"))?
        }
        None => Vec::new(),
    };
    urls.extend(
        std::mem::take(&mut args.urls)
            .iter()
            .map(|url| yt_dlp_rs::batch::py_strip(url).to_owned()),
    );
    if urls.is_empty() {
        return Err("You must provide at least one URL.".into());
    }
    let mut print_templates = args
        .print
        .iter()
        .map(|spec| PrintTemplate::parse(spec))
        .collect::<yt_dlp_rs::Result<Vec<_>>>()?;
    print_templates.extend(PrintTemplate::forced_fields(
        args.get_title,
        args.get_id,
        args.get_url,
        args.get_filename,
    ));
    let print_files = args
        .print_to_file
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[spec, file]| PrintToFile::parse(spec, file))
        .collect::<yt_dlp_rs::Result<Vec<_>>>()?;
    // Like upstream, printing implies quiet simulation unless --no-simulate.
    let printing = !print_templates.is_empty();
    // `quiet` was applied to screen output at startup.
    let Simulation {
        simulate,
        list_only,
        ..
    } = args.simulation();
    let listing = args.list_formats || args.list_subs;
    let write_subs = args.write_subs || (args.embed_subs && !args.write_auto_subs);
    let mut audio_options = AudioOptions::from_cli(&args.audio_format, &args.audio_quality)?;
    audio_options.keep_video = args.keep_video;
    // Source validation applies to both values even when recode takes precedence.
    let remux = args
        .remux_video
        .as_deref()
        .map(|format| VideoOptions::from_cli(format, VideoMode::Remux))
        .transpose()?;
    let recode = args
        .recode_video
        .as_deref()
        .map(|format| VideoOptions::from_cli(format, VideoMode::Recode))
        .transpose()?;
    if remux.is_some() && recode.is_some() {
        warning!("WARNING: --remux-video is ignored since --recode-video was given");
    }
    let processors = PostprocessOptions {
        audio: args.extract_audio.then_some(audio_options),
        video: recode.or(remux),
        embed: None,
        keep_video: args.keep_video,
    };
    let mut subtitle_conversion = args
        .convert_subs
        .as_deref()
        .map(SubtitleConversion::new)
        .transpose()?
        .filter(SubtitleConversion::is_enabled);
    if let Some(conversion) = &mut subtitle_conversion {
        conversion.keep_original = args.keep_video;
    }
    let format = args
        .format
        .as_deref()
        .or(if args.extract_audio && !args.keep_video {
            Some("bestaudio/best")
        } else {
            None
        });
    if let Some(format) = &args.format {
        validate_format_selector(format)?;
    }
    let playlist_selection = PlaylistSelection::new(
        args.playlist_items.as_deref(),
        args.playlist_start,
        args.playlist_end,
        args.playlist_reverse,
    )?;
    if args.playlist_items.is_some()
        && (args.playlist_start != 1 || args.playlist_end.is_some_and(|end| end != -1 && end != 0))
    {
        warning!(
            "WARNING: Ignoring --playlist-start and --playlist-end because --playlist-items was given"
        );
    }
    let subtitle_languages = (!args.sub_langs.is_empty()).then(|| args.sub_langs.join(","));
    let subtitle_options = SubtitleOptions::new(subtitle_languages.as_deref(), &args.sub_format)?;
    if let (Some(cookies), Some(archive)) = (&args.cookies, &args.download_archive) {
        let identical = std::path::absolute(cookies)? == std::path::absolute(archive)?;
        let canonical_alias = std::fs::canonicalize(cookies)
            .ok()
            .zip(std::fs::canonicalize(archive).ok())
            .is_some_and(|(left, right)| left == right);
        if identical || canonical_alias {
            return Err(yt_dlp_rs::Error::Cookies(
                "cookie file and download archive must have different paths".into(),
            )
            .into());
        }
    }
    let cookie_file_was_missing = args.cookies.as_ref().is_some_and(|path| !path.exists());
    let mut http_options = HttpOptions::from_cli(args.user_agent, args.referer, &args.add_headers)?;
    http_options.proxy = args.proxy;
    http_options.socket_timeout = args.socket_timeout;
    http_options.no_check_certificates = args.no_check_certificates;
    // Upstream -4/-6 store 0.0.0.0/:: into the shared source_address option.
    http_options.source_address = if args.force_ipv4 {
        Some("0.0.0.0".into())
    } else if args.force_ipv6 {
        Some("::".into())
    } else {
        args.source_address
    };
    http_options.validate()?;
    if let Some(path) = &args.cookies {
        http_options.cookies = Some(Arc::new(CookieJar::load(path)?));
    }
    let mut downloader = YoutubeDl::with_options(
        YoutubeOptions {
            js_runtime: args.youtube_js_runtime,
            include_translated_subtitles: args.write_auto_subs || args.list_subs,
        },
        http_options,
    )?;
    if let Some(path) = args.ffmpeg_location {
        downloader = downloader.with_ffmpeg(path);
    }
    if let Some(path) = args.ffprobe_location {
        downloader = downloader.with_ffprobe(path);
    }
    let archive_path = args.download_archive.clone();
    let mut archive = args
        .download_archive
        .map(DownloadArchive::load)
        .transpose()?;
    let windows_filenames = if args.windows_filenames {
        Some(true)
    } else if args.no_windows_filenames {
        Some(false)
    } else {
        None
    };
    // Upstream `overwrites`: None (default), Some(false) for -w, Some(true) forced.
    let overwrites = if args.force_overwrites {
        Some(true)
    } else if args.no_overwrites {
        Some(false)
    } else {
        None
    };
    let templates = OutputTemplates::parse(&args.output, args.restrict_filenames);
    let output_paths = yt_dlp_rs::output::parse_paths(&args.paths)?;
    let template_options = TemplateOptions {
        na_placeholder: args.output_na_placeholder,
        filename_mode: TemplateOptions::filename_mode(args.restrict_filenames, windows_filenames),
        windows_filenames: windows_filenames == Some(true),
        trim_file_name: args.trim_filenames,
        paths: output_paths,
    };
    // Like upstream, simulation never writes info JSON.
    let writes_info_json = args.write_info_json && !simulate;
    let sleep = SleepPolicy::from_cli(
        args.sleep_interval,
        args.max_sleep_interval,
        args.sleep_subtitles,
    )?;
    let min_filesize = size_option("min filesize", args.min_filesize.as_deref())?;
    let max_filesize = size_option("max filesize", args.max_filesize.as_deref())?;
    // Upstream validates the rate limit as strictly positive.
    let rate_limit = size_option("rate limit", args.limit_rate.as_deref())?;
    if rate_limit == Some(0) {
        return Err("rate limit \"0\" must be positive".into());
    }
    // Upstream compares against `float(max_downloads or 'inf')`.
    let max_downloads = args.max_downloads.filter(|&limit| limit != 0);
    let mut processed: i64 = 0;
    let mut cancelled: Option<Cancel> = None;
    let mut failed = false;
    let mut stdout = io::stdout().lock();
    for url in urls {
        let options = ExtractionOptions {
            retries: args.extractor_retries,
        };
        let mut pending: Option<Cancel> = None;
        let extracted =
            (|| -> yt_dlp_rs::Result<Option<(ExtractResult, Option<PlaylistCounts>)>> {
                let mut result =
                    downloader.extract_result_unselected_with_options(&url, options)?;
                let counts = playlist_selection.apply(&mut result);
                let result = if let Some(archive) = &archive {
                    let (result, reached) =
                        filter_archived(result, archive, args.break_on_existing)?;
                    if reached {
                        pending = Some(Cancel::Existing);
                    }
                    let Some(result) = result else {
                        return Ok(None);
                    };
                    result
                } else {
                    result
                };
                // Upstream `_match_entry` before selection: format fields are incomplete.
                let Some(mut result) = prefilter(result, &match_filters, &mut pending)? else {
                    return Ok(None);
                };
                if write_subs || args.write_auto_subs {
                    let select = |info: &mut yt_dlp_rs::VideoInfo| -> yt_dlp_rs::Result<()> {
                        let selection = subtitle_options.select_sources(
                            &info.subtitles,
                            &info.automatic_captions,
                            write_subs,
                            args.write_auto_subs,
                        )?;
                        for warning in selection.warnings {
                            warning!("WARNING: {}: {warning}", info.id);
                        }
                        info.requested_subtitles = ((write_subs && !info.subtitles.is_empty())
                            || (args.write_auto_subs && !info.automatic_captions.is_empty()))
                        .then_some(selection.requested);
                        Ok(())
                    };
                    match &mut result {
                        ExtractResult::Video(info) => select(info)?,
                        ExtractResult::Playlist(playlist) => {
                            for info in &mut playlist.entries {
                                select(info)?;
                            }
                        }
                    }
                }
                if !list_only {
                    downloader.select_result(&mut result, format)?;
                }
                Ok(Some((result, counts)))
            })();
        let (extracted, counts) = match extracted {
            Ok(Some(result)) => result,
            Ok(None) if pending.is_some() => {
                match cancel(pending.take().unwrap(), args.break_per_input) {
                    Some(cancel) => {
                        cancelled = Some(cancel);
                        break;
                    }
                    None => {
                        processed = 0;
                        continue;
                    }
                }
            }
            Ok(None) => continue,
            Err(error) => {
                eprintln!("ERROR: {url}: {error}");
                failed = true;
                continue;
            }
        };
        if let (ExtractResult::Playlist(playlist), Some(counts), true) =
            (&extracted, counts, writes_info_json)
        {
            // Upstream writes playlist metadata before processing any entry,
            // and a failed write skips the playlist.
            let written = (|| -> Result<(), Box<dyn std::error::Error>> {
                let Some(path) = templates.prepare_playlist(
                    playlist,
                    counts,
                    "pl_infojson",
                    &template_options,
                )?
                else {
                    return Ok(());
                };
                reject_reserved(&path, [&archive_path, &args.cookies])?;
                let value = playlist_info_json(playlist, counts, epoch())?;
                let written = write_info_json_with(&path, &value, overwrites != Some(false))?;
                report_info_json("playlist", &path, written);
                Ok(())
            })();
            if let Err(error) = written {
                eprintln!("ERROR: {url}: {error}");
                failed = true;
                continue;
            }
        }
        let single_json = args
            .dump_single_json
            .then(|| single_json(&extracted, counts))
            .transpose()?;
        let mut url_cancelled = false;
        for info in extracted.into_entries() {
            let mut counted = false;
            let mut entry_cancel = None;
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                if args.list_subs {
                    if !info.automatic_captions.is_empty() {
                        write!(stdout, "{}", render_automatic_captions(&info))?;
                    }
                    write!(stdout, "{}", render_subtitles(&info))?;
                }
                if args.list_formats {
                    write!(stdout, "{}", render_formats(&info))?;
                }
                if listing && !args.dump_json && !printing && simulate {
                    return Ok(());
                }
                // Upstream `process_info` rechecks complete metadata before counting.
                if !match_filters.is_empty() {
                    match match_filters.check(&info, false)? {
                        MatchOutcome::Pass => {}
                        MatchOutcome::Reject(reason) => {
                            screen!("[download] {reason}");
                            return Ok(());
                        }
                        MatchOutcome::Break(reason) => {
                            entry_cancel = Some(Cancel::Rejected(reason));
                            return Ok(());
                        }
                    }
                }
                // Upstream `process_info` counts every processed video.
                counted = true;
                if printing {
                    let filename = prepare_media(&templates, &info, &template_options)
                        .map(|path| path.to_string_lossy().into_owned());
                    // Render every line first so a failure prints nothing partial.
                    let lines = print_templates
                        .iter()
                        .map(|template| {
                            template.render(&info, filename.as_deref(), &template_options)
                        })
                        .collect::<yt_dlp_rs::Result<Vec<_>>>()?;
                    for line in lines {
                        writeln!(stdout, "{line}")?;
                    }
                }
                if !print_files.is_empty() {
                    let filename = prepare_media(&templates, &info, &template_options)
                        .map(|path| path.to_string_lossy().into_owned());
                    // Render every destination and line before appending any.
                    let writes = print_files
                        .iter()
                        .map(|pair| pair.render(&info, filename.as_deref(), &template_options))
                        .collect::<yt_dlp_rs::Result<Vec<_>>>()?;
                    for (pair, (path, line)) in print_files.iter().zip(writes) {
                        if !printing {
                            screen!(
                                "[info] Writing {:?} to: {}",
                                pair.template(),
                                path.display()
                            );
                        }
                        append_line(&path, &line)?;
                    }
                }
                if args.dump_json {
                    serde_json::to_writer(&mut stdout, &info)?;
                    writeln!(stdout)?;
                }
                // --force-write-archive records simulated or skipped videos.
                let mut force_record = |info: &yt_dlp_rs::VideoInfo| -> Result<(), String> {
                    match &mut archive {
                        Some(archive) if args.force_write_archive => archive
                            .record(info)
                            .map_err(|error| format!("archive update failed: {error}")),
                        _ => Ok(()),
                    }
                };
                if simulate {
                    // Native simulation summary; never alongside printed/JSON output.
                    if !printing && !args.dump_json && !args.dump_single_json {
                        screen!("[{}] {}: {}", info.extractor, info.id, info.title);
                    }
                    force_record(&info)?;
                } else {
                    if args.skip_download
                        && !writes_info_json
                        && info
                            .requested_subtitles
                            .as_ref()
                            .is_none_or(|subtitles| subtitles.is_empty())
                    {
                        force_record(&info)?;
                        return Ok(());
                    }
                    let path = prepare_media(&templates, &info, &template_options)?;
                    if !template_options.paths.is_empty() && path_is_absolute_template(&templates) {
                        warning!(
                            "WARNING: --paths is ignored since an absolute path is given in output template"
                        );
                    }
                    let processing =
                        processors.audio.is_some() || processors.video.is_some() || args.embed_subs;
                    let existing_media = !args.skip_download && path_exists(&path)?;
                    if existing_media && overwrites != Some(true) && processing {
                        // Upstream would rerun postprocessors on the existing file.
                        return Err(yt_dlp_rs::Error::Postprocess(format!(
                            "{} already exists; postprocessing an existing download is unsupported (use --force-overwrites)",
                            path.display()
                        ))
                        .into());
                    }
                    // Plan source, intermediate and final names before subtitle I/O.
                    let has_subtitles = info
                        .requested_subtitles
                        .as_ref()
                        .is_some_and(|subtitles| !subtitles.is_empty());
                    let sidecars =
                        match templates.prepare(&info, Some("subtitle"), &template_options)? {
                            Some(base) => subtitle_paths(&info, &base)?,
                            None => {
                                if has_subtitles {
                                    screen!("[info] Skipping writing video subtitles");
                                }
                                Vec::new()
                            }
                        };
                    let mut seen = std::collections::HashSet::new();
                    let info_json = if writes_info_json {
                        templates.prepare(&info, Some("infojson"), &template_options)?
                    } else {
                        None
                    };
                    if let Some(info_json) = &info_json {
                        seen.insert(info_json.as_os_str().to_ascii_lowercase());
                        if paths_alias(info_json, &path)? {
                            return Err(yt_dlp_rs::Error::InfoJson(
                                "info JSON path collides with media output".into(),
                            )
                            .into());
                        }
                        reject_reserved(info_json, [&archive_path, &args.cookies])?;
                    }
                    for (_, subtitle, source) in &sidecars {
                        let mut paths = vec![source.clone()];
                        if let Some(conversion) = &subtitle_conversion {
                            let target = conversion.output_path(source, &subtitle.ext)?;
                            if target != *source {
                                paths.push(target);
                            }
                            if let Some(intermediate) =
                                conversion.intermediate_path(source, &subtitle.ext)
                            {
                                paths.push(intermediate);
                            }
                        }
                        for (index, sidecar) in paths.iter().enumerate() {
                            if !seen.insert(sidecar.as_os_str().to_ascii_lowercase()) {
                                return Err(yt_dlp_rs::Error::Subtitle(
                                    "subtitle conversion or info JSON filenames collide".into(),
                                )
                                .into());
                            }
                            for reserved in [&archive_path, &args.cookies].into_iter().flatten() {
                                if paths_alias(sidecar, reserved)? {
                                    return Err(yt_dlp_rs::Error::Subtitle("subtitle path collides with cookie file or download archive".into()).into());
                                }
                            }
                            if index != 0 {
                                match std::fs::symlink_metadata(sidecar) {
                                    Ok(_) => {
                                        return Err(yt_dlp_rs::Error::OutputExists(
                                            sidecar.clone(),
                                        )
                                        .into());
                                    }
                                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                                    Err(error) => return Err(error.into()),
                                }
                            }
                        }
                    }
                    let mut embedding_files = Vec::new();
                    let planned_sidecars: Vec<_> = sidecars
                        .iter()
                        .map(|(_, subtitle, source)| (subtitle.ext.clone(), source.clone()))
                        .collect();
                    for (language, subtitle, subtitle_path) in sidecars {
                        // Related files are overwritten unless --no-overwrites.
                        if overwrites != Some(false)
                            && std::fs::symlink_metadata(&subtitle_path).is_ok()
                            && std::fs::metadata(&subtitle_path)?.is_file()
                        {
                            screen!(
                                "[download] Deleting existing file {}",
                                subtitle_path.display()
                            );
                            std::fs::remove_file(&subtitle_path)?;
                        }
                        match std::fs::symlink_metadata(&subtitle_path) {
                            Ok(_) if std::fs::metadata(&subtitle_path)?.is_file() => {
                                screen!(
                                    "[info] Subtitle {} is already present",
                                    subtitle_path.display()
                                );
                            }
                            Ok(_) => {
                                return Err(yt_dlp_rs::Error::Subtitle(format!(
                                    "subtitle destination is not a regular file: {}",
                                    subtitle_path.display()
                                ))
                                .into());
                            }
                            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                                screen!(
                                    "[info] Writing {language:?} subtitles to: {}",
                                    subtitle_path.display()
                                );
                                sleep.subtitle();
                                downloader.download_subtitle_with_options(
                                    &info,
                                    &subtitle,
                                    &subtitle_path,
                                    DownloadOptions {
                                        retries: args.retries,
                                        ..Default::default()
                                    },
                                )?;
                            }
                            Err(error) => return Err(error.into()),
                        }
                        let mut embedded_file = SubtitleFile {
                            path: subtitle_path.clone(),
                            ext: subtitle.ext.clone(),
                            language,
                            name: subtitle.name.clone(),
                        };
                        if let Some(conversion) = &subtitle_conversion {
                            let converted = downloader.convert_subtitle_with_options(
                                &subtitle_path,
                                &subtitle.ext,
                                conversion,
                            )?;
                            for warning in converted.warnings {
                                warning!("WARNING: {warning}");
                            }
                            if converted.converted {
                                // The source converter replaces the subtitle's
                                // metadata entry and drops its original name.
                                embedded_file.name = None;
                                screen!(
                                    "[SubtitlesConvertor] Destination: {}",
                                    converted.path.display()
                                );
                            }
                            embedded_file.path = converted.path;
                            embedded_file.ext = converted.ext;
                        }
                        embedding_files.push(embedded_file);
                    }
                    // Upstream writes info JSON after subtitles and before media.
                    if let Some(info_json) = &info_json {
                        let value = video_info_json(&info, epoch())?;
                        let written =
                            write_info_json_with(info_json, &value, overwrites != Some(false))?;
                        report_info_json("video", info_json, written);
                    }
                    if args.skip_download {
                        force_record(&info)?;
                        return Ok(());
                    }
                    if existing_media {
                        if overwrites == Some(true) {
                            screen!("[download] Deleting existing file {}", path.display());
                            std::fs::remove_file(&path)?;
                        } else {
                            screen!("[download] {} has already been downloaded", path.display());
                            if let Some(archive) = &mut archive {
                                archive
                                    .record(&info)
                                    .map_err(|error| format!("archive update failed: {error}"))?;
                            }
                            return Ok(());
                        }
                    }
                    screen!("[download] Destination: {}", path.display());
                    // Upstream sleeps before each stream's FileDownloader.download.
                    for _ in 0..info.requested_formats.len().max(1) {
                        sleep.media();
                    }
                    let mut last_progress = Instant::now();
                    let progress = |progress: yt_dlp_rs::DownloadProgress| {
                        if progress.phase == DownloadPhase::Downloading
                            && last_progress.elapsed() >= Duration::from_secs(1)
                        {
                            if let Some(total) = progress.total_bytes {
                                screen!(
                                    "[download] {} / {total} bytes (attempt {})",
                                    progress.downloaded_bytes,
                                    progress.attempt
                                );
                            } else {
                                screen!(
                                    "[download] {} bytes (attempt {})",
                                    progress.downloaded_bytes,
                                    progress.attempt
                                );
                            }
                            last_progress = Instant::now();
                        }
                        ControlFlow::Continue(())
                    };
                    let mut processors = processors.clone();
                    if args.embed_subs {
                        processors.embed = Some(SubtitleEmbedding {
                            subtitles: embedding_files,
                            keep_subtitles: args.write_subs || args.keep_video,
                        });
                    }
                    let (bytes, published_paths) = if processors.audio.is_some()
                        || processors.video.is_some()
                        || processors.embed.is_some()
                    {
                        let result = downloader.download_processed_with_progress(
                            &info,
                            &path,
                            DownloadOptions {
                                retries: args.retries,
                                min_filesize,
                                max_filesize,
                                rate_limit,
                            },
                            &processors,
                            progress,
                        )?;
                        for warning in result.warnings {
                            warning!("WARNING: {warning}");
                        }
                        screen!("[postprocess] Destination: {}", result.path.display());
                        let mut paths = vec![result.path];
                        paths.extend(result.kept_files);
                        (result.bytes, paths)
                    } else {
                        let bytes = downloader.download_with_progress(
                            &info,
                            &path,
                            DownloadOptions {
                                retries: args.retries,
                                min_filesize,
                                max_filesize,
                                rate_limit,
                            },
                            progress,
                        )?;
                        (bytes, vec![path.clone()])
                    };
                    let mut published_paths = published_paths;
                    published_paths.extend(info_json);
                    for (ext, source) in planned_sidecars {
                        if let Some(conversion) = &subtitle_conversion {
                            published_paths.push(conversion.output_path(&source, &ext)?);
                            published_paths.extend(conversion.intermediate_path(&source, &ext));
                        }
                        published_paths.push(source);
                    }
                    if let Some(archive_path) = &archive_path {
                        for output in &published_paths {
                            let identical =
                                std::path::absolute(output)? == std::path::absolute(archive_path)?;
                            let canonical_alias = std::fs::canonicalize(output)
                                .ok()
                                .zip(std::fs::canonicalize(archive_path).ok())
                                .is_some_and(|(left, right)| left == right);
                            if identical || canonical_alias {
                                return Err(yt_dlp_rs::Error::Archive(
                                    "download output collides with archive path".into(),
                                )
                                .into());
                            }
                        }
                    }
                    if let Some(archive) = &mut archive {
                        archive.record(&info).map_err(|error| {
                            format!("download completed, but archive update failed: {error}")
                        })?;
                    }
                    screen!("[download] Downloaded {bytes} bytes");
                }
                Ok(())
            })();
            let succeeded = match result {
                Ok(()) => true,
                Err(error) => match error.downcast_ref::<yt_dlp_rs::Error>() {
                    // Upstream skips the transfer without an error or archive record.
                    Some(yt_dlp_rs::Error::FileSizeLimit(message)) => {
                        screen!("[download] {message}");
                        true
                    }
                    _ => {
                        eprintln!("ERROR: {url}: {}: {error}", info.id);
                        failed = true;
                        false
                    }
                },
            };
            if let Some(reason) = entry_cancel {
                cancelled = cancel(reason, args.break_per_input);
                if cancelled.is_none() {
                    processed = 0;
                }
                url_cancelled = true;
                break;
            }
            if counted {
                processed += 1;
            }
            if succeeded && counted && max_downloads.is_some_and(|limit| processed >= limit) {
                cancelled = cancel(Cancel::MaxDownloads, args.break_per_input);
                if cancelled.is_none() {
                    processed = 0;
                }
                // The rest of this input is skipped either way.
                url_cancelled = true;
                break;
            }
        }
        if cancelled.is_some() {
            break;
        }
        if let Some(reason) = pending.take().filter(|_| !url_cancelled) {
            // Entries before the archived/rejected one were processed; stop now.
            cancelled = cancel(reason, args.break_per_input);
            if cancelled.is_some() {
                break;
            }
            processed = 0;
            continue;
        }
        if url_cancelled {
            continue;
        }
        if let Some(value) = single_json {
            serde_json::to_writer(&mut stdout, &value)?;
            writeln!(stdout)?;
        }
    }
    for warning in downloader.cookie_jar().take_warnings() {
        warning!("WARNING: {warning}");
    }
    if let Some(path) = &args.cookies {
        let cookies = downloader.cookie_jar();
        let save = if cookie_file_was_missing {
            cookies.save_new(path)
        } else {
            cookies.save(path)
        };
        if let Err(error) = save {
            eprintln!("ERROR: {error}");
            failed = true;
        }
    }
    if let Some(cancel) = cancelled {
        return Err(Box::new(cancel));
    }
    Ok(!failed)
}

/// Upstream `DownloadCancelled` reasons; uncaught ones exit with 101.
#[derive(Debug, Clone)]
enum Cancel {
    MaxDownloads,
    Existing,
    /// A `--break-match-filters` rejection reason.
    Rejected(String),
}

impl std::fmt::Display for Cancel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Cancel::MaxDownloads => {
                "Maximum number of downloads reached, stopping due to --max-downloads"
            }
            Cancel::Existing => {
                "Encountered a video that is already in the archive, stopping due to --break-on-existing"
            }
            Cancel::Rejected(reason) => reason,
        })
    }
}

impl std::error::Error for Cancel {}

/// Report a cancellation like `__download_wrapper`; with `--break-per-input`
/// it only ends the current input (returns `None`).
fn cancel(reason: Cancel, per_input: bool) -> Option<Cancel> {
    screen!("[info] {reason}");
    if per_input {
        None
    } else {
        screen!("Aborting remaining downloads");
        Some(reason)
    }
}

fn epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn report_info_json(label: &str, path: &std::path::Path, written: InfoJsonWrite) {
    match written {
        InfoJsonWrite::Written => {
            screen!(
                "[info] Writing {label} metadata as JSON to: {}",
                path.display()
            )
        }
        InfoJsonWrite::AlreadyPresent => {
            let mut title = label.to_owned();
            title[..1].make_ascii_uppercase();
            screen!("[info] {title} metadata is already present")
        }
    }
}

fn reject_reserved<const N: usize>(
    path: &std::path::Path,
    reserved: [&Option<PathBuf>; N],
) -> Result<(), Box<dyn std::error::Error>> {
    for reserved in reserved.into_iter().flatten() {
        if paths_alias(path, reserved)? {
            return Err(yt_dlp_rs::Error::InfoJson(
                "info JSON path collides with cookie file or download archive".into(),
            )
            .into());
        }
    }
    Ok(())
}

fn paths_alias(left: &std::path::Path, right: &std::path::Path) -> io::Result<bool> {
    Ok(std::path::absolute(left)? == std::path::absolute(right)?
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right))
}

/// Apply match filters before selection: rejected entries are skipped, and a
/// breaking rejection truncates entries and records the pending cancellation.
fn prefilter(
    result: ExtractResult,
    filters: &MatchFilters,
    pending: &mut Option<Cancel>,
) -> yt_dlp_rs::Result<Option<ExtractResult>> {
    if filters.is_empty() {
        return Ok(Some(result));
    }
    let mut keep = |info: &yt_dlp_rs::VideoInfo| -> yt_dlp_rs::Result<Option<bool>> {
        Ok(match filters.check(info, true)? {
            MatchOutcome::Pass => Some(true),
            MatchOutcome::Reject(reason) => {
                screen!("[download] {reason}");
                Some(false)
            }
            MatchOutcome::Break(reason) => {
                *pending = Some(Cancel::Rejected(reason));
                None
            }
        })
    };
    match result {
        ExtractResult::Video(info) => {
            Ok((keep(&info)? == Some(true)).then_some(ExtractResult::Video(info)))
        }
        ExtractResult::Playlist(mut playlist) => {
            let mut kept = Vec::new();
            for info in playlist.entries {
                match keep(&info)? {
                    Some(true) => kept.push(info),
                    Some(false) => {}
                    None => break,
                }
            }
            playlist.entries = kept;
            Ok(Some(ExtractResult::Playlist(playlist)))
        }
    }
}

/// Drop archived entries. With `break_on_existing`, entries stop at the first
/// archived one and the returned flag requests cancellation after the earlier
/// entries are processed (upstream processes entries lazily in order).
fn filter_archived(
    result: ExtractResult,
    archive: &DownloadArchive,
    break_on_existing: bool,
) -> yt_dlp_rs::Result<(Option<ExtractResult>, bool)> {
    let keep = |info: &yt_dlp_rs::VideoInfo| -> yt_dlp_rs::Result<bool> {
        let recorded = archive.contains(info)?;
        if recorded {
            screen!(
                "[download] {}: {} has already been recorded in the archive",
                info.id,
                info.title
            );
        }
        Ok(!recorded)
    };
    match result {
        ExtractResult::Video(info) => {
            let kept = keep(&info)?;
            Ok((
                kept.then_some(ExtractResult::Video(info)),
                !kept && break_on_existing,
            ))
        }
        ExtractResult::Playlist(mut playlist) => {
            let mut pending = Vec::new();
            let mut reached = false;
            for info in playlist.entries {
                if keep(&info)? {
                    pending.push(info);
                } else if break_on_existing {
                    reached = true;
                    break;
                }
            }
            // An all-archived playlist is still processed for playlist-level files.
            playlist.entries = pending;
            Ok((Some(ExtractResult::Playlist(playlist)), reached))
        }
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(error) if error.is::<Cancel>() => std::process::ExitCode::from(101),
        Err(error) => {
            eprintln!("ERROR: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bytes_matches_upstream_vectors() {
        // Pinned upstream `parse_bytes`; values beyond u64 are rejected natively.
        let cases: &[(&str, Option<u64>)] = &[
            ("50k", Some(51200)),
            ("44.6M", Some(46766490)),
            ("1.5", Some(2)),
            ("2.5", Some(2)),
            ("10 k", Some(10240)),
            ("10\tK", Some(10240)),
            ("1e3", None),
            (".5k", None),
            ("5.k", None),
            ("0", Some(0)),
            ("-1", None),
            ("1KiB", None),
            ("1b", None),
            ("1_0", None),
            ("10 ", None),
            ("1g", Some(1073741824)),
            ("3y", None),
            ("1.25K", Some(1280)),
            ("0.5", Some(0)),
            ("k", None),
            ("12345678901234567890", Some(12345678901234567168)),
            ("1KK", None),
        ];
        for (value, expected) in cases {
            assert_eq!(parse_bytes(value), *expected, "{value:?}");
        }
    }

    #[test]
    fn saved_upstream_simulation_resolution() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/simulate/cases.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let mut argv = vec!["yt-dlp-rs".to_owned()];
            argv.extend(
                case["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| arg.as_str().unwrap().to_owned()),
            );
            argv.push("https://example.invalid/".into());
            let simulation = Args::try_parse_from(&argv).unwrap().simulation();
            let expected = Simulation {
                simulate: case["simulate"].as_bool().unwrap(),
                list_only: case["list_only"].as_bool().unwrap(),
                quiet: case["quiet"].as_bool().unwrap(),
            };
            assert_eq!(simulation, expected, "{argv:?}");
        }
    }

    #[test]
    fn saved_embedding_flags_preserve_implied_writing_and_retention_precedence() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/subtitle_embedding/cases.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let flags = case["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap());
            let args = Args::try_parse_from(
                std::iter::once("yt-dlp-rs")
                    .chain(flags)
                    .chain(std::iter::once("https://example.test/video.mp4")),
            )
            .unwrap();
            let expected = &case["expected"];
            assert_eq!(
                args.embed_subs,
                expected["enabled"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                args.write_subs || (args.embed_subs && !args.write_auto_subs),
                expected["write_subtitles"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                args.write_auto_subs,
                expected["write_automatic"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                args.keep_video,
                expected["keep_video"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if args.embed_subs && case.get("already_have_subtitle").is_none() {
                assert_eq!(
                    args.write_subs,
                    expected["already_have_subtitle"].as_bool().unwrap(),
                    "{}",
                    case["name"]
                );
            }
        }
    }

    #[test]
    fn saved_subtitle_conversion_options_match_pinned_targets_and_alias_precedence() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/subtitle_conversion/cases.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let flags = case["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap());
            let args = Args::try_parse_from(
                std::iter::once("yt-dlp-rs")
                    .chain(flags)
                    .chain(std::iter::once("https://example.test/video.mp4")),
            )
            .unwrap();
            let target = args
                .convert_subs
                .as_deref()
                .filter(|target| *target != "none");
            assert_eq!(
                target,
                case["expected"]["target"].as_str(),
                "{}",
                case["name"]
            );
            if let Some(format) = args.convert_subs {
                assert!(SubtitleConversion::new(&format).is_ok());
            }
            assert_eq!(
                args.skip_download,
                case["expected"]["skip_download"].as_bool().unwrap()
            );
        }
    }

    #[test]
    fn automatic_caption_flags_override_in_argument_order() {
        let parse = |flags: &[&str]| {
            Args::try_parse_from(
                std::iter::once("yt-dlp-rs")
                    .chain(flags.iter().copied())
                    .chain(std::iter::once("https://example.test/video.mp4")),
            )
            .unwrap()
        };
        assert!(!parse(&["--write-auto-subs", "--no-write-automatic-subs"]).write_auto_subs);
        assert!(parse(&["--no-write-auto-subs", "--write-automatic-subs"]).write_auto_subs);
        let both = parse(&["--write-subs", "--write-auto-subs"]);
        assert!(both.write_subs && both.write_auto_subs);
    }
}
