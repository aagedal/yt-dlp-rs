use clap::Parser;
use std::io::{self, Write};
use std::{
    ops::ControlFlow,
    path::PathBuf,
    time::{Duration, Instant},
};
use yt_dlp_rs::{
    DownloadArchive, DownloadOptions, DownloadPhase, ExtractResult, ExtractionOptions,
    PlaylistSelection, SubtitleOptions, YoutubeDl, YoutubeOptions,
    output::{DEFAULT_TEMPLATE, TemplateOptions, render_formats, render_template_with_options},
    render_subtitles, subtitle_paths, validate_format_selector,
};

#[derive(Parser)]
#[command(
    version,
    about = "Native Rust yt-dlp adaptation (direct HTTP, HTML5 and HLS/DASH VOD)"
)]
struct Args {
    #[arg(required = true)]
    urls: Vec<String>,
    /// Print one JSON info object per video (including playlist entries) without downloading
    #[arg(short = 'j', long)]
    dump_json: bool,
    /// Extract metadata without writing media
    #[arg(short = 's', long)]
    simulate: bool,
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
    /// Skip media downloads while allowing requested subtitle sidecars
    #[arg(long, alias = "no-download")]
    skip_download: bool,
    /// Format ID or indexed/grouped selector with filters, + pairs and / fallbacks
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
    /// Retry transient download failures this many times (0 disables retries)
    #[arg(long, default_value_t = 3)]
    retries: u8,
    /// Retry the entire extraction after transient network failures (0 disables retries)
    #[arg(long, default_value_t = 0)]
    extractor_retries: u8,
    /// Explicit path to Deno 2.9.7 for YouTube player challenges (no runtime discovery or installation)
    #[arg(long, value_name = "PATH")]
    youtube_js_runtime: Option<PathBuf>,
    /// Explicit ffmpeg executable for merging separate video/audio streams
    #[arg(long, value_name = "PATH")]
    ffmpeg_location: Option<PathBuf>,
    /// Skip recorded extractor/video IDs and record only completed downloads
    #[arg(long, value_name = "FILE")]
    download_archive: Option<PathBuf>,
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
    #[arg(short, long, default_value = DEFAULT_TEMPLATE)]
    output: String,
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let args = Args::parse();
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
        eprintln!(
            "WARNING: Ignoring --playlist-start and --playlist-end because --playlist-items was given"
        );
    }
    let subtitle_languages = (!args.sub_langs.is_empty()).then(|| args.sub_langs.join(","));
    let subtitle_options = SubtitleOptions::new(subtitle_languages.as_deref(), &args.sub_format)?;
    let mut downloader = YoutubeDl::with_youtube_options(YoutubeOptions {
        js_runtime: args.youtube_js_runtime,
    })?;
    if let Some(path) = args.ffmpeg_location {
        downloader = downloader.with_ffmpeg(path);
    }
    let mut archive = args
        .download_archive
        .map(DownloadArchive::load)
        .transpose()?;
    let template_options = TemplateOptions {
        na_placeholder: args.output_na_placeholder,
    };
    let list_only = (args.list_formats || args.list_subs) && !args.dump_json && !args.simulate;
    let mut failed = false;
    let mut stdout = io::stdout().lock();
    for url in args.urls {
        let options = ExtractionOptions {
            retries: args.extractor_retries,
        };
        let extracted = (|| -> yt_dlp_rs::Result<Option<ExtractResult>> {
            let mut result = downloader.extract_result_unselected_with_options(&url, options)?;
            playlist_selection.apply(&mut result);
            let mut result = if let Some(archive) = &archive {
                let Some(result) = filter_archived(result, archive)? else {
                    return Ok(None);
                };
                result
            } else {
                result
            };
            if args.write_subs {
                let select = |info: &mut yt_dlp_rs::VideoInfo| -> yt_dlp_rs::Result<()> {
                    let selection = subtitle_options.select(&info.subtitles)?;
                    for warning in selection.warnings {
                        eprintln!("WARNING: {}: {warning}", info.id);
                    }
                    info.requested_subtitles =
                        (!info.subtitles.is_empty()).then_some(selection.requested);
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
                downloader.select_result(&mut result, args.format.as_deref())?;
            }
            Ok(Some(result))
        })();
        let extracted = match extracted {
            Ok(Some(result)) => result,
            Ok(None) => continue,
            Err(error) => {
                eprintln!("ERROR: {url}: {error}");
                failed = true;
                continue;
            }
        };
        for info in extracted.into_entries() {
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                if args.list_subs {
                    write!(stdout, "{}", render_subtitles(&info))?;
                }
                if args.list_formats {
                    write!(stdout, "{}", render_formats(&info))?;
                }
                if (args.list_formats || args.list_subs) && !args.dump_json {
                    return Ok(());
                }
                if args.dump_json {
                    serde_json::to_writer(&mut stdout, &info)?;
                    writeln!(stdout)?;
                } else if args.simulate {
                    eprintln!("[{}] {}: {}", info.extractor, info.id, info.title);
                } else {
                    if args.skip_download
                        && info
                            .requested_subtitles
                            .as_ref()
                            .is_none_or(|subtitles| subtitles.is_empty())
                    {
                        return Ok(());
                    }
                    let path =
                        render_template_with_options(&args.output, &info, &template_options)?;
                    if !args.skip_download {
                        match std::fs::symlink_metadata(&path) {
                            Ok(_) => return Err(yt_dlp_rs::Error::OutputExists(path).into()),
                            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                            Err(error) => return Err(error.into()),
                        }
                    }
                    // Plan all names before I/O, including language sanitation collisions.
                    for (language, subtitle, subtitle_path) in subtitle_paths(&info, &path)? {
                        match std::fs::symlink_metadata(&subtitle_path) {
                            Ok(_) if std::fs::metadata(&subtitle_path)?.is_file() => {
                                eprintln!(
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
                                eprintln!(
                                    "[info] Writing {language:?} subtitles to: {}",
                                    subtitle_path.display()
                                );
                                downloader.download_subtitle_with_options(
                                    &info,
                                    &subtitle,
                                    &subtitle_path,
                                    DownloadOptions {
                                        retries: args.retries,
                                    },
                                )?;
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                    if args.skip_download {
                        return Ok(());
                    }
                    eprintln!("[download] Destination: {}", path.display());
                    let mut last_progress = Instant::now();
                    let bytes = downloader.download_with_progress(
                        &info,
                        &path,
                        DownloadOptions {
                            retries: args.retries,
                        },
                        |progress| {
                            if progress.phase == DownloadPhase::Downloading
                                && last_progress.elapsed() >= Duration::from_secs(1)
                            {
                                if let Some(total) = progress.total_bytes {
                                    eprintln!(
                                        "[download] {} / {total} bytes (attempt {})",
                                        progress.downloaded_bytes, progress.attempt
                                    );
                                } else {
                                    eprintln!(
                                        "[download] {} bytes (attempt {})",
                                        progress.downloaded_bytes, progress.attempt
                                    );
                                }
                                last_progress = Instant::now();
                            }
                            ControlFlow::Continue(())
                        },
                    )?;
                    if let Some(archive) = &mut archive {
                        archive.record(&info).map_err(|error| {
                            format!("download completed, but archive update failed: {error}")
                        })?;
                    }
                    eprintln!("[download] Downloaded {bytes} bytes");
                }
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("ERROR: {url}: {}: {error}", info.id);
                failed = true;
            }
        }
    }
    Ok(!failed)
}

fn filter_archived(
    result: ExtractResult,
    archive: &DownloadArchive,
) -> yt_dlp_rs::Result<Option<ExtractResult>> {
    let keep = |info: &yt_dlp_rs::VideoInfo| -> yt_dlp_rs::Result<bool> {
        let recorded = archive.contains(info)?;
        if recorded {
            eprintln!(
                "[download] {} has already been recorded in the archive",
                info.id
            );
        }
        Ok(!recorded)
    };
    match result {
        ExtractResult::Video(info) => Ok(keep(&info)?.then_some(ExtractResult::Video(info))),
        ExtractResult::Playlist(mut playlist) => {
            let mut pending = Vec::new();
            for info in playlist.entries {
                if keep(&info)? {
                    pending.push(info);
                }
            }
            if pending.is_empty() {
                return Ok(None);
            }
            playlist.entries = pending;
            Ok(Some(ExtractResult::Playlist(playlist)))
        }
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(error) => {
            eprintln!("ERROR: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
