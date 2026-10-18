use clap::Parser;
use std::io::{self, Write};
use std::{
    ops::ControlFlow,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use yt_dlp_rs::{
    AudioOptions, CookieJar, DownloadArchive, DownloadOptions, DownloadPhase, ExtractResult,
    ExtractionOptions, HttpOptions, PlaylistSelection, PostprocessOptions, SubtitleConversion,
    SubtitleOptions, VideoMode, VideoOptions, YoutubeDl, YoutubeOptions,
    output::{DEFAULT_TEMPLATE, TemplateOptions, render_formats, render_template_with_options},
    render_automatic_captions, render_subtitles, subtitle_paths, validate_format_selector,
};

#[derive(Parser)]
#[command(
    group(clap::ArgGroup::new("processing").args(["extract_audio", "remux_video", "recode_video", "convert_subs"]).multiple(true)),
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
    /// Convert written subtitles to srt, vtt, ass or lrc; none disables conversion
    #[arg(long, aliases = ["convert-sub", "convert-subtitles"], value_name = "FORMAT", overrides_with = "convert_subs")]
    convert_subs: Option<String>,
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
        eprintln!("WARNING: --remux-video is ignored since --recode-video was given");
    }
    let processors = PostprocessOptions {
        audio: args.extract_audio.then_some(audio_options),
        video: recode.or(remux),
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
        eprintln!(
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
            if args.write_subs || args.write_auto_subs {
                let select = |info: &mut yt_dlp_rs::VideoInfo| -> yt_dlp_rs::Result<()> {
                    let selection = subtitle_options.select_sources(
                        &info.subtitles,
                        &info.automatic_captions,
                        args.write_subs,
                        args.write_auto_subs,
                    )?;
                    for warning in selection.warnings {
                        eprintln!("WARNING: {}: {warning}", info.id);
                    }
                    info.requested_subtitles = ((args.write_subs && !info.subtitles.is_empty())
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
                    if !info.automatic_captions.is_empty() {
                        write!(stdout, "{}", render_automatic_captions(&info))?;
                    }
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
                    // Plan source, intermediate and final names before subtitle I/O.
                    let sidecars = subtitle_paths(&info, &path)?;
                    let mut seen = std::collections::HashSet::new();
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
                                    "subtitle conversion filenames collide".into(),
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
                    for (language, subtitle, subtitle_path) in sidecars {
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
                        if let Some(conversion) = &subtitle_conversion {
                            let converted = downloader.convert_subtitle_with_options(
                                &subtitle_path,
                                &subtitle.ext,
                                conversion,
                            )?;
                            for warning in converted.warnings {
                                eprintln!("WARNING: {warning}");
                            }
                            if converted.converted {
                                eprintln!(
                                    "[SubtitlesConvertor] Destination: {}",
                                    converted.path.display()
                                );
                            }
                        }
                    }
                    if args.skip_download {
                        return Ok(());
                    }
                    eprintln!("[download] Destination: {}", path.display());
                    let mut last_progress = Instant::now();
                    let progress = |progress: yt_dlp_rs::DownloadProgress| {
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
                    };
                    let (bytes, published_paths) =
                        if processors.audio.is_some() || processors.video.is_some() {
                            let result = downloader.download_processed_with_progress(
                                &info,
                                &path,
                                DownloadOptions {
                                    retries: args.retries,
                                },
                                &processors,
                                progress,
                            )?;
                            eprintln!("[postprocess] Destination: {}", result.path.display());
                            let mut paths = vec![result.path];
                            paths.extend(result.kept_files);
                            (result.bytes, paths)
                        } else {
                            let bytes = downloader.download_with_progress(
                                &info,
                                &path,
                                DownloadOptions {
                                    retries: args.retries,
                                },
                                progress,
                            )?;
                            (bytes, vec![path.clone()])
                        };
                    let mut published_paths = published_paths;
                    for (_, subtitle, source) in subtitle_paths(&info, &path)? {
                        published_paths.push(source.clone());
                        if let Some(conversion) = &subtitle_conversion {
                            published_paths.push(conversion.output_path(&source, &subtitle.ext)?);
                            published_paths
                                .extend(conversion.intermediate_path(&source, &subtitle.ext));
                        }
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
    for warning in downloader.cookie_jar().take_warnings() {
        eprintln!("WARNING: {warning}");
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
    Ok(!failed)
}

fn paths_alias(left: &std::path::Path, right: &std::path::Path) -> io::Result<bool> {
    Ok(std::path::absolute(left)? == std::path::absolute(right)?
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right))
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

#[cfg(test)]
mod tests {
    use super::*;

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
