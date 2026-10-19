//! `--download-sections`: upstream's section planning, `section_*` fields and
//! the FFmpegFD invocation, with a recording FFmpeg stand-in. Expected lines
//! were checked against pinned upstream with the same media.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
};

fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (kind, body): (&str, &[u8]) = match path.as_str() {
                "/page.html" => (
                    "text/html",
                    b"<html><head><title>Sec</title></head><body><video src=\"v.mp4\"></video></body></html>",
                ),
                _ => ("video/mp4", b"\0\0\0\x18ftypisom media bytes"),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    origin
}

/// A recording stand-in: arguments go to `args.txt` (one per line) and the
/// output file (`file:` prefix stripped) receives fixed bytes.
#[cfg(unix)]
fn fake_ffmpeg(directory: &Path, status: u8) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("ffmpeg");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffmpeg version test'; exit 0; fi\n\
             for a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"${{0%/ffmpeg}}/args.txt\"\n\
             for a in \"$@\"; do last=\"$a\"; done\nprintf 'section' > \"${{last#file:}}\"\nexit {status}\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress"])
        .args(args)
        .current_dir(directory)
        // No ffmpeg on PATH unless --ffmpeg-location names one.
        .env("PATH", "")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn sections_print_fields_without_ffmpeg_and_report_its_absence() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let url = format!("{origin}/v.mp4");
    let output = run(
        directory.path(),
        &[
            "--download-sections",
            "*2-4,6-7",
            "--download-sections",
            "*-3-inf",
            "-s",
            "-O",
            "%(section_start)s-%(section_end)s %(section_title)s",
            &url,
        ],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "2-4 NA\n6-7 NA\n-3-NA NA\n");
    let output = run(
        directory.path(),
        &["--download-sections", "*2-4", "-o", "o.%(ext)s", &url],
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = text(&output.stdout);
    assert!(stdout.ends_with(
        "[info] v: Downloading 1 format(s): mp4\n[info] v: Downloading 1 time ranges: 2.0-4.0\n"
    ));
    assert_eq!(
        text(&output.stderr),
        "ERROR: You have requested downloading the video partially, but ffmpeg is not installed. Aborting\n"
    );
    assert!(!directory.path().join("o.mp4").exists());
    // Unmatched chapter regexes download nothing.
    let output = run(directory.path(), &["--download-sections", "intro", &url]);
    assert!(output.status.success());
    assert!(
        text(&output.stdout).ends_with(
            "[info] v: Cannot match chapters since chapter information is unavailable\n"
        )
    );
}

#[test]
fn invalid_sections_are_usage_errors() {
    let directory = tempfile::tempdir().unwrap();
    for (value, message) in [
        (
            "*a-b",
            "invalid --download-sections time range \"*a-b\". Must be of the form \"*start-end\"",
        ),
        (
            "*1--inf",
            "invalid --download-sections time range \"*1--inf\". \"-inf\" is not a valid end",
        ),
        ("(bad", "invalid --download-sections regex \"(bad\" - "),
    ] {
        let output = run(
            directory.path(),
            &["--download-sections", value, "http://127.0.0.1:1/"],
        );
        assert_eq!(output.status.code(), Some(2), "{value}");
        assert!(
            text(&output.stderr).contains(&format!("yt-dlp-rs: error: {message}")),
            "{value}"
        );
    }
}

#[cfg(unix)]
#[test]
fn ffmpeg_downloads_each_section_with_upstream_arguments() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(directory.path(), 0);
    let page = format!("{origin}/page.html");
    let output = run(
        directory.path(),
        &[
            "--ffmpeg-location",
            ffmpeg.to_str().unwrap(),
            "--download-sections",
            "*1:30-2:00.5",
            "-o",
            "o-%(section_start)s.%(ext)s",
            "--print",
            "before_dl:%(section_start)s %(section_end)s %(duration)s",
            &page,
        ],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    // `--print` implies quiet: FFmpeg gets `-loglevel quiet`.
    assert_eq!(text(&output.stdout), "90 120.5 NA\n");
    assert_eq!(
        std::fs::read(directory.path().join("o-90.mp4")).unwrap(),
        b"section"
    );
    let args = std::fs::read_to_string(directory.path().join("args.txt")).unwrap();
    let expected_headers = format!(
        "user-agent: yt-dlp-rs/{}\r\nreferer: {page}\r\n",
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        args,
        format!(
            "-y\n-loglevel\nquiet\n-hide_banner\n-headers\n{expected_headers}\n-ss\n90\n-t\n30.5\n\
             -i\n{origin}/v.mp4\n-c\ncopy\n-f\nmp4\nfile:o-90.mp4.part\n"
        )
    );
    // Screen output: one destination and FFmpegFD's finished status.
    let output = run(
        directory.path(),
        &[
            "--ffmpeg-location",
            ffmpeg.to_str().unwrap(),
            "--download-sections",
            "*2-4",
            "-o",
            "s.%(ext)s",
            &page,
        ],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).ends_with(
        "[info] page-1: Downloading 1 format(s): 0\n\
         [info] page-1: Downloading 1 time ranges: 2.0-4.0\n\
         [download] Destination: s.mp4\n\
         [download] Download completed\n"
    ));
    let args = std::fs::read_to_string(directory.path().join("args.txt")).unwrap();
    assert!(args.starts_with("-y\n-hide_banner\n-headers\n"), "{args}");
    assert!(args.contains("\n-ss\n2\n-t\n2\n-i\n"), "{args}");
}

#[cfg(unix)]
#[test]
fn ffmpeg_failures_are_reported_like_upstream() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(directory.path(), 3);
    let output = run(
        directory.path(),
        &[
            "--ffmpeg-location",
            ffmpeg.to_str().unwrap(),
            "--download-sections",
            "*2-4",
            "-o",
            "o.%(ext)s",
            &format!("{origin}/v.mp4"),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).ends_with("\n\nERROR: ffmpeg exited with code 3\n"));
    assert!(!directory.path().join("o.mp4").exists());
}
