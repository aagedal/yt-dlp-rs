use clap::Parser;
use std::io::{self, Write};
use yt_dlp_rs::{
    YoutubeDl,
    output::{DEFAULT_TEMPLATE, render_template},
};

#[derive(Parser)]
#[command(
    version,
    about = "Native Rust yt-dlp adaptation (direct HTTP and single-source HTML5 media)"
)]
struct Args {
    #[arg(required = true)]
    urls: Vec<String>,
    /// Print one JSON info object per URL without downloading
    #[arg(short = 'j', long)]
    dump_json: bool,
    /// Extract metadata without writing media
    #[arg(short = 's', long)]
    simulate: bool,
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
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let info = downloader.extract_info(&url)?;
            if args.dump_json {
                serde_json::to_writer(&mut stdout, &info)?;
                writeln!(stdout)?;
            } else if args.simulate {
                eprintln!("[{}] {}: {}", info.extractor, info.id, info.title);
            } else {
                let path = render_template(&args.output, &info)?;
                eprintln!("[download] Destination: {}", path.display());
                let bytes = downloader.download(&info, &path)?;
                eprintln!("[download] Downloaded {bytes} bytes");
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("ERROR: {url}: {error}");
            failed = true;
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
