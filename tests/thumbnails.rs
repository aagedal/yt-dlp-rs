//! HTML5 poster thumbnails: listing and `--write-(all-)thumbnail(s)` through the CLI.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
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
                let (status, mime, body) = match path.as_str() {
                    "/page.html" => (
                        200,
                        "text/html",
                        "<title>Poster</title><video poster='/poster.jpg' src='/media.mp4'></video>",
                    ),
                    "/missing.html" => (
                        200,
                        "text/html",
                        "<title>Gone</title><video poster='/nope.jpg' src='/media.mp4'></video>",
                    ),
                    "/plain.html" => (
                        200,
                        "text/html",
                        "<title>Plain</title><video src='/media.mp4'></video>",
                    ),
                    "/media.mp4" => (200, "video/mp4", "media"),
                    "/poster.jpg" => (200, "image/jpeg", "jpeg"),
                    _ => (404, "text/plain", "missing"),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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

fn cli(args: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn list_thumbnails_prints_the_sanitized_poster_table_without_downloads() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = cli(
        &["--list-thumbnails", &server.url("/page.html")],
        directory.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        format!(
            "[info] Available thumbnails for page-1:\nID Width   Height  URL\n0  unknown unknown {}\n",
            server.url("/poster.jpg")
        )
    );
    let output = cli(
        &["--list-thumbnails", &server.url("/plain.html")],
        directory.path(),
    );
    assert_eq!(text(&output.stdout), "plain-1 has no thumbnails\n");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let json = cli(&["-j", &server.url("/page.html")], directory.path());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["thumbnail"], server.url("/poster.jpg"));
    assert_eq!(value["thumbnails"][0]["id"], "0");
}

#[test]
fn write_thumbnail_follows_upstream_messages_overwrites_and_flag_order() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let page = server.url("/page.html");
    let output = cli(
        &[
            "--write-thumbnail",
            "--skip-download",
            "-o",
            "video.%(ext)s",
            &page,
        ],
        directory.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("[info] Downloading video thumbnail 0 ..."));
    assert!(text(&output.stderr).contains("[info] Writing video thumbnail 0 to: video.jpg"));
    assert_eq!(
        std::fs::read(directory.path().join("video.jpg")).unwrap(),
        b"jpeg"
    );
    assert!(!directory.path().join("video.mp4").exists());
    std::fs::write(directory.path().join("video.jpg"), b"old").unwrap();
    let output = cli(
        &[
            "-w",
            "--write-thumbnail",
            "--skip-download",
            "-o",
            "video.%(ext)s",
            &page,
        ],
        directory.path(),
    );
    assert!(text(&output.stderr).contains("[info] Video thumbnail is already present"));
    assert_eq!(
        std::fs::read(directory.path().join("video.jpg")).unwrap(),
        b"old"
    );
    // By default the existing thumbnail is replaced.
    cli(
        &[
            "--write-thumbnail",
            "--skip-download",
            "-o",
            "video.%(ext)s",
            &page,
        ],
        directory.path(),
    );
    assert_eq!(
        std::fs::read(directory.path().join("video.jpg")).unwrap(),
        b"jpeg"
    );
    // A missing thumbnail is reported, not fatal; the media still downloads.
    let missing = tempfile::tempdir().unwrap();
    let output = cli(
        &[
            "--write-thumbnail",
            "-o",
            "video.%(ext)s",
            &server.url("/missing.html"),
        ],
        missing.path(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("[info] Video Thumbnail 0 does not exist"));
    assert!(missing.path().join("video.mp4").exists());
    assert!(!missing.path().join("video.jpg").exists());
    // --no-write-thumbnail after --write-thumbnail disables writing.
    let disabled = tempfile::tempdir().unwrap();
    cli(
        &[
            "--write-thumbnail",
            "--no-write-thumbnail",
            "--skip-download",
            &page,
        ],
        disabled.path(),
    );
    assert_eq!(std::fs::read_dir(disabled.path()).unwrap().count(), 0);
    // --write-all-thumbnails with a single thumbnail uses the plain name.
    let all = tempfile::tempdir().unwrap();
    cli(
        &[
            "--write-all-thumbnails",
            "--write-thumbnail",
            "--skip-download",
            "-o",
            "v.%(ext)s",
            &page,
        ],
        all.path(),
    );
    assert_eq!(std::fs::read(all.path().join("v.jpg")).unwrap(), b"jpeg");
    let none = cli(
        &[
            "--write-thumbnail",
            "--skip-download",
            &server.url("/plain.html"),
        ],
        all.path(),
    );
    assert!(text(&none.stderr).contains("[info] There are no video thumbnails to download"));
}
