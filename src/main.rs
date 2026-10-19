use clap::{CommandFactory, FromArgMatches, Parser};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use yt_dlp_rs::{
    AudioOptions, CookieJar, DownloadArchive, DownloadOptions, DownloadPhase, EntrySelection,
    ExtractResult, ExtractionOptions, HttpOptions, InfoJsonWrite, MatchFilters, MatchOutcome,
    PlaylistCounts, PlaylistSelection, PostprocessOptions, SubtitleConversion, SubtitleEmbedding,
    SubtitleFile, SubtitleOptions, ThumbnailConversion, VideoMode, VideoOptions, YoutubeDl,
    YoutubeOptions,
    dates::DateRange,
    output::{
        ExecCommand, ExecCommands, OutputTemplates, PrintTemplate, PrintToFile, Stage, StagePaths,
        TemplateOptions,
    },
    render_automatic_captions, render_subtitles, subtitle_paths, validate_format_selector,
    write_info_json_with,
};

/// `-q`: suppress informational `[...]` screen lines (upstream `to_screen`).
static QUIET: AtomicBool = AtomicBool::new(false);
/// `--no-warnings`: suppress `WARNING:` lines.
static NO_WARNINGS: AtomicBool = AtomicBool::new(false);
/// Upstream `logtostderr` (`-o -`): screen and progress output use stderr.
static LOG_TO_STDERR: AtomicBool = AtomicBool::new(false);

/// Upstream `_out_files.out`: stdout, or stderr with `logtostderr`.
fn out_stream() -> Box<dyn Write> {
    if LOG_TO_STDERR.load(Ordering::Relaxed) {
        Box::new(io::stderr().lock())
    } else {
        Box::new(io::stdout().lock())
    }
}

/// Upstream `to_screen`: stdout, or stderr with `logtostderr`.
macro_rules! screen {
    ($($arg:tt)*) => {
        if !QUIET.load(Ordering::Relaxed) {
            if LOG_TO_STDERR.load(Ordering::Relaxed) {
                eprintln!($($arg)*);
            } else {
                println!($($arg)*);
            }
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
    version,
    args_override_self = true,
    about = "Native Rust yt-dlp adaptation (direct HTTP, HTML5 and HLS/DASH VOD)"
)]
struct Args {
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
    /// Resume partially downloaded files/fragments (default)
    #[arg(short = 'c', long = "continue", overrides_with_all = ["continue_partial", "no_continue"])]
    continue_partial: bool,
    /// Do not resume partially downloaded files (restart from the beginning)
    #[arg(long, overrides_with_all = ["continue_partial", "no_continue"])]
    no_continue: bool,
    /// Use .part files instead of writing directly into the output file (default)
    #[arg(long, overrides_with_all = ["part", "no_part"])]
    part: bool,
    /// Do not use .part files - write directly into the output file
    #[arg(long, overrides_with_all = ["part", "no_part"])]
    no_part: bool,
    /// Output progress as new lines
    #[arg(long)]
    newline: bool,
    /// Do not print progress
    #[arg(long, overrides_with_all = ["no_progress", "progress"])]
    no_progress: bool,
    /// Show progress, even in quiet mode
    #[arg(long, overrides_with_all = ["no_progress", "progress"])]
    progress: bool,
    /// Time between progress output (default: 0)
    #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
    progress_delta: Option<f64>,
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
    #[arg(long, hide = true)]
    get_thumbnail: bool,
    #[arg(long, hide = true)]
    get_description: bool,
    #[arg(long, hide = true)]
    get_duration: bool,
    #[arg(long, hide = true)]
    get_format: bool,
    /// List available formats (selection still applies with -j or -s; no downloads)
    #[arg(short = 'F', long = "list-formats")]
    list_formats: bool,
    /// Present the list of formats in a more readable table (default)
    #[arg(long, hide = true, overrides_with = "list_formats_old")]
    list_formats_as_table: bool,
    /// Present the list of formats in the older layout
    #[arg(
        long,
        alias = "no-list-formats-as-table",
        overrides_with = "list_formats_as_table"
    )]
    list_formats_old: bool,
    /// List available subtitle languages/formats without downloading
    #[arg(long)]
    list_subs: bool,
    /// Use the Last-modified header to set the file modification time
    #[arg(long, overrides_with_all = ["mtime", "no_mtime"])]
    mtime: bool,
    /// Do not use the Last-modified header to set the file modification time (default)
    #[arg(long, overrides_with_all = ["mtime", "no_mtime"])]
    no_mtime: bool,
    /// Write the best available thumbnail image
    #[arg(long, action = clap::ArgAction::Count)]
    write_thumbnail: u8,
    /// Do not write thumbnail images (default)
    #[arg(long, action = clap::ArgAction::Count)]
    no_write_thumbnail: u8,
    /// Write all thumbnail images
    #[arg(long, action = clap::ArgAction::Count)]
    write_all_thumbnails: u8,
    /// List available thumbnails without downloading
    #[arg(long)]
    list_thumbnails: bool,
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
    /// Execute a command, optionally prefixed with when (video, before_dl, post_process, after_move (default), after_video); fields allow d/i/f/q conversions
    #[arg(long, value_name = "[WHEN:]CMD", allow_hyphen_values = true)]
    exec: Vec<String>,
    /// Remove any previously defined --exec
    #[arg(long, action = clap::ArgAction::Count)]
    no_exec: u8,
    #[arg(long, hide = true, value_name = "CMD", allow_hyphen_values = true)]
    exec_before_download: Vec<String>,
    #[arg(long, hide = true, action = clap::ArgAction::Count)]
    no_exec_before_download: u8,
    /// Embed metadata (title, date, description, URL, artist, genre, languages) with ffmpeg
    #[arg(long, alias = "add-metadata", overrides_with_all = ["embed_metadata", "no_embed_metadata"])]
    embed_metadata: bool,
    /// Do not embed metadata (default)
    #[arg(long, alias = "no-add-metadata", overrides_with_all = ["embed_metadata", "no_embed_metadata"])]
    no_embed_metadata: bool,
    /// Add chapter markers to the video file (implied by --embed-metadata)
    #[arg(long, alias = "add-chapters", overrides_with_all = ["embed_chapters", "no_embed_chapters"])]
    embed_chapters: bool,
    /// Do not add chapter markers
    #[arg(long, alias = "no-add-chapters", overrides_with_all = ["embed_chapters", "no_embed_chapters"])]
    no_embed_chapters: bool,
    /// Parse additional metadata like title/artist from other fields (default WHEN: pre_process)
    #[arg(long, value_name = "[WHEN:]FROM:TO", allow_hyphen_values = true)]
    parse_metadata: Vec<String>,
    /// Replace text in metadata fields using a regex (default WHEN: pre_process)
    #[arg(long, value_names = ["[WHEN:]FIELDS", "REGEX", "REPLACE"], num_args = 3, allow_hyphen_values = true)]
    replace_in_metadata: Vec<String>,
    #[arg(long, hide = true, value_name = "FORMAT")]
    metadata_from_title: Option<String>,
    /// Embed the infojson as an attachment to mkv/mka video files
    #[arg(long, overrides_with = "no_embed_info_json")]
    embed_info_json: bool,
    /// Do not embed the infojson as an attachment to the video file
    #[arg(long, overrides_with = "embed_info_json")]
    no_embed_info_json: bool,
    /// Split video into multiple files based on internal chapters (the "chapter" output template)
    #[arg(long, alias = "split-tracks", overrides_with_all = ["split_chapters", "no_split_chapters"])]
    split_chapters: bool,
    /// Do not split video based on chapters (default)
    #[arg(long, alias = "no-split-tracks", overrides_with_all = ["split_chapters", "no_split_chapters"])]
    no_split_chapters: bool,
    /// Embed the best written thumbnail as cover art (mp3, mkv/mka, mp4/m4a/m4v/mov; implies writing it)
    #[arg(long, overrides_with_all = ["embed_thumbnail", "no_embed_thumbnail"])]
    embed_thumbnail: bool,
    /// Do not embed thumbnails (default)
    #[arg(long, overrides_with_all = ["embed_thumbnail", "no_embed_thumbnail"])]
    no_embed_thumbnail: bool,
    /// Convert written thumbnails to jpg, png or webp (rules like webp>jpg/png); none disables conversion
    #[arg(long, value_name = "FORMAT", overrides_with = "convert_thumbnails")]
    convert_thumbnails: Option<String>,
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
    /// Process the video or playlist information in FILE (written with --write-info-json) instead of URLs
    #[arg(long, alias = "load-info", value_name = "FILE")]
    load_info_json: Option<String>,
    /// Write an internet shortcut file for the platform (.webloc on macOS, .desktop on Linux, .url otherwise)
    #[arg(long)]
    write_link: bool,
    /// Write a .url Windows internet shortcut
    #[arg(long)]
    write_url_link: bool,
    /// Write a .webloc macOS internet shortcut
    #[arg(long)]
    write_webloc_link: bool,
    /// Write a .desktop Linux internet shortcut
    #[arg(long)]
    write_desktop_link: bool,
    /// Write cleaned video and playlist metadata to .info.json files (existing files are kept)
    #[arg(long, overrides_with = "no_write_info_json")]
    write_info_json: bool,
    /// Write the video description to a .description file
    #[arg(long, overrides_with_all = ["write_description", "no_write_description"])]
    write_description: bool,
    /// Do not write the video description (default)
    #[arg(long, overrides_with_all = ["write_description", "no_write_description"])]
    no_write_description: bool,
    /// Write playlist metadata files with --write-info-json/--write-description (default)
    #[arg(long, overrides_with_all = ["write_playlist_metafiles", "no_write_playlist_metafiles"])]
    write_playlist_metafiles: bool,
    /// Do not write playlist metadata files
    #[arg(long, overrides_with_all = ["write_playlist_metafiles", "no_write_playlist_metafiles"])]
    no_write_playlist_metafiles: bool,
    /// Disable info JSON writing
    #[arg(long, overrides_with = "write_info_json")]
    no_write_info_json: bool,
    /// Format ID or indexed/grouped selector with filters, + pairs and / fallbacks
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
    /// Number of retries (default is 10), or "infinite"
    #[arg(
        short = 'R',
        long,
        default_value = "10",
        value_name = "RETRIES",
        allow_hyphen_values = true
    )]
    retries: String,
    /// Number of retries for a fragment (default is 10), or "infinite" (DASH and hlsnative)
    #[arg(
        long,
        default_value = "10",
        value_name = "RETRIES",
        allow_hyphen_values = true
    )]
    fragment_retries: String,
    /// Time to sleep between retries, optionally prefixed by http (default) or fragment: a number, linear=START[:END[:STEP=1]] or exp=START[:END[:BASE=2]]
    #[arg(long, value_name = "[TYPE:]EXPR", allow_hyphen_values = true)]
    retry_sleep: Vec<String>,
    /// Skip unavailable fragments for DASH and hlsnative downloads (default)
    #[arg(long, alias = "no-abort-on-unavailable-fragments", overrides_with_all = ["skip_unavailable_fragments", "abort_on_unavailable_fragments"])]
    skip_unavailable_fragments: bool,
    /// Abort download if a fragment is unavailable
    #[arg(long, alias = "no-skip-unavailable-fragments", overrides_with_all = ["skip_unavailable_fragments", "abort_on_unavailable_fragments"])]
    abort_on_unavailable_fragments: bool,
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
    /// Don't load any more configuration files except those given to --config-locations (Alias: --no-config)
    #[arg(long, visible_alias = "no-config")]
    ignore_config: bool,
    /// Do not load any custom configuration files (default); inside a configuration file, ignore its previous --config-locations
    #[arg(long)]
    no_config_locations: bool,
    /// Location of the main configuration file; either the path to the config or its containing directory ("-" for stdin)
    #[arg(long, value_name = "PATH")]
    config_locations: Vec<String>,
    /// Sort the formats by the fields given (comma-separated; later values take precedence)
    #[arg(
        short = 'S',
        long,
        value_name = "SORTORDER",
        allow_hyphen_values = true
    )]
    format_sort: Vec<String>,
    /// Disregard previous user specified sort order and reset to the default
    #[arg(long)]
    format_sort_reset: bool,
    /// Force user specified sort order to have precedence over all fields (Alias: --S-force)
    #[arg(
        long,
        visible_alias = "S-force",
        overrides_with = "no_format_sort_force"
    )]
    format_sort_force: bool,
    /// Some fields have precedence over the user specified sort order (default)
    #[arg(long, overrides_with = "format_sort_force")]
    no_format_sort_force: bool,
    /// Prefer video formats with free containers over non-free ones of the same quality
    #[arg(long, overrides_with = "no_prefer_free_formats")]
    prefer_free_formats: bool,
    /// Don't give any special preference to free containers (default)
    #[arg(long, overrides_with = "prefer_free_formats")]
    no_prefer_free_formats: bool,
    /// Automatically correct known faults of the file: never (do nothing), warn
    /// (only emit a warning), detect_or_warn (the default; fix the file if we can,
    /// warn otherwise), force (try fixing even if the file already exists)
    #[arg(long, value_name = "POLICY", value_parser = ["never", "ignore", "warn", "detect_or_warn", "force"])]
    fixup: Option<String>,
    /// Use the mpegts container for HLS videos (no MPEG-TS fixup)
    #[arg(long, overrides_with = "no_hls_use_mpegts")]
    hls_use_mpegts: bool,
    /// Do not use the mpegts container for HLS videos (default)
    #[arg(long, overrides_with = "hls_use_mpegts")]
    no_hls_use_mpegts: bool,
    /// Download only chapters matching the regex, or "*START-END" time
    /// ranges (negative values count from the end), through ffmpeg; may be
    /// repeated
    #[arg(long, value_name = "REGEX", allow_hyphen_values = true)]
    download_sections: Vec<String>,
    /// Containers that may be used when merging formats, separated by "/",
    /// e.g. "mp4/mkv"; ignored if no merge is required (currently supported:
    /// avi, flv, mkv, mov, mp4, webm)
    #[arg(long, value_name = "FORMAT")]
    merge_output_format: Option<String>,
    /// Remux media to FORMAT or ordered SOURCE>TARGET/TARGET rules (stream copy)
    #[arg(long, value_name = "FORMAT", overrides_with = "remux_video")]
    remux_video: Option<String>,
    /// Re-encode media to FORMAT or ordered SOURCE>TARGET/TARGET rules
    #[arg(long, value_name = "FORMAT", overrides_with = "recode_video")]
    recode_video: Option<String>,
    /// Keep the intermediate video file on disk after post-processing
    #[arg(short = 'k', long, overrides_with = "no_keep_video")]
    keep_video: bool,
    /// Remove processing sources after success (default)
    #[arg(long, overrides_with = "keep_video")]
    no_keep_video: bool,
    /// Explicit ffprobe executable for audio inspection (otherwise beside ffmpeg)
    #[arg(long, value_name = "PATH")]
    ffprobe_location: Option<PathBuf>,
    /// Location of the ffmpeg binary; either the path to the binary or its
    /// containing directory (default: ffmpeg/ffprobe from PATH)
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
    #[arg(long, overrides_with = "no_playlist_reverse")]
    playlist_reverse: bool,
    #[arg(long, hide = true, overrides_with = "playlist_reverse")]
    no_playlist_reverse: bool,
    /// Download playlist videos in random order
    #[arg(long)]
    playlist_random: bool,
    /// Process entries in the playlist as they are received. This disables n_entries, --playlist-random and --playlist-reverse
    #[arg(long, overrides_with = "no_lazy_playlist")]
    lazy_playlist: bool,
    /// Process videos in the playlist only after the entire playlist is parsed (default)
    #[arg(long, overrides_with = "lazy_playlist")]
    no_lazy_playlist: bool,
    /// Download only the video, if the URL refers to a video and a playlist
    #[arg(long, overrides_with = "yes_playlist")]
    no_playlist: bool,
    /// Download the playlist, if the URL refers to a video and a playlist
    #[arg(long, overrides_with = "no_playlist")]
    yes_playlist: bool,
    /// Do not extract a playlist's URL result entries; supported playlists
    /// carry fully extracted entries, which are processed either way
    #[arg(long, overrides_with = "no_flat_playlist")]
    flat_playlist: bool,
    /// Fully extract the videos of a playlist (default)
    #[arg(long, overrides_with = "flat_playlist")]
    no_flat_playlist: bool,
    /// Number of allowed failures until the rest of the playlist is skipped
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    skip_playlist_after_errors: Option<i64>,
    #[arg(long, hide = true)]
    id: bool,
    #[arg(long, hide = true)]
    print_json: bool,
    #[arg(long, hide = true)]
    all_subs: bool,
    #[arg(long, hide = true, default_value_t = 1, allow_hyphen_values = true)]
    autonumber_start: i64,
    #[arg(long, hide = true, allow_hyphen_values = true)]
    autonumber_size: Option<i64>,
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
    /// Download only videos uploaded on this date: YYYYMMDD or (now|today|yesterday)[-N(day|week|month|year)[s]]
    #[arg(
        long,
        value_name = "DATE",
        allow_hyphen_values = true,
        overrides_with = "date"
    )]
    date: Option<String>,
    /// Download only videos uploaded on or before this date
    #[arg(
        long,
        value_name = "DATE",
        allow_hyphen_values = true,
        overrides_with = "datebefore"
    )]
    datebefore: Option<String>,
    /// Download only videos uploaded on or after this date
    #[arg(
        long,
        value_name = "DATE",
        allow_hyphen_values = true,
        overrides_with = "dateafter"
    )]
    dateafter: Option<String>,
    /// Download only videos suitable for the given age
    #[arg(
        long,
        value_name = "YEARS",
        allow_hyphen_values = true,
        overrides_with = "age_limit"
    )]
    age_limit: Option<i64>,
    #[arg(
        long,
        hide = true,
        value_name = "COUNT",
        allow_hyphen_values = true,
        overrides_with = "min_views"
    )]
    min_views: Option<i64>,
    #[arg(
        long,
        hide = true,
        value_name = "COUNT",
        allow_hyphen_values = true,
        overrides_with = "max_views"
    )]
    max_views: Option<i64>,
    #[arg(
        long,
        hide = true,
        value_name = "REGEX",
        allow_hyphen_values = true,
        overrides_with = "match_title"
    )]
    match_title: Option<String>,
    #[arg(
        long,
        hide = true,
        value_name = "REGEX",
        allow_hyphen_values = true,
        overrides_with = "reject_title"
    )]
    reject_title: Option<String>,
    #[arg(long, hide = true)]
    break_on_reject: bool,
    /// Abort after processing NUMBER videos (0 means no limit)
    #[arg(long, value_name = "NUMBER", allow_hyphen_values = true)]
    max_downloads: Option<i64>,
    /// Ignore download and postprocessing errors; a video whose postprocessing fails still counts as downloaded
    #[arg(short = 'i', long, action = clap::ArgAction::Count)]
    ignore_errors: u8,
    /// Continue with the next video on download errors (default)
    #[arg(long, action = clap::ArgAction::Count)]
    no_abort_on_error: u8,
    /// Abort downloading of further videos if an error occurs
    #[arg(long, alias = "no-ignore-errors", action = clap::ArgAction::Count)]
    abort_on_error: u8,
    /// Maximum download rate in bytes per second, e.g. 50K or 4.2M
    #[arg(short = 'r', long, aliases = ["rate-limit"], value_name = "RATE")]
    limit_rate: Option<String>,
    /// Whether to emit color codes ([STREAM:]POLICY); this port never emits them
    #[arg(long, value_name = "[STREAM:]POLICY")]
    color: Vec<String>,
    #[arg(long, alias = "no-colours", hide = true)]
    no_colors: bool,
    /// Remove some internal metadata such as filenames from the infojson (default)
    #[arg(long, overrides_with = "no_clean_info_json")]
    clean_info_json: bool,
    /// Write all fields to the infojson
    #[arg(long, overrides_with = "clean_info_json")]
    no_clean_info_json: bool,
    /// Extractor names to use separated by commas (regexes, "all", "default", "end", "-" to exclude)
    #[arg(long, alias = "ies", value_name = "NAMES")]
    use_extractors: Vec<String>,
    #[arg(long, hide = true)]
    force_generic_extractor: bool,
    /// Prefix for unqualified URLs ("error", "fixup_error"; search prefixes are unsupported)
    #[arg(long, value_name = "PREFIX")]
    default_search: Option<String>,
    /// Make sure formats are selected only from those that are actually downloadable
    #[arg(long, overrides_with_all = ["check_formats", "check_all_formats", "no_check_formats"])]
    check_formats: bool,
    /// Check all formats for whether they are actually downloadable
    #[arg(long, overrides_with_all = ["check_formats", "check_all_formats", "no_check_formats"])]
    check_all_formats: bool,
    /// Do not check that the formats are actually downloadable
    #[arg(long, overrides_with_all = ["check_formats", "check_all_formats", "no_check_formats"])]
    no_check_formats: bool,
    /// List all supported extractors and the URLs they would handle
    #[arg(long)]
    list_extractors: bool,
    /// Output descriptions of all supported extractors
    #[arg(long)]
    extractor_descriptions: bool,
    /// Number of fragments of a dash/hlsnative video that should be downloaded concurrently
    #[arg(
        short = 'N',
        long,
        value_name = "N",
        default_value_t = 1,
        allow_hyphen_values = true
    )]
    concurrent_fragments: i64,
    /// Keep downloaded fragments on disk after downloading is finished
    #[arg(long, overrides_with = "no_keep_fragments")]
    keep_fragments: bool,
    /// Delete downloaded fragments after downloading is finished (default)
    #[arg(long, overrides_with = "keep_fragments")]
    no_keep_fragments: bool,
    /// Size of a chunk for chunk-based HTTP downloading, e.g. 10485760 or 10M
    #[arg(long, value_name = "SIZE")]
    http_chunk_size: Option<String>,
    /// Download only the first 10241 bytes of HTTP media (upstream's test mode)
    #[arg(long, hide = true)]
    test: bool,
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
    /// Seconds to sleep between requests during data extraction
    #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
    sleep_requests: Option<f64>,
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
        // Upstream `print_only`: print templates imply simulation unless a
        // later stage (`before_dl` onwards) prints; any printing implies quiet.
        let any_getting = self.dump_json
            || self.dump_single_json
            || self.get_title
            || self.get_id
            || self.get_url
            || self.get_filename
            || self.get_thumbnail
            || self.get_description
            || self.get_duration
            || self.get_format;
        let print_only = !self.print.is_empty()
            && self.print.iter().all(|spec| {
                !matches!(
                    yt_dlp_rs::output::print_stage_name(spec).as_str(),
                    "before_dl" | "post_process" | "after_move" | "after_video" | "playlist"
                )
            });
        let getting = any_getting || print_only;
        let listing = self.list_formats || self.list_subs || self.list_thumbnails;
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
            // Hidden `--print-json` is quiet JSON without simulation.
            quiet: quiet.unwrap_or(any_getting || self.print_json || !self.print.is_empty()),
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
    // `-o -`: the media goes to stdout.
    if templates.get(None) == "-" {
        return Ok(PathBuf::from("-"));
    }
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
            // Lazy playlists have no `requested_entries` upstream, and full
            // playlists drop it ("Do not set for full playlist").
            let full = counts.is_some_and(|counts| {
                indices
                    .iter()
                    .copied()
                    .eq((1..=counts.playlist_count).map(Some))
            });
            if !full && counts.is_none_or(|counts| counts.n_entries.is_some()) {
                object.insert("requested_entries".into(), serde_json::to_value(indices)?);
            }
            object.insert("entries".into(), entries.unwrap_or_default());
            Ok(value)
        }
    }
}

/// Python `str(float)` for ordinary values (`1.0`, `0.5`, `nan`, `inf`).
fn py_float(value: f64) -> String {
    yt_dlp_rs::screen::py_float(value)
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
        requests: Option<f64>,
    ) -> Result<Self, String> {
        for (name, value) in [
            ("subtitles sleep interval", subtitles),
            ("requests sleep interval", requests),
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
    let mut argv = std::env::args();
    let program = argv.next().unwrap_or_else(|| "yt-dlp-rs".into());
    let arguments = config_arguments(argv.collect())?;
    let (arguments, deprecated) = strip_deprecated_options(arguments);
    let arguments = expand_preset_aliases(arguments).map_err(UsageError)?;
    let matches = command().get_matches_from(std::iter::once(program).chain(arguments));
    if !deprecated.is_empty() {
        // Upstream `deprecated_feature`, written directly to stderr.
        eprintln!(
            "Deprecated Feature: The following options have been deprecated: {}\nPlease remove them from your command/configuration to avoid future errors.\nSee  https://github.com/yt-dlp/yt-dlp/issues/14198  for more details",
            deprecated.join(", ")
        );
    }
    let mut args = Args::from_arg_matches(&matches)?;
    // Upstream `parse_metadata` (shared by both options, in command-line
    // order per stage) plus the deprecated `--metadata-from-title`.
    let metadata_actions = {
        let mut ordered: Vec<(usize, Vec<String>)> = Vec::new();
        for (id, arity) in [("parse_metadata", 1), ("replace_in_metadata", 3)] {
            let indices: Vec<usize> = matches.indices_of(id).into_iter().flatten().collect();
            let values: Vec<String> = matches
                .get_many::<String>(id)
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            for (chunk, index) in values.chunks(arity).zip(indices.chunks(arity)) {
                ordered.push((index[0], chunk.to_vec()));
            }
        }
        ordered.sort_by_key(|(index, _)| *index);
        if let Some(format) = &args.metadata_from_title {
            ordered.push((usize::MAX, vec![format!("pre_process:title:{format}")]));
        }
        let mut actions = yt_dlp_rs::metadata_parser::MetadataActions::default();
        for (_, values) in ordered {
            match values.as_slice() {
                [value] => actions.add_interpret(value),
                [fields, search, replace] => actions.add_replace(fields, search, replace),
                _ => unreachable!("clap arity"),
            }
            .map_err(usage)?;
        }
        for when in yt_dlp_rs::metadata_parser::WHEN {
            if actions.has(when) && !matches!(when, "pre_process" | "after_filter" | "video") {
                return Err(usage(format!(
                    "metadata parsing at {when} is not supported natively; use pre_process, after_filter or video"
                ))
                .into());
            }
        }
        actions
    };
    // Upstream `allsubtitles`: every language, implying --write-subs unless
    // embedding or automatic captions are requested.
    if args.all_subs {
        args.sub_langs = vec!["all".into()];
        if !(args.embed_subs || args.write_auto_subs) {
            args.write_subs = true;
        }
    }
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
    // Upstream `ignoreerrors`: -i (True), --no-abort-on-error ("only_download",
    // the default) and --abort-on-error (False); the last flag wins.
    let error_mode = [
        ("ignore_errors", ErrorMode::Ignore),
        ("no_abort_on_error", ErrorMode::OnlyDownload),
        ("abort_on_error", ErrorMode::Abort),
    ]
    .into_iter()
    .filter(|(id, _)| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine))
    .flat_map(|(id, mode)| {
        matches
            .indices_of(id)
            .into_iter()
            .flatten()
            .map(move |i| (i, mode))
    })
    .max_by_key(|(index, _)| *index)
    .map_or(ErrorMode::OnlyDownload, |(_, mode)| mode);
    // Upstream `writethumbnail`: --write-thumbnail keeps an earlier "all",
    // --no-write-thumbnail clears it, --write-all-thumbnails sets "all".
    let thumbnail_mode = {
        let mut events: Vec<(usize, u8)> = Vec::new();
        for (id, kind) in [
            ("write_thumbnail", 1),
            ("no_write_thumbnail", 0),
            ("write_all_thumbnails", 2),
        ] {
            if matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine) {
                events.extend(
                    matches
                        .indices_of(id)
                        .into_iter()
                        .flatten()
                        .map(|i| (i, kind)),
                );
            }
        }
        events.sort();
        events.into_iter().fold(0u8, |state, (_, kind)| match kind {
            1 => state.max(1),
            other => other,
        })
    };
    let match_filters = MatchFilters::parse(
        &after_reset("match_filters", "no_match_filters"),
        &after_reset("break_match_filters", "no_break_match_filters"),
    )?;
    // Upstream `report_conflict` for --date, then `opts.date` resolution;
    // the warnings print only after validation succeeds.
    let now = epoch() as i64;
    let date = args.date.as_deref().filter(|date| !date.is_empty());
    let mut date_warnings = Vec::new();
    let (mut dateafter, mut datebefore) = (args.dateafter.as_deref(), args.datebefore.as_deref());
    if date.is_some() {
        for (name, value) in [
            ("--dateafter", &mut dateafter),
            ("--datebefore", &mut datebefore),
        ] {
            if value.take().is_some_and(|value| !value.is_empty()) {
                date_warnings.push(format!("WARNING: {name} is ignored since --date was given"));
            }
        }
    }
    let date_range = match date {
        Some(date) => Some(DateRange::day(date, now).map_err(usage)?),
        None if dateafter.is_some() || datebefore.is_some() => {
            Some(DateRange::new(dateafter, datebefore, now).map_err(usage)?)
        }
        None => None,
    };
    let title_pattern = |pattern: Option<&String>| {
        pattern
            .map(|pattern| EntrySelection::title_pattern(pattern))
            .transpose()
    };
    let match_filters = match_filters.with_selection(EntrySelection {
        match_title: title_pattern(args.match_title.as_ref())?,
        reject_title: title_pattern(args.reject_title.as_ref())?,
        date_range,
        min_views: args.min_views,
        max_views: args.max_views,
        age_limit: args.age_limit,
        break_on_reject: args.break_on_reject,
        now,
    });
    for warning in date_warnings {
        warning!("{warning}");
    }
    QUIET.store(args.simulation().quiet, Ordering::Relaxed);
    yt_dlp_rs::screen::set_sink(|message| match message {
        yt_dlp_rs::screen::Message::Screen(text) => screen!("{text}"),
        yt_dlp_rs::screen::Message::Warning(text) => warning!("WARNING: {text}"),
        yt_dlp_rs::screen::Message::Stderr(text) => eprintln!("{text}"),
    });
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
    let mut print_templates = args
        .print
        .iter()
        .map(|spec| PrintTemplate::parse(spec))
        .collect::<yt_dlp_rs::Result<Vec<_>>>()?;
    print_templates.extend(PrintTemplate::forced_fields(
        yt_dlp_rs::output::ForcedFields {
            title: args.get_title,
            id: args.get_id,
            url: args.get_url,
            thumbnail: args.get_thumbnail,
            description: args.get_description,
            filename: args.get_filename,
            duration: args.get_duration,
            format: args.get_format,
        },
    ));
    let print_files = args
        .print_to_file
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[spec, file]| PrintToFile::parse(spec, file))
        .collect::<yt_dlp_rs::Result<Vec<_>>>()?;
    let prints_stage = |stage: Stage| {
        print_templates
            .iter()
            .any(|template| template.stage() == stage)
            || print_files.iter().any(|pair| pair.stage() == stage)
    };
    // Like upstream, printing implies quiet simulation unless --no-simulate.
    let printing = !print_templates.is_empty();
    // `quiet` was applied to screen output at startup.
    let Simulation {
        simulate,
        list_only,
        ..
    } = args.simulation();
    let listing = args.list_formats || args.list_subs || args.list_thumbnails;
    let quiet_screen = QUIET.load(Ordering::Relaxed);
    let progress_options = ProgressOptions {
        enabled: if args.no_progress {
            false
        } else if args.progress {
            true
        } else {
            !quiet_screen
        },
        newline: args.newline,
        quiet: quiet_screen,
        delta: args
            .progress_delta
            .filter(|delta| *delta > 0.0 && delta.is_finite())
            .map(Duration::from_secs_f64),
    };
    let write_subs = args.write_subs || (args.embed_subs && !args.write_auto_subs);
    // Upstream `validate_regex('audio format', ...)` and
    // `validate_positive('audio quality', int_or_none(float_or_none(q), default=0))`.
    AudioOptions::from_cli(&args.audio_format, "5").map_err(|_| {
        usage(format!(
            "invalid audio format \"{}\" given",
            args.audio_format
        ))
    })?;
    if let Some(quality) = yt_dlp_rs::batch::py_strip(&args.audio_quality)
        .parse::<f64>()
        .ok()
        .filter(|quality| quality.is_finite() && quality.trunc() < 0.0)
    {
        return Err(usage(format!(
            "audio quality \"{}\" must be positive or 0",
            quality.trunc()
        ))
        .into());
    }
    // Upstream `-S` (`_list_from_options_callback` with append=-1): each value
    // is split on commas and prepended; `--format-sort-reset` clears earlier ones.
    let mut format_sort: Vec<String> = Vec::new();
    for value in after_reset("format_sort", "format_sort_reset") {
        let mut fields: Vec<String> = value
            .split(',')
            .map(|field| field.trim().to_owned())
            .filter(|field| !field.is_empty())
            .collect();
        fields.append(&mut format_sort);
        format_sort = fields;
    }
    static SORT_FIELD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^ *(?:\+?[a-zA-Z0-9_]+(?:[~:].*?)?)? *$").unwrap()
    });
    if let Some(field) = format_sort.iter().find(|field| !SORT_FIELD.is_match(field)) {
        return Err(usage(format!("invalid format sorting \"{field}\" given")).into());
    }
    // Upstream `validate_regex` uses `re.match`: only a valid prefix is required.
    if let Some(format) = &args.merge_output_format {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            let ext = "(?:avi|flv|mkv|mov|mp4|webm)";
            regex::Regex::new(&format!("^{ext}(?:/{ext})*")).unwrap()
        });
        if !PATTERN.is_match(format) {
            return Err(usage(format!("invalid merge output format \"{format}\" given")).into());
        }
    }
    let mut audio_options = AudioOptions::from_cli(&args.audio_format, &args.audio_quality)?;
    audio_options.keep_video = args.keep_video;
    // Source validation applies to both values even when recode takes precedence.
    let remux = args
        .remux_video
        .as_deref()
        .map(|format| {
            VideoOptions::from_cli(format, VideoMode::Remux)
                .map_err(|_| usage(format!("invalid remux video format \"{format}\" given")))
        })
        .transpose()?;
    let recode = args
        .recode_video
        .as_deref()
        .map(|format| {
            VideoOptions::from_cli(format, VideoMode::Recode)
                .map_err(|_| usage(format!("invalid recode video format \"{format}\" given")))
        })
        .transpose()?;
    if remux.is_some() && recode.is_some() {
        warning!("WARNING: --remux-video is ignored since --recode-video was given");
    }
    let processors = PostprocessOptions {
        ignore_errors: error_mode == ErrorMode::Ignore,
        thumbnail: None,
        metadata: args.embed_metadata,
        // Upstream: `--embed-metadata` implies `--embed-chapters` unless
        // `--no-embed-chapters` was given.
        chapters: args.embed_chapters || (args.embed_metadata && !args.no_embed_chapters),
        // Filled per video; see `embed_info_json` below.
        infojson: None,
        fixup: Some(yt_dlp_rs::FixupOptions {
            policy: args.fixup.clone(),
            hls_use_mpegts: args.hls_use_mpegts,
        }),
        audio: args.extract_audio.then_some(audio_options),
        video: recode.or(remux),
        embed: None,
        keep_video: args.keep_video,
    };
    let mut subtitle_conversion = args
        .convert_subs
        .as_deref()
        .map(|format| {
            SubtitleConversion::new(format)
                .map_err(|_| usage(format!("invalid subtitle format \"{format}\" given")))
        })
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
    // Upstream `report_conflict` between the playlist ordering options.
    let mut playlist_reverse = args.playlist_reverse;
    let mut playlist_random = args.playlist_random;
    if playlist_random && playlist_reverse {
        warning!("WARNING: --playlist-reverse is ignored since --playlist-random was given");
        playlist_reverse = false;
    }
    if args.lazy_playlist {
        for (enabled, name) in [
            (&mut playlist_reverse, "--playlist-reverse"),
            (&mut playlist_random, "--playlist-random"),
        ] {
            if *enabled {
                warning!("WARNING: {name} is ignored since --lazy-playlist was given");
                *enabled = false;
            }
        }
    }
    let playlist_selection = PlaylistSelection::new(
        args.playlist_items.as_deref(),
        args.playlist_start,
        args.playlist_end,
        playlist_reverse,
    )
    .map(|selection| selection.random(playlist_random).lazy(args.lazy_playlist))
    .map_err(|error| -> Box<dyn std::error::Error> {
        match (&error, &args.playlist_items) {
            // Upstream validates start/end, then parses the item segments.
            (yt_dlp_rs::Error::PlaylistSelection(message), _)
                if message.starts_with("playlist ") =>
            {
                usage(message).into()
            }
            (yt_dlp_rs::Error::PlaylistSelection(message), Some(items))
                if message.ends_with("is not a valid specification")
                    || message.ends_with("cannot be zero")
                    || message.ends_with("consecutive commas") =>
            {
                usage(format!(
                    "Invalid playlist-items {}: {message}",
                    yt_dlp_rs::batch::py_repr(items)
                ))
                .into()
            }
            _ => error.into(),
        }
    })?;
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
            no_playlist: args.no_playlist,
        },
        http_options,
    )?;
    let (ffmpeg, ffprobe) = ffmpeg_executables(args.ffmpeg_location.as_deref());
    if let Some(path) = ffmpeg {
        downloader = downloader.with_ffmpeg(path);
    }
    if let Some(path) = args.ffprobe_location.or(ffprobe) {
        downloader = downloader.with_ffprobe(path);
    }
    downloader = downloader.with_format_sort(yt_dlp_rs::FormatSortOptions {
        fields: format_sort,
        force: args.format_sort_force,
        prefer_free: args.prefer_free_formats,
    });
    if let Some(formats) = &args.merge_output_format {
        downloader =
            downloader.with_merge_output_format(formats.split('/').map(str::to_owned).collect());
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
    // Upstream `parse_chapters('--download-sections', ..., advanced=True)`.
    let download_sections =
        yt_dlp_rs::sections::DownloadSections::parse(&args.download_sections).map_err(usage)?;
    if args.concurrent_fragments <= 0 {
        return Err(usage(format!(
            "concurrent fragments \"{}\" must be positive",
            args.concurrent_fragments
        ))
        .into());
    }
    // Upstream `validate_positive` for the autonumber options.
    if args.autonumber_start < 0 {
        return Err(usage(format!(
            "autonumber start \"{}\" must be positive or 0",
            args.autonumber_start
        ))
        .into());
    }
    if let Some(size) = args.autonumber_size.filter(|size| *size <= 0) {
        return Err(usage(format!("autonumber size \"{size}\" must be positive")).into());
    }
    // Upstream `--id`: the default template becomes `%(id)s.%(ext)s` unless
    // `-o` sets one (`report_conflict`).
    if args.id {
        if OutputTemplates::sets_default(&args.output) {
            warning!("WARNING: --id is ignored since --output was given");
        } else {
            warning!("WARNING: --id is deprecated. Use -o \"%(id)s.%(ext)s\" instead");
            args.output.push("%(id)s.%(ext)s".into());
        }
    }
    let templates = OutputTemplates::parse(&args.output, args.restrict_filenames);
    LOG_TO_STDERR.store(templates.get(None) == "-", Ordering::Relaxed);
    let downloader = downloader
        .with_prefer_best_format(templates.get(None) == "-")
        .with_external_quiet(QUIET.load(Ordering::Relaxed))
        .with_hls_use_mpegts(args.hls_use_mpegts);
    let output_paths = yt_dlp_rs::output::parse_paths(&args.paths)?;
    let template_options = TemplateOptions {
        na_placeholder: args.output_na_placeholder,
        filename_mode: TemplateOptions::filename_mode(args.restrict_filenames, windows_filenames),
        windows_filenames: windows_filenames == Some(true),
        trim_file_name: args.trim_filenames,
        paths: output_paths,
        autonumber: Default::default(),
        autonumber_size: args.autonumber_size.map_or(5, |size| size as usize),
        video_autonumber: Default::default(),
    };
    // Upstream `exec_cmd`: --no-exec clears earlier values; the hidden
    // --exec-before-download is ignored when "--exec before_dl:" is given.
    let exec_values = after_reset("exec", "no_exec");
    let before_download = after_reset("exec_before_download", "no_exec_before_download");
    let mut exec_specs: Vec<(Option<&str>, &str)> = exec_values
        .iter()
        .map(|value| (None, value.as_str()))
        .collect();
    let mut exec = ExecCommands::parse(&exec_specs, &template_options)?;
    if !before_download.is_empty() {
        if exec.has(Stage::BeforeDownload) {
            warning!(
                "WARNING: --exec-before-download is ignored since \"--exec before_dl:\" was given"
            );
        } else {
            exec_specs.extend(
                before_download
                    .iter()
                    .map(|value| (Some("before_dl"), value.as_str())),
            );
            exec = ExecCommands::parse(&exec_specs, &template_options)?;
        }
    }
    // Like upstream, simulation never writes info JSON.
    let writes_info_json = args.write_info_json && !simulate;
    // Upstream `embed_infojson`: explicit, or `if_exists` with --embed-metadata.
    let embed_info_json = if args.embed_info_json {
        Some(true)
    } else if args.no_embed_info_json || !args.embed_metadata {
        None
    } else {
        Some(false)
    };
    let writes_description = args.write_description && !simulate;
    // Upstream `write_links`: --write-link picks the platform's type.
    let platform_link = if cfg!(target_os = "macos") {
        "webloc"
    } else if cfg!(target_os = "linux") {
        "desktop"
    } else {
        "url"
    };
    let link_kinds: Vec<&str> = [
        ("url", args.write_url_link),
        ("webloc", args.write_webloc_link),
        ("desktop", args.write_desktop_link),
    ]
    .into_iter()
    .filter(|(kind, enabled)| *enabled || (args.write_link && *kind == platform_link))
    .map(|(kind, _)| kind)
    .collect();
    let writes_links = !link_kinds.is_empty() && !simulate;
    // Upstream --embed-thumbnail writes the thumbnail when not otherwise asked
    // and then deletes it after embedding.
    let keeps_thumbnails = thumbnail_mode != 0;
    let thumbnail_mode = if args.embed_thumbnail {
        thumbnail_mode.max(1)
    } else {
        thumbnail_mode
    };
    let writes_thumbnails = thumbnail_mode != 0 && !simulate;
    let thumbnail_conversion = args
        .convert_thumbnails
        .as_deref()
        .map(|format| {
            ThumbnailConversion::from_cli(format)
                .map_err(|_| usage(format!("invalid thumbnail format \"{format}\" given")))
        })
        .transpose()?
        .flatten();
    let playlist_files = !args.no_write_playlist_metafiles;
    let sleep = SleepPolicy::from_cli(
        args.sleep_interval,
        args.max_sleep_interval,
        args.sleep_subtitles,
        args.sleep_requests,
    )
    .map_err(usage)?;
    if let Some(seconds) = args.sleep_requests {
        yt_dlp_rs::screen::set_sleep_requests(seconds);
    }
    // Upstream `parse_retries` and `parse_sleep_func`.
    let parse_retries = |name: &str, value: &str| -> Result<u32, UsageError> {
        let text = yt_dlp_rs::batch::py_strip(value);
        if matches!(text, "inf" | "infinite") {
            return Ok(u32::MAX);
        }
        let digits = text.strip_prefix('+').unwrap_or(text);
        let negative = digits.starts_with('-');
        let magnitude = digits.trim_start_matches('-');
        if magnitude.is_empty()
            || digits.matches('-').count() > 1
            || !magnitude.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(usage(format!(
                "invalid {name} retry count \"{value}\" given"
            )));
        }
        if negative && magnitude.bytes().any(|b| b != b'0') {
            return Err(usage(format!(
                "{name} retry count \"{}\" must be positive or 0",
                text.trim_start_matches('+')
            )));
        }
        Ok(magnitude
            .parse::<u64>()
            .map_or(u32::MAX - 1, |n| n.min(u64::from(u32::MAX - 1)) as u32))
    };
    let retries = parse_retries("download", &args.retries)?;
    let fragment_retries = parse_retries("fragment", &args.fragment_retries)?;
    let mut retry_sleep = yt_dlp_rs::RetrySleep::default();
    for spec in &args.retry_sleep {
        let (key, expression) = match spec.split_once(':') {
            Some((key, rest))
                if ["http", "fragment", "file_access", "extractor"]
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(key)) =>
            {
                (key.to_ascii_lowercase(), rest)
            }
            _ => ("http".to_owned(), spec.as_str()),
        };
        let parsed = if expression.is_empty() {
            None
        } else {
            Some(
                yt_dlp_rs::SleepExpression::parse(expression).ok_or_else(|| {
                    usage(format!(
                        "invalid {key} retry sleep expression {}",
                        yt_dlp_rs::batch::py_repr(expression)
                    ))
                })?,
            )
        };
        match key.as_str() {
            "http" => retry_sleep.http = parsed,
            "fragment" => retry_sleep.fragment = parsed,
            // File access and extractor retries are not represented.
            _ => {}
        }
    }
    let skip_unavailable_fragments = Some(!args.abort_on_unavailable_fragments);
    // Upstream `continuedl` and `nopart` for whole-file media downloads.
    let part_files = if args.no_part {
        yt_dlp_rs::PartFiles::NoPart
    } else {
        yt_dlp_rs::PartFiles::Part {
            resume: !args.no_continue,
        }
    };
    let min_filesize = size_option("min filesize", args.min_filesize.as_deref()).map_err(usage)?;
    let max_filesize = size_option("max filesize", args.max_filesize.as_deref()).map_err(usage)?;
    // Upstream validates the rate limit as strictly positive.
    let rate_limit = size_option("rate limit", args.limit_rate.as_deref()).map_err(usage)?;
    if rate_limit == Some(0) {
        return Err(usage("rate limit \"0\" must be positive").into());
    }
    let http_chunk_size =
        size_option("http chunk size", args.http_chunk_size.as_deref()).map_err(usage)?;
    let downloader = downloader
        .with_http_chunk_size(http_chunk_size)
        .with_test(args.test)
        .with_force_generic_extractor(args.force_generic_extractor)
        .with_concurrent_fragments(args.concurrent_fragments as usize)
        .with_keep_fragments(args.keep_fragments)
        .with_check_formats(if args.check_formats {
            yt_dlp_rs::CheckFormats::Selected
        } else if args.check_all_formats {
            yt_dlp_rs::CheckFormats::All
        } else if args.no_check_formats {
            yt_dlp_rs::CheckFormats::Never
        } else {
            yt_dlp_rs::CheckFormats::Default
        })
        .with_default_search(args.default_search.clone())
        // Upstream `_list_from_options_callback`: comma-separated, stripped.
        .with_allowed_extractors(
            &args
                .use_extractors
                .iter()
                .flat_map(|value| value.split(','))
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        )
        .map_err(usage)?;
    // Upstream `--color` validation; output is always uncolored.
    for value in &args.color {
        let policy = match value.split_once(':') {
            Some((stream, policy))
                if matches!(stream.to_lowercase().as_str(), "stdout" | "stderr") =>
            {
                policy
            }
            _ => value.as_str(),
        }
        .trim();
        if !matches!(
            policy,
            "always" | "auto" | "auto-tty" | "no_color" | "no_color-tty" | "never"
        ) {
            return Err(usage(format!("\"{policy}\" is not a valid color policy")).into());
        }
    }
    // Upstream `print_extractor_information`, for this port's extractors
    // (sorted, `generic` last); the generic extractor lists unmatched URLs.
    if args.list_extractors {
        let mut out = io::stdout().lock();
        let (youtube, generic): (Vec<&String>, Vec<&String>) = urls
            .iter()
            .partition(|url| yt_dlp_rs::extractor::youtube_suitable(url));
        writeln!(out, "html5")?;
        writeln!(out, "youtube")?;
        for url in youtube {
            writeln!(out, "  {url}")?;
        }
        writeln!(out, "generic")?;
        for url in generic {
            writeln!(out, "  {url}")?;
        }
        return Ok(true);
    }
    if args.extractor_descriptions {
        // Upstream writes the joined descriptions without a final newline.
        write!(
            io::stdout().lock(),
            "html5\nyoutube: [youtube] YouTube\ngeneric: Generic downloader that works on some sites"
        )?;
        return Ok(true);
    }
    // Upstream reports a missing URL after validating the options.
    if urls.is_empty() && args.load_info_json.is_none() {
        return Err(UsageError(
            "You must provide at least one URL.\nType yt-dlp-rs --help to see a list of all options."
                .into(),
        )
        .into());
    }
    // Upstream compares against `float(max_downloads or 'inf')`.
    let max_downloads = args.max_downloads.filter(|&limit| limit != 0);
    let abort_on_error = error_mode == ErrorMode::Abort;
    let mut aborted = false;
    let mut processed: i64 = 0;
    let mut videos: u64 = 0;
    let mut cancelled: Option<Cancel> = None;
    let mut failed = false;
    // Upstream `_out_files.out`: JSON, listings and prints use stderr with
    // `logtostderr` (`-o -`), whose stdout carries the media.
    let mut stdout = out_stream();
    // Upstream `download_with_info_file` replaces the URLs.
    let inputs: Vec<(String, Option<ExtractResult>)> = match &args.load_info_json {
        Some(file) => {
            if !urls.is_empty() {
                warning!("WARNING: URLs are ignored due to --load-info-json");
            }
            let path = yt_dlp_rs::batch::expand_path_text(file);
            let mut loaded = yt_dlp_rs::load_info_json(std::path::Path::new(&path))?;
            // Upstream runs process_video_result (fills, sorting, IDs) again.
            for result in &mut loaded {
                downloader.process_result(result)?;
            }
            loaded
                .into_iter()
                .map(|result| (path.clone(), Some(result)))
                .collect()
        }
        None => urls.into_iter().map(|url| (url, None)).collect(),
    };
    for (url, loaded) in inputs {
        let options = ExtractionOptions {
            retries: args.extractor_retries,
        };
        let mut pending: Option<Cancel> = None;
        let extracted =
            (|| -> yt_dlp_rs::Result<Option<(ExtractResult, Option<PlaylistCounts>)>> {
                let mut result = match loaded {
                    // Loaded playlists have no entries (`sanitize_info`), so
                    // upstream's `EntryNotInPlaylist` falls back to the URL.
                    Some(ExtractResult::Playlist(playlist)) => {
                        let title = [&playlist.title, &playlist.id]
                            .into_iter()
                            .find(|text| !text.is_empty())
                            .map_or("<Untitled>", |text| text.as_str());
                        screen!("[download] Downloading playlist: {title}");
                        warning!(
                            "WARNING: The info failed to download: There are no entries; trying with URL {}",
                            playlist.webpage_url
                        );
                        downloader.extract_result_unselected_with_options(
                            playlist.webpage_url.as_str(),
                            options,
                        )?
                    }
                    Some(result) => result,
                    None => downloader.extract_result_unselected_with_options(&url, options)?,
                };
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
                match &error {
                    yt_dlp_rs::Error::Reported(message) => eprintln!("ERROR: {message}"),
                    _ => eprintln!("ERROR: {url}: {error}"),
                }
                failed = true;
                if abort_on_error {
                    break;
                }
                continue;
            }
        };
        // Upstream `__process_playlist` progress lines use the playlist
        // title (or ID).
        let playlist_title = match &extracted {
            ExtractResult::Playlist(playlist) => Some(
                [&playlist.title, &playlist.id]
                    .into_iter()
                    .find(|text| !text.is_empty())
                    .map_or("<Untitled>".to_owned(), |text| text.to_string()),
            ),
            ExtractResult::Video(_) => None,
        };
        if let Some(title) = &playlist_title {
            screen!("[download] Downloading playlist: {title}");
        }
        if let (ExtractResult::Playlist(playlist), Some(counts), true) = (
            &extracted,
            counts,
            playlist_files && (writes_info_json || writes_description),
        ) {
            // Upstream writes playlist metadata before processing any entry,
            // and a failed write skips the playlist.
            let written = (|| -> Result<(), Box<dyn std::error::Error>> {
                if writes_info_json
                    && let Some(path) = templates.prepare_playlist(
                        playlist,
                        counts,
                        "pl_infojson",
                        &template_options,
                    )?
                {
                    reject_reserved(&path, [&archive_path, &args.cookies])?;
                    let value = yt_dlp_rs::playlist_info_json_with(
                        playlist,
                        counts,
                        epoch(),
                        !args.no_clean_info_json,
                    )?;
                    let written = write_info_json_with(&path, &value, overwrites != Some(false))?;
                    report_info_json("playlist", &path, written);
                }
                if writes_description
                    && templates
                        .prepare_playlist(playlist, counts, "pl_description", &template_options)?
                        .is_some()
                {
                    // Playlist results carry no represented description.
                    screen!("[info] There's no playlist description to write");
                }
                Ok(())
            })();
            if let Err(error) = written {
                eprintln!("ERROR: {url}: {error}");
                failed = true;
                if abort_on_error {
                    break;
                }
                continue;
            }
        }
        if let (ExtractResult::Playlist(playlist), true) =
            (&extracted, args.list_thumbnails && playlist_files)
        {
            // Playlist results carry no represented thumbnails.
            screen!("{} has no thumbnails", playlist.id);
        }
        if let (ExtractResult::Playlist(playlist), Some(counts), Some(title)) =
            (&extracted, counts, &playlist_title)
        {
            screen!(
                "[{}] Playlist {title}: Downloading {} items of {}",
                playlist.extractor,
                counts
                    .n_entries
                    .map_or_else(|| "N/A".into(), |count| count.to_string()),
                counts.playlist_count
            );
        }
        // Upstream `-J` prints the processed entries (`null` for entries whose
        // processing failed) after the playlist.
        let mut single_json = args
            .dump_single_json
            .then(|| single_json(&extracted, counts))
            .transpose()?;
        let mut json_entries = single_json
            .as_mut()
            .and_then(|value| value.get_mut("entries"))
            .map(serde_json::Value::take);
        let mut url_cancelled = false;
        // Upstream `__process_playlist` `failures`.
        let mut extraction_failures = 0;
        for (entry_index, mut info) in extracted.into_entries().into_iter().enumerate() {
            let mut counted = false;
            let postprocess_failed = std::cell::Cell::new(false);
            // Upstream `__pending_error` from a failed preprocessing command.
            let pending_error = std::cell::Cell::new(None::<String>);
            let defer = |error: String| {
                eprintln!("ERROR: Preprocessing: {error}");
                let earlier = pending_error.take();
                pending_error.set(earlier.or(Some(format!("Preprocessing: {error}"))));
            };
            let mut entry_cancel = None;
            // Upstream `_num_videos` (`process_video_result`).
            videos += 1;
            template_options.video_autonumber.set(Some(videos));
            if let Some(index) = info.playlist_autonumber
                && playlist_title.is_some()
            {
                let count = info
                    .n_entries
                    .map_or_else(|| "N/A".into(), |count| count.to_string());
                screen!("[download] Downloading item {index} of {count}");
            }
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                // Upstream `process_video_result`: `--check-all-formats` tests
                // every format first.
                downloader.check_all_formats(&mut info);
                // Upstream `process_video_result`: `pre_process` and
                // `after_filter` processors precede listing and selection.
                for when in ["pre_process", "after_filter"] {
                    run_metadata_parser(&metadata_actions, when, &mut info, &template_options)?;
                }
                // Upstream lists thumbnails, then subtitles, then formats.
                if args.list_thumbnails {
                    emit_listing(&mut stdout, &yt_dlp_rs::output::render_thumbnails(&info))?;
                }
                if args.list_subs {
                    if !info.automatic_captions.is_empty() {
                        emit_listing(&mut stdout, &render_automatic_captions(&info))?;
                    }
                    emit_listing(&mut stdout, &render_subtitles(&info))?;
                }
                if args.list_formats {
                    emit_listing(
                        &mut stdout,
                        &yt_dlp_rs::output::render_formats_listing(&info, !args.list_formats_old),
                    )?;
                }
                // Upstream `process_video_result` format selection.
                if !list_only {
                    downloader.select_video(&mut info, format).map_err(
                        |error| -> Box<dyn std::error::Error> {
                            match error {
                                yt_dlp_rs::Error::FormatSelection(_) => {
                                    Box::new(EntryExtractionError(format!(
                                        "[{}] {}: Requested format is not available. \
                                         Use --list-formats for a list of available formats",
                                        info.extractor, info.id
                                    )))
                                }
                                other => Box::new(other),
                            }
                        },
                    )?;
                }
                if let Some(entry) = json_entries
                    .as_mut()
                    .and_then(|entries| entries.get_mut(entry_index))
                {
                    *entry = serde_json::to_value(&info)?;
                } else if let Some(value @ Some(_)) =
                    (playlist_title.is_none() && args.dump_single_json).then_some(&mut single_json)
                {
                    *value = Some(serde_json::to_value(&info)?);
                }
                if listing && !args.dump_json && !printing && simulate {
                    return Ok(());
                }
                // Upstream `process_video_result` after format selection.
                let format_ids = if info.requested_formats.is_empty() {
                    info.format_id.clone().unwrap_or_else(|| info.ext.clone())
                } else {
                    info.requested_formats
                        .iter()
                        .map(|format| format.format_id.as_str())
                        .collect::<Vec<_>>()
                        .join("+")
                };
                // Upstream `download_ranges`: every requested section is
                // processed as its own info dict (none when no chapter matches).
                let requested = download_sections.requested(&info);
                if requested.is_empty() {
                    return Ok(());
                }
                screen!("[info] {}: Downloading 1 format(s): {format_ids}", info.id);
                if requested != [None] {
                    screen!(
                        "[info] {}: Downloading {} time ranges: {}",
                        info.id,
                        requested.len(),
                        yt_dlp_rs::sections::describe(&requested)
                    );
                }
                let base = info.clone();
                for section in &requested {
                    let mut info = yt_dlp_rs::sections::apply(&base, section.as_ref());
                    (|| -> Result<(), Box<dyn std::error::Error>> {
                        // Upstream `process_info` rechecks complete metadata before counting.
                        if !match_filters.is_empty() {
                            match match_filters.check(&info, false)? {
                                MatchOutcome::Pass => {}
                                MatchOutcome::Reject(reason) => {
                                    screen!("[download] {reason}");
                                    return Ok(());
                                }
                                MatchOutcome::Break(reason) => {
                                    screen!("[download] {reason}");
                                    entry_cancel = Some(Cancel::Rejected);
                                    return Ok(());
                                }
                            }
                        }
                        // Upstream `pre_process(info, 'video')` before counting.
                        run_metadata_parser(
                            &metadata_actions,
                            "video",
                            &mut info,
                            &template_options,
                        )?;
                        // Upstream `process_info` counts every processed video.
                        counted = true;
                        template_options.autonumber.set(Some(
                            (args.autonumber_start - 1 + processed + 1).max(0) as u64,
                        ));
                        if let Err(error) =
                            run_exec(&exec, Stage::Video, &info, None, None, &template_options)
                        {
                            defer(error);
                        }
                        // `_forceprint`: `filename` is the prepared media filename.
                        let print_filename = prepare_media(&templates, &info, &template_options)
                            .map(|path| path.to_string_lossy().into_owned());
                        let stage_print = |stage: Stage, paths: StagePaths<'_>| {
                            render_stage(
                                stage,
                                &print_templates,
                                &print_files,
                                &info,
                                print_filename.as_deref(),
                                paths,
                                &template_options,
                            )
                        };
                        stage_print(
                            Stage::Video,
                            StagePaths {
                                filepath: None,
                                prepared: print_filename.as_deref().ok(),
                            },
                        )?
                        .emit()?;
                        // Upstream `forcejson`: `--dump-json` or `--print-json`.
                        if args.dump_json || args.print_json {
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
                            force_record(&info)?;
                        } else {
                            if args.skip_download
                                && !writes_info_json
                                && !writes_description
                                && !writes_thumbnails
                                && !exec.has(Stage::BeforeDownload)
                                && !writes_links
                                && !prints_stage(Stage::BeforeDownload)
                                && info
                                    .requested_subtitles
                                    .as_ref()
                                    .is_none_or(|subtitles| subtitles.is_empty())
                            {
                                force_record(&info)?;
                                return Ok(());
                            }
                            let path = prepare_media(&templates, &info, &template_options)?;
                            if !template_options.paths.is_empty()
                                && path_is_absolute_template(&templates)
                            {
                                warning!(
                                    "WARNING: --paths is ignored since an absolute path is given in output template"
                                );
                            }
                            let processing = processors.audio.is_some()
                                || processors.video.is_some()
                                || args.embed_subs
                                || processors.metadata
                                || processors.chapters
                                || embed_info_json.is_some()
                                || args.split_chapters
                                || args.embed_thumbnail;
                            let existing_media = !args.skip_download && path_exists(&path)?;
                            if existing_media && overwrites != Some(true) && processing {
                                // Upstream would rerun postprocessors on the existing file.
                                return Err(yt_dlp_rs::Error::Postprocess(format!(
                            "{} already exists; postprocessing an existing download is unsupported (use --force-overwrites)",
                            path.display()
                        ))
                        .into());
                            }
                            // Upstream writes the description before subtitles and info JSON.
                            if writes_description
                                && let Some(path) = templates.prepare(
                                    &info,
                                    Some("description"),
                                    &template_options,
                                )?
                            {
                                write_description(&path, &info, overwrites)?;
                            }
                            // Plan source, intermediate and final names before subtitle I/O.
                            let has_subtitles = info
                                .requested_subtitles
                                .as_ref()
                                .is_some_and(|subtitles| !subtitles.is_empty());
                            let sidecars = match templates.prepare(
                                &info,
                                Some("subtitle"),
                                &template_options,
                            )? {
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
                                            "subtitle conversion or info JSON filenames collide"
                                                .into(),
                                        )
                                        .into());
                                    }
                                    for reserved in
                                        [&archive_path, &args.cookies].into_iter().flatten()
                                    {
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
                                            Err(error)
                                                if error.kind() == io::ErrorKind::NotFound => {}
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
                                            "[info] Video subtitle {}.{} is already present",
                                            language,
                                            subtitle.ext
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
                                            "[info] Writing video subtitles to: {}",
                                            subtitle_path.display()
                                        );
                                        sleep.subtitle();
                                        // Upstream `dl(..., subtitle=True)`: the file
                                        // downloader announces and reports the sidecar.
                                        screen!(
                                            "[download] Destination: {}",
                                            subtitle_path.display()
                                        );
                                        let mut reporter = ProgressReporter::new(&progress_options);
                                        downloader.download_subtitle_with_progress(
                                            &info,
                                            &subtitle,
                                            &subtitle_path,
                                            DownloadOptions {
                                                retries,
                                                update_time: args.mtime,
                                                ..Default::default()
                                            },
                                            |progress| {
                                                reporter.event(&progress);
                                                ControlFlow::Continue(())
                                            },
                                        )?;
                                        reporter.finish();
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
                            // Upstream writes thumbnails after subtitles and before info JSON.
                            let mut thumbnail_files = Vec::new();
                            if writes_thumbnails {
                                thumbnail_files = write_thumbnails(
                                    &downloader,
                                    &info,
                                    templates.prepare(
                                        &info,
                                        Some("thumbnail"),
                                        &template_options,
                                    )?,
                                    thumbnail_mode == 2,
                                    overwrites,
                                    retries,
                                )?;
                            }
                            if let Some(conversion) = &thumbnail_conversion {
                                convert_thumbnails(
                                    &downloader,
                                    &mut thumbnail_files,
                                    conversion,
                                    args.keep_video,
                                )?;
                            }
                            // Upstream writes info JSON after subtitles and before media.
                            if let Some(info_json) = &info_json {
                                let value = yt_dlp_rs::video_info_json_with(
                                    &info,
                                    epoch(),
                                    !args.no_clean_info_json,
                                )?;
                                let written = write_info_json_with(
                                    info_json,
                                    &value,
                                    overwrites != Some(false),
                                )?;
                                report_info_json("video", info_json, written);
                            }
                            // Upstream writes internet shortcuts after info JSON.
                            if writes_links {
                                write_links(
                                    &link_kinds,
                                    &info,
                                    &templates,
                                    &template_options,
                                    overwrites,
                                )?;
                            }
                            let prepared = path.to_string_lossy().into_owned();
                            stage_print(Stage::BeforeDownload, stage_paths(None, &prepared))?
                                .emit()?;
                            if let Err(error) = run_exec(
                                &exec,
                                Stage::BeforeDownload,
                                &info,
                                None,
                                Some(&path),
                                &template_options,
                            ) {
                                defer(error);
                            }
                            // Upstream prints `post_process` before merging/processing
                            // and `after_move` with the absolute moved path.
                            let post_print = stage_print(
                                Stage::PostProcess,
                                stage_paths(Some(&prepared), &prepared),
                            )?;
                            let move_print = |filepath: &std::path::Path| -> Result<(), String> {
                                let filepath = py_abspath(filepath).to_string_lossy().into_owned();
                                stage_print(
                                    Stage::AfterMove,
                                    stage_paths(Some(&filepath), &prepared),
                                )
                                .map_err(|error| error.to_string())?
                                .emit()
                                .map_err(|error| error.to_string())
                            };

                            if args.skip_download {
                                force_record(&info)?;
                                return Ok(());
                            }
                            if existing_media {
                                if overwrites == Some(true) {
                                    screen!("[download] Deleting existing file {}", path.display());
                                    std::fs::remove_file(&path)?;
                                } else {
                                    screen!(
                                        "[download] {} has already been downloaded",
                                        path.display()
                                    );
                                    post_print.emit()?;
                                    if !after_download(
                                        &exec,
                                        &info,
                                        &path,
                                        &path,
                                        &template_options,
                                        error_mode,
                                        &pending_error,
                                        &postprocess_failed,
                                        &move_print,
                                    ) {
                                        return Ok(());
                                    }
                                    if let Some(archive) = &mut archive {
                                        archive.record(&info).map_err(|error| {
                                            format!("archive update failed: {error}")
                                        })?;
                                    }
                                    return Ok(());
                                }
                            }
                            // Upstream `_catch_unsafe_extension_error` around the
                            // download's `replace_extension` calls.
                            if let Some(ext) = std::iter::once(&info.ext)
                                .chain(info.requested_formats.iter().map(|format| &format.ext))
                                .find(|ext| !yt_dlp_rs::output::allowed_extension(ext))
                            {
                                return Err(format!(
                            "The extracted extension ({}) is unusual and will be skipped for safety reasons. If you believe this is an error, please report it",
                            yt_dlp_rs::batch::py_repr(ext)
                        )
                        .into());
                            }
                            let partial = yt_dlp_rs::sections::is_partial(&info);
                            // Upstream `process_info`: a partial download needs FFmpegFD.
                            if partial && !downloader.has_ffmpeg() {
                                return Err(yt_dlp_rs::Error::Reported(
                            "You have requested downloading the video partially, but ffmpeg is not installed. Aborting"
                                .into(),
                        )
                        .into());
                            }
                            if !partial
                                && !info.requested_formats.is_empty()
                                && !downloader.has_ffmpeg()
                            {
                                let message = "You have requested merging of multiple formats but ffmpeg is not installed";
                                if error_mode == ErrorMode::Abort {
                                    return Err(format!(
                                        "{message}. Aborting due to --abort-on-error"
                                    )
                                    .into());
                                }
                                warning!("WARNING: {message}. The formats won't be merged");
                            }
                            // Upstream `HlsFD` refetches the media playlist first.
                            let refreshed = if !partial && info.requested_formats.is_empty() {
                                downloader.refresh_hls(&info)?
                            } else {
                                None
                            };
                            let info = refreshed.as_ref().unwrap_or(&info);
                            if !partial
                                && info.requested_formats.is_empty()
                                && !info.fragments.is_empty()
                            {
                                // Upstream `_prepare_frag_download`.
                                let downloader = match info.protocol.as_deref() {
                                    Some("http_dash_segments") => "dashsegments",
                                    _ => "hlsnative",
                                };
                                screen!("[{downloader}] Total fragments: {}", info.fragments.len());
                            }
                            // Merges announce each format's file instead, except
                            // FFmpegFD merging a partial download into one file.
                            if info.requested_formats.is_empty() || partial {
                                screen!("[download] Destination: {}", path.display());
                            }
                            // Upstream sleeps before each stream's FileDownloader.download.
                            for _ in 0..info.requested_formats.len().max(1) {
                                sleep.media();
                            }
                            let post_printed = std::cell::Cell::new(false);
                            let mut reporter = ProgressReporter::new(&progress_options);
                            let mut progress = |progress: yt_dlp_rs::DownloadProgress| {
                                reporter.event(&progress);
                                if matches!(
                                    progress.phase,
                                    DownloadPhase::Merging | DownloadPhase::Postprocessing
                                ) && !post_printed.replace(true)
                                    && post_print.emit().is_err()
                                {
                                    return ControlFlow::Break(());
                                }
                                ControlFlow::Continue(())
                            };
                            let mut processors = processors.clone();
                            if let Some(explicit) = embed_info_json {
                                processors.infojson = Some(yt_dlp_rs::InfoJsonEmbed {
                                    written: info_json.clone(),
                                    explicit,
                                    json: yt_dlp_rs::to_python_json(
                                        &yt_dlp_rs::video_info_json_with(
                                            info,
                                            epoch(),
                                            !args.no_clean_info_json,
                                        )?,
                                    )?,
                                });
                            }
                            // Upstream embeds the best thumbnail on disk.
                            let mut embedded_thumbnail = None;
                            if args.embed_thumbnail {
                                if info.metadata.thumbnails.is_empty() {
                                    screen!(
                                        "[EmbedThumbnail] There aren't any thumbnails to embed"
                                    );
                                } else if let Some((_, path)) = thumbnail_files.last() {
                                    let path = yt_dlp_rs::fixup_webp(path)?;
                                    processors.thumbnail = Some(path.clone());
                                    embedded_thumbnail = Some(path);
                                } else {
                                    screen!("[EmbedThumbnail] There are no thumbnails on disk");
                                }
                            }
                            if args.embed_subs {
                                processors.embed = Some(SubtitleEmbedding {
                                    subtitles: embedding_files,
                                    keep_subtitles: args.write_subs || args.keep_video,
                                });
                            }
                            let processing = processors.audio.is_some()
                                || processors.video.is_some()
                                || processors.embed.is_some()
                                || processors.metadata
                                || processors.chapters
                                || processors.infojson.is_some()
                                || processors.thumbnail.is_some();
                            if path.as_os_str() == "-" {
                                // Upstream streams to stdout and skips postprocessing,
                                // fixups, the archive record and after-move hooks.
                                let streams: Vec<yt_dlp_rs::VideoInfo> = if info
                                    .requested_formats
                                    .is_empty()
                                {
                                    vec![info.clone()]
                                } else {
                                    let reason = if downloader.has_ffmpeg() {
                                        return Err(
                                    "merging multiple formats to stdout through ffmpeg is unsupported"
                                        .into(),
                                );
                                    } else {
                                        "but ffmpeg is not installed"
                                    };
                                    warning!(
                                        "WARNING: You have requested downloading multiple formats to stdout {reason}. The formats will be streamed one after the other"
                                    );
                                    info.requested_formats
                                        .iter()
                                        .map(|format| info.with_selected_format(format))
                                        .collect()
                                };
                                for stream in &streams {
                                    let refreshed = if info.requested_formats.is_empty() {
                                        None
                                    } else {
                                        downloader.refresh_hls(stream)?
                                    };
                                    let stream = refreshed.as_ref().unwrap_or(stream);
                                    // A single result already announced itself above.
                                    if !info.requested_formats.is_empty() {
                                        if !stream.fragments.is_empty() {
                                            let downloader = match stream.protocol.as_deref() {
                                                Some("http_dash_segments") => "dashsegments",
                                                _ => "hlsnative",
                                            };
                                            screen!(
                                                "[{downloader}] Total fragments: {}",
                                                stream.fragments.len()
                                            );
                                        }
                                        screen!("[download] Destination: -");
                                    }
                                    let temporary = tempfile::tempdir()?;
                                    let target = temporary.path().join("media");
                                    downloader.download_with_progress(
                                        stream,
                                        &target,
                                        DownloadOptions {
                                            min_filesize,
                                            max_filesize,
                                            rate_limit,
                                            update_time: false,
                                            part_files: yt_dlp_rs::PartFiles::Private,
                                            retries,
                                            fragment_retries,
                                            retry_sleep,
                                            skip_unavailable_fragments,
                                            merge_files: yt_dlp_rs::MergeFiles::Private,
                                        },
                                        &mut progress,
                                    )?;
                                    let mut media = std::fs::File::open(&target)?;
                                    let mut out = io::stdout().lock();
                                    io::copy(&mut media, &mut out)?;
                                    out.flush()?;
                                }
                                return Ok(());
                            }
                            let (bytes, published_paths) = if processing {
                                let result = downloader.download_processed_with_progress(
                                    info,
                                    &path,
                                    DownloadOptions {
                                        min_filesize,
                                        max_filesize,
                                        rate_limit,
                                        update_time: args.mtime,
                                        part_files,
                                        retries,
                                        fragment_retries,
                                        retry_sleep,
                                        skip_unavailable_fragments,
                                        merge_files: yt_dlp_rs::MergeFiles::Upstream {
                                            keep: args.keep_video,
                                        },
                                    },
                                    &processors,
                                    progress,
                                )?;
                                for warning in result.warnings {
                                    warning!("WARNING: {warning}");
                                }
                                // Upstream `run_pp` under --ignore-errors.
                                for error in result.errors {
                                    eprintln!("ERROR: {error}");
                                    postprocess_failed.set(true);
                                }
                                if let Some(thumbnail) = &embedded_thumbnail
                                    && !keeps_thumbnails
                                    && std::fs::remove_file(thumbnail).is_err()
                                {
                                    warning!(
                                        "WARNING: Unable to delete file {}",
                                        thumbnail.display()
                                    );
                                }
                                let mut paths = vec![result.path];
                                paths.extend(result.kept_files);
                                (result.bytes, paths)
                            } else {
                                let bytes = downloader.download_with_progress(
                                    info,
                                    &path,
                                    DownloadOptions {
                                        min_filesize,
                                        max_filesize,
                                        rate_limit,
                                        update_time: args.mtime,
                                        part_files,
                                        retries,
                                        fragment_retries,
                                        retry_sleep,
                                        skip_unavailable_fragments,
                                        merge_files: yt_dlp_rs::MergeFiles::Upstream {
                                            keep: args.keep_video,
                                        },
                                    },
                                    progress,
                                )?;
                                // Upstream `fixup()` for an unmerged, unprocessed download.
                                if let Some(fixup) = &processors.fixup {
                                    downloader.fixup(info, &path, &path, fixup)?;
                                }
                                (bytes, vec![path.clone()])
                            };
                            let mut published_paths = published_paths;
                            published_paths.extend(info_json);
                            for (ext, source) in planned_sidecars {
                                if let Some(conversion) = &subtitle_conversion {
                                    published_paths.push(conversion.output_path(&source, &ext)?);
                                    published_paths
                                        .extend(conversion.intermediate_path(&source, &ext));
                                }
                                published_paths.push(source);
                            }
                            if let Some(archive_path) = &archive_path {
                                for output in &published_paths {
                                    let identical = std::path::absolute(output)?
                                        == std::path::absolute(archive_path)?;
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
                            if !post_printed.get() {
                                post_print.emit()?;
                            }
                            // Upstream `FFmpegSplitChaptersPP`, the last
                            // `post_process` processor before `--exec`.
                            if args.split_chapters {
                                let split = (|| -> yt_dlp_rs::Result<()> {
                                    if info.metadata.chapters.is_empty() {
                                        screen!(
                                            "[SplitChapters] Chapter information is unavailable"
                                        );
                                        return Ok(());
                                    }
                                    let mut chapters = Vec::new();
                                    for (index, chapter) in
                                        info.metadata.chapters.iter().enumerate()
                                    {
                                        let mut section = info.clone();
                                        section.section_number = Some(index + 1);
                                        section.section_title = chapter.title.clone();
                                        section.section_start = chapter.start_time;
                                        section.section_end = chapter.end_time;
                                        let Some(destination) = templates.prepare(
                                            &section,
                                            Some("chapter"),
                                            &template_options,
                                        )?
                                        else {
                                            continue;
                                        };
                                        chapters.push((chapter.clone(), destination));
                                    }
                                    downloader.split_chapters(&published_paths[0], &chapters)
                                })();
                                if let Err(error) = split {
                                    postprocess_failed.set(true);
                                    if error_mode == ErrorMode::Ignore {
                                        eprintln!("ERROR: {error}");
                                    } else {
                                        eprintln!("ERROR: Postprocessing: {error}");
                                        return Ok(());
                                    }
                                }
                            }
                            if !after_download(
                                &exec,
                                info,
                                &published_paths[0],
                                &path,
                                &template_options,
                                error_mode,
                                &pending_error,
                                &postprocess_failed,
                                &move_print,
                            ) {
                                return Ok(());
                            }
                            if let Some(archive) = &mut archive {
                                archive.record(info).map_err(|error| {
                                    format!(
                                        "download completed, but archive update failed: {error}"
                                    )
                                })?;
                            }
                            let _ = bytes;
                        }
                        Ok(())
                    })()?;
                    // A break filter or download limit ends the remaining sections.
                    if entry_cancel.is_some() {
                        break;
                    }
                }
                Ok(())
            })();
            let mut extraction_failed = false;
            let succeeded = match result {
                Ok(()) => true,
                Err(error) if error.is::<EntryExtractionError>() => {
                    eprintln!("ERROR: {error}");
                    failed = true;
                    extraction_failed = true;
                    if let Some(entry) = json_entries
                        .as_mut()
                        .and_then(|entries| entries.get_mut(entry_index))
                    {
                        *entry = serde_json::Value::Null;
                    } else if playlist_title.is_none() && single_json.is_some() {
                        single_json = Some(serde_json::Value::Null);
                    }
                    false
                }
                Err(error) => match error.downcast_ref::<yt_dlp_rs::Error>() {
                    // Upstream skips the transfer without an error or archive record.
                    Some(yt_dlp_rs::Error::FileSizeLimit(message)) => {
                        screen!("[download] {message}");
                        true
                    }
                    // Upstream `report_error` text without a URL/ID prefix.
                    Some(yt_dlp_rs::Error::Reported(message)) => {
                        eprintln!("ERROR: {message}");
                        failed = true;
                        false
                    }
                    _ => {
                        eprintln!("ERROR: {url}: {}: {error}", info.id);
                        failed = true;
                        false
                    }
                },
            };
            failed |= postprocess_failed.get();
            // Upstream `_raise_pending_errors` after `process_info`, then the
            // `after_video` commands (without file paths).
            if let Some(error) = pending_error.take() {
                eprintln!("ERROR: {error}");
                failed = true;
            }
            if counted
                && !(abort_on_error && failed)
                && let Err(error) = render_stage(
                    Stage::AfterVideo,
                    &print_templates,
                    &print_files,
                    &info,
                    prepare_media(&templates, &info, &template_options)
                        .map(|path| path.to_string_lossy().into_owned())
                        .as_deref(),
                    StagePaths::default(),
                    &template_options,
                )
                .map_err(|error| error.to_string())
                .and_then(|rendered| rendered.emit().map_err(|error| error.to_string()))
                .and_then(|()| {
                    run_exec(
                        &exec,
                        Stage::AfterVideo,
                        &info,
                        None,
                        None,
                        &template_options,
                    )
                })
            {
                eprintln!("ERROR: {error}");
                failed = true;
            }
            // Upstream `--abort-on-error`: the first reported error ends the queue.
            if abort_on_error && failed {
                aborted = true;
                break;
            }
            extraction_failures += usize::from(extraction_failed);
            if let (Some(limit), Some(title)) = (
                args.skip_playlist_after_errors.filter(|limit| *limit != 0),
                &playlist_title,
            ) && extraction_failures as i64 >= limit
            {
                eprintln!(
                    "ERROR: Skipping the remaining entries in playlist \"{title}\" since {extraction_failures} items failed extraction"
                );
                failed = true;
                break;
            }
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
        if cancelled.is_some() || aborted {
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
        if let Some(title) = &playlist_title {
            screen!("[download] Finished downloading playlist: {title}");
        }
        if let (Some(object), Some(entries)) = (
            single_json.as_mut().and_then(|value| value.as_object_mut()),
            json_entries,
        ) {
            object.insert("entries".into(), entries);
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

/// Run `--parse-metadata`/`--replace-in-metadata` actions for `when`.
fn run_metadata_parser(
    actions: &yt_dlp_rs::metadata_parser::MetadataActions,
    when: &str,
    info: &mut yt_dlp_rs::VideoInfo,
    options: &TemplateOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let (messages, error) = match actions.run(when, info, options) {
        Ok(messages) => (messages, None),
        Err((messages, error)) => (messages, Some(error)),
    };
    for message in messages {
        match message {
            yt_dlp_rs::metadata_parser::Message::Screen(text) => screen!("{text}"),
            yt_dlp_rs::metadata_parser::Message::Warning(text) => warning!("{text}"),
        }
    }
    match error {
        // Raised from the processor and reported as `ERROR: MESSAGE`.
        Some(error) => Err(Box::new(yt_dlp_rs::Error::Reported(error))),
        None => Ok(()),
    }
}

/// Upstream `__list_table`: the caption (or `ID has no NAME`) is a screen
/// message and only the table is written to stdout.
fn emit_listing(stdout: &mut impl Write, text: &str) -> io::Result<()> {
    let (caption, table) = text.split_once('\n').unwrap_or((text, ""));
    screen!("{caption}");
    stdout.write_all(table.as_bytes())
}

/// An entry `ExtractorError` from upstream `process_video_result`, reported
/// as `ERROR: [EXTRACTOR] ID: MESSAGE`.
#[derive(Debug)]
struct EntryExtractionError(String);

impl std::fmt::Display for EntryExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EntryExtractionError {}

/// An upstream `parser.error`: usage, `PROG: error: MESSAGE`, exit status 2.
#[derive(Debug)]
struct UsageError(String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

/// The parser, with optparse's value handling: an option that takes a
/// value consumes the next argument even when it starts with `-`.
fn command() -> clap::Command {
    Args::command().mut_args(|argument| {
        let takes_value = argument.get_action().takes_values() && !argument.is_positional();
        argument.allow_hyphen_values(takes_value)
    })
}

/// Upstream `_PRESET_ALIASES`.
const PRESET_ALIASES: [(&str, &[&str]); 5] = [
    (
        "mp3",
        &["-f", "ba[acodec^=mp3]/ba/b", "-x", "--audio-format", "mp3"],
    ),
    (
        "aac",
        &[
            "-f",
            "ba[acodec^=aac]/ba[acodec^=mp4a.40.]/ba/b",
            "-x",
            "--audio-format",
            "aac",
        ],
    ),
    (
        "mp4",
        &[
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
            "-S",
            "vcodec:h264,lang,quality,res,fps,hdr:12,acodec:aac",
        ],
    ),
    (
        "mkv",
        &["--merge-output-format", "mkv", "--remux-video", "mkv"],
    ),
    (
        "sleep",
        &[
            "--sleep-subtitles",
            "5",
            "--sleep-requests",
            "0.75",
            "--sleep-interval",
            "10",
            "--max-sleep-interval",
            "20",
        ],
    ),
];

/// Upstream `-t`/`--preset-alias NAME`: the preset's options replace the
/// alias in place (`parser.rargs[:0] = ...`). Option values (including
/// another option's `-t` value) and arguments after `--` are left alone.
fn expand_preset_aliases(arguments: Vec<String>) -> Result<Vec<String>, String> {
    // Value arities are only known once the command is built.
    let mut command = Args::command();
    command.build();
    let takes_value = |token: &str| -> bool {
        let found = if let Some(long) = token.strip_prefix("--") {
            command.get_arguments().find(|argument| {
                argument.get_long() == Some(long)
                    || argument
                        .get_all_aliases()
                        .is_some_and(|aliases| aliases.contains(&long))
            })
        } else {
            let short = token.chars().nth(1);
            command
                .get_arguments()
                .find(|argument| argument.get_short() == short)
        };
        found.is_some_and(|argument| {
            argument
                .get_num_args()
                .is_some_and(|range| range.takes_values())
        })
    };
    let arity = |token: &str| -> usize {
        let long = token.strip_prefix("--").unwrap_or(token);
        let found = command.get_arguments().find(|argument| {
            argument.get_long() == Some(long)
                || argument
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&long))
        });
        found
            .and_then(|argument| argument.get_num_args())
            .map_or(1, |range| range.min_values().max(1))
    };
    let mut output = Vec::with_capacity(arguments.len());
    let mut iter = arguments.into_iter();
    while let Some(token) = iter.next() {
        if token == "--" {
            output.push(token);
            output.extend(iter);
            break;
        }
        let name = if token == "-t" || token == "--preset-alias" {
            Some(
                iter.next()
                    .ok_or_else(|| format!("{token} option requires 1 argument"))?,
            )
        } else {
            token
                .strip_prefix("--preset-alias=")
                .or_else(|| {
                    token
                        .strip_prefix("-t")
                        .filter(|_| !token.starts_with("--"))
                })
                .map(str::to_owned)
        };
        if let Some(name) = name {
            let preset = PRESET_ALIASES
                .iter()
                .find(|(preset, _)| *preset == name)
                .ok_or_else(|| format!("Unknown preset alias: {name}"))?;
            output.extend(preset.1.iter().map(|value| (*value).to_owned()));
            continue;
        }
        // Keep option values with their option.
        let inline = token.starts_with("--") && token.contains('=');
        let short_inline = !token.starts_with("--") && token.starts_with('-') && token.len() > 2;
        if token.starts_with('-')
            && token.len() > 1
            && !inline
            && !short_inline
            && takes_value(&token)
        {
            let count = if token.starts_with("--") {
                arity(&token)
            } else {
                1
            };
            output.push(token);
            output.extend(iter.by_ref().take(count));
            continue;
        }
        output.push(token);
    }
    Ok(output)
}

/// Upstream `deprecated_switches`/`deprecated_arguments`: accepted no-ops.
fn strip_deprecated_options(arguments: Vec<String>) -> (Vec<String>, Vec<String>) {
    const SWITCHES: &[&str] = &[
        "--xattr-set-filesize",
        "--dump-user-agent",
        "--youtube-include-dash-manifest",
        "--no-youtube-skip-dash-manifest",
        "--youtube-skip-dash-manifest",
        "--no-youtube-include-dash-manifest",
        "--youtube-include-hls-manifest",
        "--no-youtube-skip-hls-manifest",
        "--youtube-skip-hls-manifest",
        "--no-youtube-include-hls-manifest",
        "--youtube-print-sig-code",
        "--sponskrub",
        "--no-sponskrub",
        "--sponskrub-cut",
        "--no-sponskrub-cut",
        "--sponskrub-force",
        "--no-sponskrub-force",
        "--prefer-avconv",
        "--no-prefer-ffmpeg",
        "--prefer-ffmpeg",
        "--no-prefer-avconv",
        "-C",
        "--call-home",
        "--no-call-home",
        "--include-ads",
        "--no-include-ads",
        "--write-annotations",
        "--no-write-annotations",
    ];
    const ARGUMENTS: &[&str] = &[
        "--sponskrub-location",
        "--sponskrub-args",
        "--cn-verification-proxy",
    ];
    let mut kept = Vec::with_capacity(arguments.len());
    let mut deprecated = Vec::new();
    let mut iter = arguments.into_iter();
    while let Some(argument) = iter.next() {
        if argument == "--" {
            kept.push(argument);
            kept.extend(iter);
            break;
        }
        let name = argument
            .split_once('=')
            .map_or(argument.as_str(), |(name, _)| name);
        if SWITCHES.contains(&argument.as_str()) {
            deprecated.push(argument);
        } else if ARGUMENTS.contains(&name) {
            if !argument.contains('=') {
                iter.next();
            }
            deprecated.push(name.to_owned());
        } else {
            kept.push(argument);
        }
    }
    (kept, deprecated)
}

/// What one configuration's own arguments say about further configuration
/// (upstream `parse_known_args` of those arguments).
#[derive(Default)]
struct ConfigScan {
    ignore_config: bool,
    locations: Vec<String>,
    home: Option<String>,
}

/// Find `--ignore-config`, `--config-locations` and `-P` home values in an
/// argument list, using clap's option table to skip option values.
fn scan_config_options(command: &clap::Command, args: &[String]) -> ConfigScan {
    let mut scan = ConfigScan::default();
    let find_long = |name: &str| {
        command.get_arguments().find(|arg| {
            arg.get_long() == Some(name)
                || arg
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&name))
        })
    };
    let takes_value = |arg: &clap::Arg| arg.get_action().takes_values();
    let mut record = |id: &str, value: Option<&str>| match (id, value) {
        ("ignore_config", _) => scan.ignore_config = true,
        ("no_config_locations", _) => scan.locations.clear(),
        ("config_locations", Some(value)) => scan.locations.push(value.to_owned()),
        ("paths", Some(value)) => {
            // `-P [TYPES:]PATH` with the shared typed-key rule.
            if let Ok(paths) = yt_dlp_rs::output::parse_paths(&[value.to_owned()])
                && let Some(home) = paths.get("home")
            {
                scan.home = Some(home.clone());
            }
        }
        _ => {}
    };
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        index += 1;
        if token == "--" {
            break;
        }
        if let Some(long) = token.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (long, None),
            };
            let Some(arg) = find_long(name) else {
                continue;
            };
            let value = if takes_value(arg) {
                inline.or_else(|| {
                    let value = args.get(index).map(String::as_str);
                    index += 1;
                    value
                })
            } else {
                None
            };
            record(arg.get_id().as_str(), value);
        } else if let Some(cluster) = token.strip_prefix('-').filter(|rest| !rest.is_empty()) {
            for (offset, short) in cluster.char_indices() {
                let Some(arg) = command.get_arguments().find(|arg| {
                    arg.get_short() == Some(short)
                        || arg
                            .get_short_and_visible_aliases()
                            .is_some_and(|aliases| aliases.contains(&short))
                }) else {
                    break;
                };
                if takes_value(arg) {
                    let rest = &cluster[offset + short.len_utf8()..];
                    let value = if rest.is_empty() {
                        let value = args.get(index).cloned();
                        index += 1;
                        value
                    } else {
                        Some(rest.to_owned())
                    };
                    record(arg.get_id().as_str(), value.as_deref());
                    break;
                }
                record(arg.get_id().as_str(), None);
            }
        }
    }
    scan
}

/// One loaded configuration and the configurations its locations name.
struct ConfigNode {
    args: Vec<String>,
    children: Vec<ConfigNode>,
}

impl ConfigNode {
    /// Upstream `Config.all_args`: later locations first, then own arguments.
    fn all_args(&self, out: &mut Vec<String>) {
        for child in self.children.iter().rev() {
            child.all_args(out);
        }
        out.extend(self.args.iter().cloned());
    }
}

/// Upstream `Config.init`/`load_configs` for one configuration; `None` when
/// the file was already loaded.
fn load_config(
    command: &clap::Command,
    args: Vec<String>,
    filename: Option<&Path>,
    loaded: &mut std::collections::HashSet<PathBuf>,
) -> Result<Option<ConfigNode>, UsageError> {
    let mut directory = PathBuf::new();
    if let Some(filename) = filename {
        let location = std::fs::canonicalize(filename).unwrap_or_else(|_| filename.into());
        directory = location.parent().map(Path::to_path_buf).unwrap_or_default();
        if !loaded.insert(location) {
            return Ok(None);
        }
    }
    let mut node = ConfigNode {
        args: Vec::new(),
        children: Vec::new(),
    };
    for location in scan_config_options(command, &args).locations {
        if location == "-" {
            if !loaded.insert(PathBuf::from("-")) {
                continue;
            }
            eprintln!("Reading options from STDIN - EOF (Ctrl+D) to end:");
            let mut text = String::new();
            std::io::Read::read_to_string(&mut io::stdin(), &mut text)
                .map_err(|error| usage(error.to_string()))?;
            let stdin_args = yt_dlp_rs::config::shlex_split(&text).map_err(usage)?;
            if let Some(child) = load_config(command, stdin_args, None, loaded)? {
                node.children.push(child);
            }
            continue;
        }
        let mut path = directory.join(yt_dlp_rs::batch::expand_path_text(&location));
        if path.is_dir() {
            path = path.join(format!("{}.conf", yt_dlp_rs::config::PACKAGE_NAME));
        }
        if !path.exists() {
            return Err(usage(format!(
                "config location {} does not exist",
                path.display()
            )));
        }
        let file_args = yt_dlp_rs::config::read_file(&path)
            .map_err(usage)?
            .unwrap_or_default();
        if let Some(child) = load_config(command, file_args, Some(&path), loaded)? {
            node.children.push(child);
        }
    }
    node.args = args;
    Ok(Some(node))
}

/// Upstream `parseOpts` configuration assembly: command line, then
/// portable, home, user and system configurations until `--ignore-config`,
/// flattened so the command line takes precedence.
fn config_arguments(cli: Vec<String>) -> Result<Vec<String>, UsageError> {
    use yt_dlp_rs::config::{
        PACKAGE_NAME, first_config, read_file, system_config_dirs, user_config_dirs,
    };
    let command = Args::command();
    let mut loaded = std::collections::HashSet::new();
    // (label, node) in append order; `all_args` reverses it.
    let mut configs: Vec<(&str, ConfigNode)> = Vec::new();
    let flatten = |configs: &[(&str, ConfigNode)]| {
        let mut out = Vec::new();
        for (_, node) in configs.iter().rev() {
            node.all_args(&mut out);
        }
        out
    };
    if let Some(node) = load_config(&command, cli, None, &mut loaded)? {
        configs.push(("Command-line", node));
    }
    let mut loaded_all = true;
    for label in ["Portable", "Home", "User", "System"] {
        let current = scan_config_options(&command, &flatten(&configs));
        if current.ignore_config {
            loaded_all = false;
            break;
        }
        let found = match label {
            "Portable" => std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_path_buf))
                .map(|dir| dir.join(format!("{PACKAGE_NAME}.conf")))
                .map(|path| read_file(&path).map(|args| args.map(|args| (args, path))))
                .transpose()
                .map_err(usage)?
                .flatten(),
            "Home" => {
                let home = current.home.as_deref().unwrap_or("");
                let home = yt_dlp_rs::batch::expand_path_text(home);
                let path = Path::new(home.trim()).join(format!("{PACKAGE_NAME}.conf"));
                read_file(&path).map_err(usage)?.map(|args| (args, path))
            }
            "User" => {
                let home = std::env::var_os("HOME").map(PathBuf::from);
                first_config(&user_config_dirs(home.as_deref())).map_err(usage)?
            }
            _ => first_config(&system_config_dirs()).map_err(usage)?,
        };
        if let Some((args, path)) = found
            && let Some(node) = load_config(&command, args, Some(&path), &mut loaded)?
        {
            configs.push((label, node));
        }
    }
    // `--ignore-config` inside the system configuration drops the user one.
    if loaded_all && scan_config_options(&command, &flatten(&configs)).ignore_config {
        configs.retain(|(label, _)| *label != "User");
    }
    Ok(flatten(&configs))
}

fn usage(message: impl std::fmt::Display) -> UsageError {
    UsageError(message.to_string())
}

/// Upstream `_write_link_file` for each requested type. Like upstream, an
/// existing shortcut is kept unless `-w`/`--no-overwrites` is given.
fn write_links(
    kinds: &[&str],
    info: &yt_dlp_rs::VideoInfo,
    templates: &OutputTemplates,
    options: &TemplateOptions,
    overwrites: Option<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(uri) = yt_dlp_rs::output::iri_to_uri(&info.webpage_url) else {
        warning!(
            "WARNING: Cannot write internet shortcut file because the actual URL of \"{}\" is unknown or disallowed",
            info.webpage_url
        );
        return Ok(());
    };
    let Some(base) = templates.prepare(info, Some("link"), options)? else {
        return Ok(());
    };
    for kind in kinds {
        let path = PathBuf::from(yt_dlp_rs::output::replace_extension(
            &base.to_string_lossy(),
            kind,
            Some(&info.ext),
        ));
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        if overwrites != Some(false) && path.exists() {
            screen!("[info] Internet shortcut (.{kind}) is already present");
            continue;
        }
        screen!(
            "[info] Writing internet shortcut (.{kind}) to: {}",
            path.display()
        );
        let text = yt_dlp_rs::output::link_file(kind, &uri, &path.to_string_lossy());
        std::fs::write(&path, text)
            .map_err(|_| format!("Cannot write internet shortcut {}", path.display()))?;
    }
    Ok(())
}

fn stage_paths<'a>(filepath: Option<&'a str>, prepared: &'a str) -> StagePaths<'a> {
    StagePaths {
        filepath,
        prepared: Some(prepared),
    }
}

/// Rendered `--print`/`--print-to-file` output for one stage.
struct StagePrint {
    lines: Vec<String>,
    files: Vec<(String, PathBuf, String)>,
}

impl StagePrint {
    /// Print lines, then append file lines with upstream's `[info] Writing`.
    fn emit(&self) -> io::Result<()> {
        let mut stdout = out_stream();
        for line in &self.lines {
            writeln!(stdout, "{line}")?;
        }
        stdout.flush()?;
        for (template, path, line) in &self.files {
            screen!(
                "[info] Writing {} to: {}",
                yt_dlp_rs::batch::py_repr(template),
                path.display()
            );
            append_line(path, line)?;
        }
        Ok(())
    }
}

/// Render a stage's templates (all lines before any output, so a failure
/// prints nothing partial).
fn render_stage(
    stage: Stage,
    templates: &[PrintTemplate],
    files: &[PrintToFile],
    info: &yt_dlp_rs::VideoInfo,
    filename: Result<&str, &yt_dlp_rs::Error>,
    paths: StagePaths<'_>,
    options: &TemplateOptions,
) -> yt_dlp_rs::Result<StagePrint> {
    let lines = templates
        .iter()
        .filter(|template| template.stage() == stage && template.applies(info))
        .map(|template| template.render_at(info, filename, paths, options))
        .collect::<yt_dlp_rs::Result<_>>()?;
    let files = files
        .iter()
        .filter(|pair| pair.stage() == stage)
        .map(|pair| {
            let (path, line) = pair.render_at(info, filename, paths, options)?;
            Ok((pair.template().to_owned(), path, line))
        })
        .collect::<yt_dlp_rs::Result<_>>()?;
    Ok(StagePrint { lines, files })
}

/// Python `os.path.abspath`: join with the working directory and normalize
/// `.`/`..` components lexically.
fn py_abspath(path: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let joined = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let mut normal = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    normal.components().next_back(),
                    Some(Component::RootDir) | None
                ) {
                    normal.pop();
                }
            }
            other => normal.push(other),
        }
    }
    normal
}

/// Upstream `ExecPP.run` for one stage: announce and run each command,
/// stopping at the first failure.
fn run_exec(
    exec: &ExecCommands,
    stage: Stage,
    info: &yt_dlp_rs::VideoInfo,
    filepath: Option<&std::path::Path>,
    filename: Option<&std::path::Path>,
    options: &TemplateOptions,
) -> Result<(), String> {
    let text =
        |path: Option<&std::path::Path>| path.map(|path| path.to_string_lossy().into_owned());
    let (filepath, filename) = (text(filepath), text(filename));
    for command in exec.stage(stage) {
        let command = command
            .render(info, filepath.as_deref(), filename.as_deref(), options)
            .map_err(|error| error.to_string())?;
        screen!("[Exec] Executing command: {command}");
        ExecCommand::run(&command)?;
    }
    Ok(())
}

/// After a download or an existing file: report a pending preprocessing
/// error, then run `post_process` and `after_move` commands with `run_pp`
/// error handling. Returns whether the video may be archived.
#[allow(clippy::too_many_arguments)]
fn after_download(
    exec: &ExecCommands,
    info: &yt_dlp_rs::VideoInfo,
    filepath: &std::path::Path,
    filename: &std::path::Path,
    options: &TemplateOptions,
    mode: ErrorMode,
    pending: &std::cell::Cell<Option<String>>,
    failed: &std::cell::Cell<bool>,
    print_after_move: &dyn Fn(&std::path::Path) -> Result<(), String>,
) -> bool {
    if let Some(error) = pending.take() {
        eprintln!("ERROR: {error}");
        failed.set(true);
        if mode == ErrorMode::Abort {
            return false;
        }
    }
    // `MoveFilesAfterDownloadPP` leaves an absolute `filepath`.
    let moved = py_abspath(filepath);
    for (stage, filepath) in [
        (Stage::PostProcess, filepath),
        (Stage::AfterMove, moved.as_path()),
    ] {
        let printed = if stage == Stage::AfterMove {
            print_after_move(filepath)
        } else {
            Ok(())
        };
        if let Err(error) = printed
            .and_then(|()| run_exec(exec, stage, info, Some(filepath), Some(filename), options))
        {
            failed.set(true);
            if mode == ErrorMode::Ignore {
                eprintln!("ERROR: {error}");
            } else {
                eprintln!("ERROR: Postprocessing: {error}");
                return false;
            }
        }
    }
    true
}

/// Upstream progress parameters: `noprogress` (quiet unless `--progress`),
/// `progress_with_newline` and `progress_delta`.
struct ProgressOptions {
    enabled: bool,
    newline: bool,
    quiet: bool,
    delta: Option<Duration>,
}

/// `FileDownloader.report_progress` through one status printer per
/// downloaded stream, written to upstream's `_out_files.out` (stdout, or
/// stderr with `logtostderr`).
struct ProgressReporter<'a> {
    options: &'a ProgressOptions,
    active: Option<ActiveDownload>,
}

struct ActiveDownload {
    stream: Option<u8>,
    start: Instant,
    downloaded: u64,
    printer: yt_dlp_rs::output::progress::StatusPrinter,
    next: Instant,
}

impl<'a> ProgressReporter<'a> {
    fn new(options: &'a ProgressOptions) -> Self {
        Self {
            options,
            active: None,
        }
    }

    fn write(text: &str) {
        if LOG_TO_STDERR.load(Ordering::Relaxed) {
            let mut stderr = io::stderr().lock();
            let _ = stderr.write_all(text.as_bytes());
            let _ = stderr.flush();
        } else {
            let mut stdout = io::stdout().lock();
            let _ = stdout.write_all(text.as_bytes());
            let _ = stdout.flush();
        }
    }

    fn event(&mut self, progress: &yt_dlp_rs::DownloadProgress) {
        use yt_dlp_rs::output::progress::{ProgressStatus, downloading_line};
        match progress.phase {
            DownloadPhase::Downloading => {}
            // A reused merge input: upstream reports it finished without timing.
            DownloadPhase::Publishing
                if self.active.is_none() && progress.stream_index.is_some() =>
            {
                if !self.options.enabled {
                    screen!("[download] Download completed");
                } else {
                    let line =
                        yt_dlp_rs::output::progress::finished_line(progress.downloaded_bytes, None);
                    let mut printer = yt_dlp_rs::output::progress::StatusPrinter::new(
                        self.options.newline,
                        !self.options.quiet,
                    );
                    Self::write(&printer.print(&format!("[download] {line}")));
                    Self::write(&printer.end());
                }
                return;
            }
            DownloadPhase::Merging | DownloadPhase::Postprocessing | DownloadPhase::Publishing => {
                return self.finish();
            }
            // An external downloader's single `finished` status.
            DownloadPhase::Completed => {
                if let Some(active) = self.active.as_mut() {
                    active.downloaded = progress.downloaded_bytes;
                }
                return self.finish();
            }
            _ => return,
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.stream != progress.stream_index)
        {
            self.finish();
        }
        let now = Instant::now();
        let active = self.active.get_or_insert_with(|| ActiveDownload {
            stream: progress.stream_index,
            start: now,
            downloaded: 0,
            printer: yt_dlp_rs::output::progress::StatusPrinter::new(
                self.options.newline,
                !self.options.quiet,
            ),
            next: now,
        });
        active.downloaded = progress.downloaded_bytes;
        // Upstream's HTTP downloader reports after each received block.
        if !self.options.enabled
            || (progress.downloaded_bytes == 0 && progress.fragment_index.is_none())
        {
            return;
        }
        if let Some(delta) = self.options.delta {
            if now < active.next {
                return;
            }
            active.next += delta;
        }
        let elapsed = now.duration_since(active.start).as_secs_f64();
        let downloaded = progress.downloaded_bytes as f64;
        let speed =
            (progress.downloaded_bytes != 0 && elapsed >= 0.001).then(|| downloaded / elapsed);
        // Fragments: completed count and a size estimate from the average
        // fragment so far (upstream also counts the current one's length).
        let (fragment_index, estimate) = match (progress.fragment_index, progress.fragment_count) {
            (Some(index), Some(count)) if progress.total_bytes.is_none() => {
                let estimate = (progress.downloaded_bytes > 0)
                    .then(|| downloaded / f64::from(index) * f64::from(count));
                (Some(index.saturating_sub(1)), estimate)
            }
            (Some(index), _) => (Some(index.saturating_sub(1)), None),
            _ => (None, None),
        };
        let remaining = progress
            .total_bytes
            .map(|total| total as f64)
            .or(estimate)
            .map(|total| total - downloaded);
        let eta = speed
            .zip(remaining)
            .map(|(speed, remaining)| (remaining / speed).trunc());
        let status = ProgressStatus {
            downloaded_bytes: progress.downloaded_bytes,
            total_bytes: progress.total_bytes,
            total_bytes_estimate: estimate,
            elapsed: Some(elapsed),
            speed,
            eta,
            fragment_index,
            fragment_count: progress.fragment_count,
        };
        let line = format!("[download] {}", downloading_line(&status));
        Self::write(&active.printer.print(&line));
    }

    /// The `finished` status of the current stream.
    fn finish(&mut self) {
        let Some(mut active) = self.active.take() else {
            return;
        };
        if !self.options.enabled {
            screen!("[download] Download completed");
            return;
        }
        let elapsed = active.start.elapsed().as_secs_f64();
        let line = format!(
            "[download] {}",
            yt_dlp_rs::output::progress::finished_line(active.downloaded, Some(elapsed))
        );
        Self::write(&active.printer.print(&line));
        Self::write(&active.printer.end());
    }
}

impl Drop for ProgressReporter<'_> {
    /// End an interrupted status line before an error is reported.
    fn drop(&mut self) {
        if let Some(mut active) = self.active.take()
            && self.options.enabled
        {
            Self::write(&active.printer.end());
        }
    }
}

/// Upstream `ignoreerrors` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorMode {
    Ignore,
    OnlyDownload,
    Abort,
}

/// Upstream `DownloadCancelled` reasons; uncaught ones exit with 101.
#[derive(Debug, Clone)]
enum Cancel {
    MaxDownloads,
    Existing,
    /// A `--break-match-filters` (or `--break-on-reject`) rejection.
    Rejected,
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
            Cancel::Rejected => {
                "Encountered a video that did not match filter, stopping due to --break-match-filter"
            }
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

/// Upstream `FFmpegPostProcessor._determine_executables`: without a location
/// the programs come from `PATH`; a directory holds both, and a file's name
/// selects its program, with the other found by substituting the name.
fn ffmpeg_executables(location: Option<&Path>) -> (Option<PathBuf>, Option<PathBuf>) {
    const PROGRAMS: [&str; 2] = ["ffmpeg", "ffprobe"];
    let Some(location) = location else {
        let [ffmpeg, ffprobe] = PROGRAMS.map(find_in_path);
        return (ffmpeg, ffprobe);
    };
    if !location.exists() {
        warning!(
            "WARNING: ffmpeg-location {} does not exist! Continuing without ffmpeg",
            location.display()
        );
        return (None, None);
    }
    let (directory, named) = if location.is_dir() {
        (location.to_path_buf(), None)
    } else {
        let filename = location
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let basename = PROGRAMS
            .into_iter()
            .find(|program| filename.contains(program))
            .unwrap_or("ffmpeg");
        let absolute = std::path::absolute(location).unwrap_or_else(|_| location.into());
        let directory = absolute.parent().map(Path::to_path_buf).unwrap_or_default();
        (directory, Some((basename, filename)))
    };
    let [ffmpeg, ffprobe] = PROGRAMS.map(|program| {
        let Some((basename, filename)) = &named else {
            return directory.join(program);
        };
        if *basename == program {
            return location.to_path_buf();
        }
        if filename.contains(basename) {
            let sibling = directory.join(filename.replace(basename, program));
            if sibling.exists() {
                return sibling;
            }
        }
        directory.join(program)
    });
    (Some(ffmpeg), Some(ffprobe))
}

/// The first executable `PATH` entry for `program` (with `.exe` on Windows).
fn find_in_path(program: &str) -> Option<PathBuf> {
    let name = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(&name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Upstream `determine_ext(url, 'jpg')`.
fn determine_ext(url: &str) -> String {
    const KNOWN: &[&str] = &[
        "3g2", "3gp", "f4v", "mk3d", "divx", "mpg", "ogv", "m4v", "wmv", "avi", "flv", "mkv",
        "mov", "mp4", "webm", "aac", "ape", "asf", "f4a", "f4b", "m4b", "m4r", "oga", "ogx", "spx",
        "vorbis", "wma", "weba", "aiff", "alac", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav",
        "f4f", "f4m", "m3u8", "smil", "mpd",
    ];
    if !url.contains('.') {
        return "jpg".into();
    }
    let path = url.split('?').next().unwrap_or("");
    let guess = path.rsplit_once('.').map_or(path, |(_, guess)| guess);
    if !guess.is_empty() && guess.bytes().all(|b| b.is_ascii_alphanumeric()) {
        guess.into()
    } else if KNOWN.contains(&guess.trim_end_matches('/')) {
        guess.trim_end_matches('/').into()
    } else {
        "jpg".into()
    }
}

/// Upstream `_write_thumbnails` for a video: try thumbnails from best to worst
/// until one is written (or all of them with `write_all`).
fn write_thumbnails(
    downloader: &YoutubeDl,
    info: &yt_dlp_rs::VideoInfo,
    base: Option<PathBuf>,
    write_all: bool,
    overwrites: Option<bool>,
    retries: u32,
) -> Result<Vec<(usize, PathBuf)>, Box<dyn std::error::Error>> {
    let thumbnails = &info.metadata.thumbnails;
    let mut on_disk = Vec::new();
    if thumbnails.is_empty() {
        screen!("[info] There are no video thumbnails to download");
        return Ok(on_disk);
    }
    let Some(base) = base else {
        return Ok(on_disk);
    };
    let multiple = write_all && thumbnails.len() > 1;
    let base = base.to_string_lossy();
    let mut written = false;
    for (index, thumbnail) in thumbnails.iter().enumerate().rev() {
        let id = thumbnail.id.as_deref().unwrap_or("None");
        let mut ext = determine_ext(&thumbnail.url);
        if multiple {
            ext = format!("{id}.{ext}");
        }
        let display = format!("video thumbnail {id}");
        let path = PathBuf::from(yt_dlp_rs::output::replace_extension(
            &base,
            &ext,
            Some(&info.ext),
        ));
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        if path_exists(&path)? {
            if overwrites == Some(false) {
                let label = if multiple {
                    display.clone()
                } else {
                    "video thumbnail".into()
                };
                let mut chars = label.chars();
                let capitalized: String = chars
                    .next()
                    .map(|c| {
                        c.to_uppercase()
                            .chain(chars.flat_map(char::to_lowercase))
                            .collect()
                    })
                    .unwrap_or_default();
                screen!("[info] {capitalized} is already present");
                on_disk.push((index, path));
                written = true;
                if !write_all {
                    break;
                }
                continue;
            }
            screen!("[download] Deleting existing file {}", path.display());
            std::fs::remove_file(&path)?;
        }
        screen!("[info] Downloading {display} ...");
        let options = DownloadOptions {
            retries,
            ..Default::default()
        };
        match downloader.download_thumbnail(&thumbnail.url, &path, options) {
            Ok(_) => {
                screen!("[info] Writing {display} to: {}", path.display());
                on_disk.push((index, path));
                written = true;
            }
            Err(yt_dlp_rs::Error::Http(error))
                if error.status() == Some(reqwest::StatusCode::NOT_FOUND) =>
            {
                let title: Vec<String> = display
                    .split(' ')
                    .map(|word| {
                        let mut chars = word.chars();
                        chars
                            .next()
                            .map(|c| c.to_uppercase().chain(chars).collect())
                            .unwrap_or_default()
                    })
                    .collect();
                screen!("[info] {} does not exist", title.join(" "));
            }
            Err(error) => warning!("WARNING: Unable to download {display}: {error}"),
        }
        if written && !write_all {
            break;
        }
    }
    on_disk.sort();
    Ok(on_disk)
}

/// Upstream `FFmpegThumbnailsConvertorPP.run` over the written thumbnails.
fn convert_thumbnails(
    downloader: &YoutubeDl,
    thumbnails: &mut [(usize, PathBuf)],
    conversion: &ThumbnailConversion,
    keep_original: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if thumbnails.is_empty() {
        screen!("[ThumbnailsConvertor] There aren't any thumbnails to convert");
    }
    for (_, path) in thumbnails.iter_mut() {
        let original = yt_dlp_rs::fixup_webp(path)?;
        if original != *path {
            screen!(
                "[ThumbnailsConvertor] Correcting thumbnail \"{}\" extension to webp",
                path.display()
            );
        }
        let mut ext = original
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ext == "jpeg" {
            ext = "jpg".into();
        }
        match conversion.resolve(&ext) {
            Err(reason) => {
                screen!(
                    "[ThumbnailsConvertor] Not converting thumbnail \"{}\"; {reason}",
                    original.display()
                );
                *path = original;
            }
            Ok(target) => {
                let converted = PathBuf::from(yt_dlp_rs::output::replace_extension(
                    &original.to_string_lossy(),
                    &target,
                    None,
                ));
                screen!(
                    "[ThumbnailsConvertor] Converting thumbnail \"{}\" to {target}",
                    original.display()
                );
                downloader.convert_thumbnail(&original, &converted, &target)?;
                if !keep_original {
                    screen!(
                        "Deleting original file {} (pass -k to keep)",
                        original.display()
                    );
                    if std::fs::remove_file(&original).is_err() {
                        warning!("WARNING: Unable to delete file {}", original.display());
                    }
                }
                *path = converted;
            }
        }
    }
    Ok(())
}

/// Upstream `_write_description` for a video.
fn write_description(
    path: &std::path::Path,
    info: &yt_dlp_rs::VideoInfo,
    overwrites: Option<bool>,
) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    if overwrites == Some(false) && path.exists() {
        screen!("[info] Video description is already present");
    } else if let Some(description) = &info.metadata.description {
        screen!("[info] Writing video description to: {}", path.display());
        std::fs::write(path, description)?;
    } else {
        screen!("[info] There's no video description to write");
    }
    Ok(())
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
                screen!("[download] {reason}");
                *pending = Some(Cancel::Rejected);
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
        Err(error) if error.is::<UsageError>() => {
            // Upstream `_generate_error_message`, written after a newline.
            eprintln!(
                "\nUsage: yt-dlp-rs [OPTIONS] URL [URL...]\n\nyt-dlp-rs: error: {}",
                yt_dlp_rs::batch::py_strip(&error.to_string())
            );
            std::process::ExitCode::from(2)
        }
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
    fn preset_aliases_expand_in_place_like_upstream() {
        let expand = |args: &[&str]| {
            expand_preset_aliases(args.iter().map(|arg| (*arg).to_owned()).collect())
        };
        assert_eq!(
            expand(&["-t", "mp3", "-o", "-t", "URL"]).unwrap(),
            [
                "-f",
                "ba[acodec^=mp3]/ba/b",
                "-x",
                "--audio-format",
                "mp3",
                "-o",
                "-t",
                "URL"
            ]
        );
        assert_eq!(
            expand(&["-tmkv", "--preset-alias=sleep", "--", "-t"]).unwrap()[..4],
            ["--merge-output-format", "mkv", "--remux-video", "mkv"]
        );
        assert_eq!(
            expand(&["-t", "nope"]).unwrap_err(),
            "Unknown preset alias: nope"
        );
    }

    #[test]
    fn descriptions_follow_upstream_overwrite_and_missing_rules() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sub/video.description");
        let mut info: yt_dlp_rs::VideoInfo = serde_json::from_value(serde_json::json!({
            "id": "v", "title": "t", "ext": "mp4", "url": "https://example.invalid/v.mp4",
            "webpage_url": "https://example.invalid/", "extractor": "generic",
            "extractor_key": "Generic", "direct": true, "description": "Line one\nLine two",
        }))
        .unwrap();
        write_description(&path, &info, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Line one\nLine two"
        );
        info.metadata.description = Some("changed".into());
        write_description(&path, &info, Some(false)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Line one\nLine two"
        );
        write_description(&path, &info, None).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed");
        info.metadata.description = None;
        let missing = directory.path().join("none.description");
        write_description(&missing, &info, None).unwrap();
        assert!(!missing.exists());
    }

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
