use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use yt_dlp_rs::{Error, YoutubeDl, output::render_template};

// Local-only fixture: no live sites, Python installation, or external test services.
struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let worker = thread::spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => panic!("fixture accept failed: {e}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                let deadline = Instant::now() + Duration::from_secs(2);
                while !request.windows(4).any(|w| w == b"\r\n\r\n") && Instant::now() < deadline {
                    match stream.read(&mut buffer) {
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                // A client may abandon a speculative connection. Never answer an
                // empty/incomplete request with a response for a fabricated path.
                if !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    continue;
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (status, mime, body, extra, length) = match path {
                    "/redirect" => (
                        "302 Found",
                        "text/plain",
                        "",
                        "Location: /hello%20world.mp4?token=secret\r\n",
                        0,
                    ),
                    "/hello%20world.mp4?token=secret" | "/clip.mp4" => {
                        ("200 OK", "video/mp4", "media bytes", "", 11)
                    }
                    "/audio" => ("200 OK", "audio/mpeg; charset=binary", "audio", "", 5),
                    "/opaque.webm" => ("200 OK", "application/octet-stream", "webm", "", 4),
                    "/truncated.mp4" => ("200 OK", "video/mp4", "short", "", 100),
                    "/partial.mp4" => (
                        "206 Partial Content",
                        "video/mp4",
                        "short",
                        "Content-Range: bytes 0-4/100\r\n",
                        5,
                    ),
                    "/page.mp4" => ("200 OK", "text/html", "<html>not media</html>", "", 22),
                    "/playlist.m3u8" => {
                        ("200 OK", "application/vnd.apple.mpegurl", "#EXTM3U", "", 7)
                    }
                    _ => ("404 Not Found", "text/plain", "missing", "", 7),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {length}\r\n{extra}Connection: close\r\n\r\n{body}"
                );
                // Extraction intentionally closes a response after inspecting headers.
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            base,
            stop,
            worker: Some(worker),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn extracts_redirected_media_and_downloads_exact_bytes() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let info = dl.extract_info(&server.url("/redirect")).unwrap();
    assert_eq!(info.title, "hello world");
    assert_eq!(info.id, "hello world");
    assert_eq!(info.ext, "mp4");
    assert_eq!(info.filesize, Some(11));
    assert_eq!(info.webpage_url.as_str(), server.url("/redirect"));
    assert!(info.url.as_str().ends_with("?token=secret"));
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("nested/video.mp4");
    assert_eq!(dl.download(&info, &output).unwrap(), 11);
    assert_eq!(std::fs::read(&output).unwrap(), b"media bytes");
    assert!(matches!(
        dl.download(&info, &output),
        Err(Error::OutputExists(_))
    ));
    assert_eq!(std::fs::read(&output).unwrap(), b"media bytes");
}

#[test]
fn recognizes_mime_and_binary_extension_fallback() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    assert_eq!(dl.extract_info(&server.url("/audio")).unwrap().ext, "mp3");
    assert_eq!(
        dl.extract_info(&server.url("/opaque.webm")).unwrap().ext,
        "webm"
    );
}

#[test]
fn rejects_pages_manifests_errors_and_non_http_urls() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    for path in ["/page.mp4", "/playlist.m3u8"] {
        assert!(matches!(
            dl.extract_info(&server.url(path)),
            Err(Error::UnsupportedUrl(_))
        ));
    }
    assert!(matches!(
        dl.extract_info(&server.url("/missing")),
        Err(Error::Http(_))
    ));
    assert!(matches!(
        dl.extract_info("file:///tmp/video.mp4"),
        Err(Error::UnsupportedScheme(_))
    ));
    assert!(matches!(
        dl.extract_info("not a url"),
        Err(Error::InvalidUrl(_))
    ));
}

#[test]
fn truncated_transfers_do_not_publish_or_leave_temporary_files() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let info = dl.extract_info(&server.url("/truncated.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("video.mp4");
    assert!(dl.download(&info, &output).is_err());
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn unsolicited_partial_responses_cannot_be_published_as_complete_media() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    assert!(matches!(
        dl.extract_info(&server.url("/partial.mp4")),
        Err(Error::PartialResponse)
    ));
    let mut info = dl.extract_info(&server.url("/clip.mp4")).unwrap();
    info.url = server.url("/partial.mp4").parse().unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        dl.download(&info, directory.path().join("clip.mp4")),
        Err(Error::PartialResponse)
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn templates_are_explicit_and_metadata_cannot_inject_paths_or_fields() {
    let server = Server::start();
    let mut info = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/clip.mp4"))
        .unwrap();
    info.title = "../folder/%(id)s".into();
    assert_eq!(
        render_template("downloads/%%-%(title)s.%(ext)s", &info)
            .unwrap()
            .to_str()
            .unwrap(),
        "downloads/%-_folder_%(id)s.mp4"
    );
    for invalid in ["%(unknown)s", "%(title)d", "%", "", "-"] {
        assert!(render_template(invalid, &info).is_err(), "{invalid}");
    }
}

#[test]
fn cli_json_continues_after_errors_and_returns_failure() {
    let server = Server::start();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--dump-json",
            &server.url("/missing"),
            &server.url("/clip.mp4"),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    let info: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(info["extractor_key"], "Generic");
    assert_eq!(info["filesize"], 11);
    assert!(String::from_utf8(output.stderr).unwrap().contains("404"));
}

#[test]
fn cli_simulate_and_download() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("clip.mp4");
    let args = ["-o", target.to_str().unwrap(), &server.url("/clip.mp4")];
    let simulated = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--simulate")
        .args(args)
        .output()
        .unwrap();
    assert!(simulated.status.success());
    assert!(!target.exists());
    let downloaded = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(args)
        .output()
        .unwrap();
    assert!(downloaded.status.success());
    assert_eq!(std::fs::read(target).unwrap(), b"media bytes");
}
