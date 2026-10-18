//! `--max-downloads` and `--min/max-filesize` through local HTTP media and the CLI.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
struct Server {
    origin: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (status, body) = if path == "/big.mp4" {
                    (200, "x".repeat(96 * 1024))
                } else if path.ends_with(".mp4") {
                    (200, "media".to_owned())
                } else {
                    (404, "missing".to_owned())
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            origin,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn cli(args: &[&str], directory: &Path, stdin: Option<&[u8]>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(stdin.unwrap_or_default()).unwrap();
    drop(input);
    child.wait_with_output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn max_downloads_stops_after_processed_videos_with_exit_101() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let (a, b) = (server.url("/a.mp4"), server.url("/b.mp4"));
    let output = cli(&["--max-downloads", "1", &a, &b], directory.path(), None);
    assert_eq!(output.status.code(), Some(101), "{}", stderr(&output));
    assert!(
        stderr(&output).contains(
            "[info] Maximum number of downloads reached, stopping due to --max-downloads"
        )
    );
    assert!(directory.path().join("a [a].mp4").exists());
    assert!(!directory.path().join("b [b].mp4").exists());
    // Simulated videos count too; 0 means no limit.
    let output = cli(
        &["--max-downloads", "1", "-O", "id", &a, &b],
        directory.path(),
        None,
    );
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a\n");
    let output = cli(
        &["--max-downloads", "0", "-O", "id", &a, &b],
        directory.path(),
        None,
    );
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a\nb\n");
}

#[test]
fn filesize_limits_skip_media_without_error_or_archive() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    let url = server.url("/a.mp4");
    for (flag, size, message) in [
        (
            "--max-filesize",
            "4",
            "File is larger than max-filesize (5 bytes > 4 bytes). Aborting.",
        ),
        (
            "--min-filesize",
            "1k",
            "File is smaller than min-filesize (5 bytes < 1024 bytes). Aborting.",
        ),
    ] {
        let output = cli(
            &[
                flag,
                size,
                "--download-archive",
                archive.to_str().unwrap(),
                &url,
            ],
            directory.path(),
            None,
        );
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(
            stderr(&output).contains(&format!("[download] {message}")),
            "{}",
            stderr(&output)
        );
        assert!(!archive.exists());
        assert!(!directory.path().join("a [a].mp4").exists());
    }
    let output = cli(&["--max-filesize", "5", &url], directory.path(), None);
    assert!(output.status.success());
    assert!(directory.path().join("a [a].mp4").exists());
    let invalid = cli(&["--min-filesize", "1KiB", &url], directory.path(), None);
    assert!(!invalid.status.success());
    assert!(stderr(&invalid).contains("invalid min filesize \"1KiB\" given"));
}

#[test]
fn quiet_and_no_warnings_suppress_screen_and_warning_lines() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let url = server.url("/a.mp4");
    let output = cli(&["-q", &url], directory.path(), None);
    assert!(output.status.success());
    assert_eq!(stderr(&output), "");
    assert!(directory.path().join("a [a].mp4").exists());
    // Errors are still reported when quiet.
    let output = cli(&["-q", &server.url("/missing.txt")], directory.path(), None);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("ERROR:"));
    // --no-quiet restores messages; the last flag wins.
    let output = cli(
        &["-q", "--no-quiet", "--force-overwrites", &url],
        directory.path(),
        None,
    );
    assert!(stderr(&output).contains("[download] Destination:"));
    let output = cli(
        &[
            "--no-warnings",
            "--remux-video",
            "mp4",
            "--recode-video",
            "mkv",
            "-s",
            &url,
        ],
        directory.path(),
        None,
    );
    assert!(!stderr(&output).contains("WARNING:"), "{}", stderr(&output));
    let output = cli(
        &["--remux-video", "mp4", "--recode-video", "mkv", "-s", &url],
        directory.path(),
        None,
    );
    assert!(stderr(&output).contains("WARNING: --remux-video is ignored"));
}

#[test]
fn limit_rate_slows_media_transfers_and_rejects_zero() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let url = server.url("/big.mp4");
    let start = std::time::Instant::now();
    let output = cli(&["-r", "128K", &url], directory.path(), None);
    assert!(output.status.success(), "{}", stderr(&output));
    // 96 KiB at 128 KiB/s takes at least ~0.75 s.
    assert!(
        start.elapsed() >= Duration::from_millis(600),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(
        std::fs::metadata(directory.path().join("big [big].mp4"))
            .unwrap()
            .len(),
        96 * 1024
    );
    let zero = cli(&["--limit-rate", "0", &url], directory.path(), None);
    assert!(!zero.status.success());
    assert!(stderr(&zero).contains("rate limit \"0\" must be positive"));
}
