use clap::Parser;
use std::io::{self, Write};
use std::{
    ops::ControlFlow,
    path::PathBuf,
    time::{Duration, Instant},
};
use yt_dlp_rs::{
    DownloadOptions, DownloadPhase, ExtractionOptions, YoutubeDl, YoutubeOptions,
    output::{DEFAULT_TEMPLATE, render_template},
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
    /// Format ID, best/worst, bestvideo+bestaudio, or a fallback expression
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
    /// Filename template: %(title)s, %(id)s, %(ext)s, %(extractor)s, and %%
    #[arg(short, long, default_value = DEFAULT_TEMPLATE)]
    output: String,
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut downloader = YoutubeDl::with_youtube_options(YoutubeOptions {
        js_runtime: args.youtube_js_runtime,
    })?;
    if let Some(path) = args.ffmpeg_location {
        downloader = downloader.with_ffmpeg(path);
    }
    let mut failed = false;
    let mut stdout = io::stdout().lock();
    for url in args.urls {
        let extracted = match downloader.extract_result_with_options(
            &url,
            args.format.as_deref(),
            ExtractionOptions {
                retries: args.extractor_retries,
            },
        ) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("ERROR: {url}: {error}");
                failed = true;
                continue;
            }
        };
        for info in extracted.into_entries() {
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                if args.dump_json {
                    serde_json::to_writer(&mut stdout, &info)?;
                    writeln!(stdout)?;
                } else if args.simulate {
                    eprintln!("[{}] {}: {}", info.extractor, info.id, info.title);
                } else {
                    let path = render_template(&args.output, &info)?;
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
