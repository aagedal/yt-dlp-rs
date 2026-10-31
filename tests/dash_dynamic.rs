//! Dynamic (live) DASH MPDs like pinned upstream: GenericIE marks them
//! `is_live`, `--ignore-dynamic-mpd` drops their formats, and live
//! `http_dash_segments` downloads use FFmpegFD on the manifest (`-re` before
//! each input). Expected outputs were captured from the pinned upstream on
//! the same saved manifests (`tests/fixtures/dash/dynamic-*.mpd`); the format
//! dicts themselves are compared by `tools/compare_dash.py` and
//! `tests/info_order.rs`. No test reads a segment or needs a real FFmpeg.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{Arc, Mutex},
    thread,
};

/// Serves `tests/fixtures/dash/NAME.mpd`; anything else is a 404. Records
/// every request path.
struct Server {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => request.extend_from_slice(&buffer[..count]),
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                recorded.lock().unwrap().push(path.clone());
                let name = path.trim_start_matches('/');
                let body = (name.ends_with(".mpd") && !name.contains('/'))
                    .then(|| {
                        std::fs::read(
                            Path::new(env!("CARGO_MANIFEST_DIR"))
                                .join("tests/fixtures/dash")
                                .join(name),
                        )
                        .ok()
                    })
                    .flatten();
                let _ = match body {
                    Some(body) => write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/dash+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .and_then(|()| stream.write_all(&body)),
                    None => stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    ),
                };
            }
        });
        Self { origin, requests }
    }

    fn url(&self, name: &str) -> String {
        format!("{}/{name}", self.origin)
    }

    /// Only manifests were requested (no initialization or media segment).
    fn assert_only_manifests(&self) {
        let requests = self.requests.lock().unwrap();
        assert!(
            requests.iter().all(|path| path.ends_with(".mpd")),
            "{requests:?}"
        );
    }
}

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--user-agent", "fixture"])
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// GenericIE's `live_status`, the live title and FFmpegFD's protocol choice
/// (`--print` after format selection), like upstream.
#[test]
fn dynamic_manifests_are_live_generic_results() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    for (name, format) in [
        ("dynamic-number.mpd", "v1+a1"),
        ("dynamic-timeline.mpd", "v+a"),
        ("dynamic-list.mpd", "list+audio"),
    ] {
        let output = run(
            directory.path(),
            &[
                "-s",
                "--print",
                "%(fulltitle)s|%(is_live)s|%(was_live)s|%(live_status)s|%(format_id)s|%(protocol)s",
                &server.url(name),
            ],
        );
        assert!(output.status.success(), "{}", text(&output.stderr));
        let stem = name.trim_end_matches(".mpd");
        assert_eq!(
            text(&output.stdout),
            format!("{stem}|True|False|is_live|{format}|http_dash_segments+http_dash_segments\n")
        );
    }
    server.assert_only_manifests();
}

/// `--ignore-dynamic-mpd` (`dynamic_mpd: False`): no Periods, so upstream's
/// "No video formats found!"; the last of the paired options wins.
#[test]
fn ignore_dynamic_mpd_drops_the_formats_and_allow_restores_them() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let url = server.url("dynamic-number.mpd");
    for flag in ["--ignore-dynamic-mpd", "--no-allow-dynamic-mpd"] {
        let output = run(directory.path(), &["-j", flag, &url]);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(
            text(&output.stderr).ends_with(
                "ERROR: [generic] dynamic-number: No video formats found!; please report this issue on  https://github.com/yt-dlp/yt-dlp/issues?q= , filling out the appropriate issue template. Confirm you are on the latest version using  yt-dlp -U\n"
            ),
            "{}",
            text(&output.stderr)
        );
    }
    // `--ignore-no-formats-error` prints the formats-less live result.
    let output = run(
        directory.path(),
        &[
            "--ignore-dynamic-mpd",
            "--ignore-no-formats-error",
            "--print",
            "%(live_status)s|%(formats)s",
            &url,
        ],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "is_live|[]\n");
    assert!(text(&output.stderr).contains("WARNING: No video formats found!\n"));
    for flags in [
        ["--ignore-dynamic-mpd", "--allow-dynamic-mpd"],
        ["--ignore-dynamic-mpd", "--no-ignore-dynamic-mpd"],
    ] {
        let output = run(
            directory.path(),
            &[flags[0], flags[1], "-s", "--print", "format_id", &url],
        );
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "v1+a1\n");
    }
    server.assert_only_manifests();
}

/// No format of a dynamic MPD is `is_from_start`.
#[test]
fn live_from_start_has_no_dynamic_dash_formats() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &["--live-from-start", "-j", &server.url("dynamic-number.mpd")],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).ends_with(
            "ERROR: [generic] dynamic-number: --live-from-start is passed, but there are no formats that can be downloaded from the start. If you want to download from the current time, use --no-live-from-start\n"
        ),
        "{}",
        text(&output.stderr)
    );
}

#[cfg(unix)]
mod download {
    use super::*;

    /// A fake FFmpeg recording its arguments (NUL-separated) in `args`
    /// and writing `live` to its output (the last argument, a `file:` URL).
    fn fake_ffmpeg(directory: &Path) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("ffmpeg");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 case \"$1\" in -version) echo 'ffmpeg version 7.0 Copyright'; exit 0;; esac\n\
                 printf '%s\\0' \"$@\" > '{}'\n\
                 for out; do :; done\n\
                 printf live > \"${{out#file:}}\"\n",
                directory.join("args").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    fn recorded(directory: &Path) -> Vec<String> {
        let args = std::fs::read_to_string(directory.join("args")).unwrap();
        args.split_terminator('\0').map(str::to_owned).collect()
    }

    /// Pinned upstream's `-headers` value for these downloads.
    const HEADERS: &str = "User-Agent: fixture\r\nAccept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8\r\nAccept-Language: en-us,en;q=0.5\r\nSec-Fetch-Mode: navigate\r\n";

    /// Upstream's arguments, except that this port's FFmpegFD also sends the
    /// page as `Referer` for any non-direct download (also static DASH).
    fn without_referer(args: Vec<String>, manifest: &str) -> Vec<String> {
        let referer = format!("Referer: {manifest}\r\n");
        args.into_iter()
            .map(|arg| arg.replace(&referer, ""))
            .collect()
    }

    /// `_get_suitable_downloader`: live `http_dash_segments` uses FFmpegFD on
    /// the manifest URL with `-re` per input and `-map I:manifest_stream_number`,
    /// merged into one file and without the DASH fixups (upstream's
    /// arguments, captured with the same fake FFmpeg).
    #[test]
    fn live_dash_downloads_through_ffmpeg_with_upstream_arguments() {
        let server = Server::new();
        let tools = tempfile::tempdir().unwrap();
        let ffmpeg = fake_ffmpeg(tools.path());
        let manifest = server.url("dynamic-timeline.mpd");
        for (format, expected) in [
            (
                "v+a",
                vec![
                    "-y",
                    "-hide_banner",
                    "-headers",
                    HEADERS,
                    "-re",
                    "-i",
                    &manifest,
                    "-headers",
                    HEADERS,
                    "-re",
                    "-i",
                    &manifest,
                    "-c",
                    "copy",
                    "-map",
                    "0:0",
                    "-map",
                    "1:1",
                    "-f",
                    "mp4",
                    "file:o.mp4.part",
                ],
            ),
            (
                "a",
                vec![
                    "-y",
                    "-hide_banner",
                    "-headers",
                    HEADERS,
                    "-re",
                    "-i",
                    &manifest,
                    "-c",
                    "copy",
                    "-map",
                    "0:1",
                    "-f",
                    "ipod",
                    "file:o.m4a.part",
                ],
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let output = run(
                directory.path(),
                &[
                    "--ffmpeg-location",
                    &ffmpeg,
                    "-f",
                    format,
                    "-o",
                    "o.%(ext)s",
                    &manifest,
                ],
            );
            let stdout = text(&output.stdout);
            assert!(output.status.success(), "{}", text(&output.stderr));
            assert_eq!(
                without_referer(recorded(tools.path()), &manifest),
                expected,
                "{format}"
            );
            let ext = if format == "a" { "m4a" } else { "mp4" };
            assert_eq!(
                std::fs::read(directory.path().join(format!("o.{ext}"))).unwrap(),
                b"live"
            );
            // Upstream's DASH fixups follow DashSegmentsFD and native m4a
            // downloads only.
            assert!(!stdout.contains("[Fixup"), "{stdout}");
            assert!(!stdout.contains("Total fragments"), "{stdout}");
        }
        server.assert_only_manifests();
    }

    /// `--downloader native` keeps DashSegmentsFD, whose live check is
    /// upstream's `Live DASH videos are not supported` (upstream then tries
    /// the snapshot's fragments; this port stops). Without ffmpeg, FFmpegFD
    /// fails like upstream.
    #[test]
    fn native_or_missing_ffmpeg_live_dash_downloads_fail_explicitly() {
        let server = Server::new();
        let tools = tempfile::tempdir().unwrap();
        let ffmpeg = fake_ffmpeg(tools.path());
        let manifest = server.url("dynamic-timeline.mpd");
        for (args, message) in [
            (
                vec!["--ffmpeg-location", &ffmpeg, "--downloader", "native"],
                "ERROR: Live DASH videos are not supported\n",
            ),
            (
                vec!["--ffmpeg-location", &ffmpeg, "--downloader", "dash:native"],
                "ERROR: Live DASH videos are not supported\n",
            ),
            (
                vec!["--ffmpeg-location", "/nonexistent/ffmpeg"],
                "ERROR: m3u8 download detected but ffmpeg could not be found. Please install\n",
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut args = args.clone();
            args.extend(["-f", "v", "-o", "o.%(ext)s", &manifest]);
            let output = run(directory.path(), &args);
            let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
            assert_eq!(output.status.code(), Some(1), "{stdout}{stderr}");
            assert!(stderr.contains(message), "{stderr}");
            assert!(!stdout.contains("Total fragments"), "{stdout}");
            assert!(!tools.path().join("args").exists());
            assert_eq!(
                std::fs::read_dir(directory.path()).unwrap().count(),
                0,
                "{args:?}"
            );
        }
        server.assert_only_manifests();
    }
}
