use clap::Parser;
use std::io::{self, Write};
use yt_dlp_rs::{
    DownloadOptions, YoutubeDl,
    output::{DEFAULT_TEMPLATE, render_template},
};

#[derive(Parser)]
#[command(
    version,
    about = "Native Rust yt-dlp adaptation (direct HTTP and HTML5 media)"
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
    /// Select one literal HTML5 source ID (zero-based declaration index)
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
    /// Retry transient download failures this many times (0 disables retries; extraction is not retried)
    #[arg(long, default_value_t = 3)]
    retries: u8,
    /// Filename template: %(title)s, %(id)s, %(ext)s, %(extractor)s, and %%
    #[arg(short, long, default_value = DEFAULT_TEMPLATE)]
    output: String,
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let args = Args::parse();
    let downloader = YoutubeDl::new()?;
    let mut failed = false;
    let mut stdout = io::stdout().lock();
    for url in args.urls {
        let extracted = match downloader.extract_result_with_format(&url, args.format.as_deref()) {
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
                    let bytes = downloader.download_with_options(
                        &info,
                        &path,
                        DownloadOptions {
                            retries: args.retries,
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
