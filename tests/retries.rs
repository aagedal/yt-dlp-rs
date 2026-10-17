use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use yt_dlp_rs::{DownloadOptions, YoutubeDl};

struct Response {
    status: u16,
    mime: &'static str,
    body: &'static str,
    length: usize,
}

impl Response {
    fn media() -> Self {
        Self {
            status: 200,
            mime: "video/mp4",
            body: "complete media",
            length: 14,
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            ..Self::media()
        }
    }
}

// The first media GET is extraction's probe; subsequent GETs are downloads.
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(respond: impl Fn(&str, usize) -> Response + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let recorded = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let worker = thread::spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => panic!("fixture accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 1024];
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(length) => bytes.extend_from_slice(&buffer[..length]),
                    }
                }
                if !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    continue;
                }
                let request = String::from_utf8(bytes).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                let count = {
                    let mut requests = recorded.lock().unwrap();
                    requests.push(request);
                    requests
                        .iter()
                        .filter(|request| request.split_whitespace().nth(1) == Some(path.as_str()))
                        .count()
                };
                let response = respond(&path, count);
                let wire = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status, response.mime, response.length, response.body
                );
                let _ = stream.write_all(wire.as_bytes());
            }
        });
        Self {
            base,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn requests_for(&self, path: &str) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.split_whitespace().nth(1) == Some(path))
            .cloned()
            .collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn directory_names(directory: &std::path::Path) -> Vec<String> {
    let mut names = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn transient_statuses_retry_and_preserve_complete_output() {
    for status in [408, 429, 500, 502, 503, 504] {
        let server = Server::start(move |_, count| {
            if count == 2 {
                Response::status(status)
            } else {
                Response::media()
            }
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("clip.mp4");
        assert_eq!(
            downloader
                .download_with_options(&info, &destination, DownloadOptions { retries: 1 })
                .unwrap(),
            14
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"complete media");
        assert_eq!(server.requests_for("/clip.mp4").len(), 3, "status {status}");
        assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
    }
}

#[test]
fn truncated_body_restarts_without_range_or_partial_bytes() {
    let server = Server::start(|_, count| {
        if count == 2 {
            Response {
                body: "partial",
                length: 100,
                ..Response::media()
            }
        } else {
            Response::media()
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    assert_eq!(
        downloader
            .download_with_options(&info, &destination, DownloadOptions { retries: 1 })
            .unwrap(),
        14
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"complete media");
    let requests = server.requests_for("/clip.mp4");
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| !request.to_ascii_lowercase().contains("\r\nrange:"))
    );
    assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
}

#[test]
fn exhausted_retries_count_additional_attempts_and_clean_temporary_files() {
    for truncated in [false, true] {
        let server = Server::start(move |_, count| {
            if count == 1 {
                Response::media()
            } else if truncated {
                Response {
                    body: "partial",
                    length: 100,
                    ..Response::media()
                }
            } else {
                Response::status(503)
            }
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let error = downloader
            .download_with_options(
                &info,
                directory.path().join("clip.mp4"),
                DownloadOptions { retries: 2 },
            )
            .unwrap_err();
        assert_eq!(
            server.requests_for("/clip.mp4").len(),
            4,
            "truncated={truncated}: {error}"
        );
        assert!(directory_names(directory.path()).is_empty());
    }
}

#[test]
fn permanent_download_failures_are_not_retried() {
    for status in [401, 404, 501, 206] {
        let server = Server::start(move |_, count| {
            if count == 1 {
                Response::media()
            } else {
                Response::status(status)
            }
        });
        let downloader = YoutubeDl::new().unwrap();
        let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        assert!(
            downloader
                .download_with_options(
                    &info,
                    directory.path().join("clip.mp4"),
                    DownloadOptions { retries: 2 }
                )
                .is_err()
        );
        assert_eq!(server.requests_for("/clip.mp4").len(), 2, "status {status}");
        assert!(directory_names(directory.path()).is_empty());
    }
}

#[test]
fn existing_destination_is_preserved_without_attempting_download() {
    let server = Server::start(|_, _| Response::media());
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    std::fs::write(&destination, "existing file").unwrap();
    assert!(
        downloader
            .download_with_options(&info, &destination, DownloadOptions { retries: 2 })
            .is_err()
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"existing file");
    assert_eq!(server.requests_for("/clip.mp4").len(), 1);
    assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
}

#[test]
fn html5_page_referrer_is_preserved_on_every_retry() {
    let server = Server::start(|path, count| {
        if path == "/page.html" {
            let body = "<video src='/clip.mp4'></video>";
            Response {
                mime: "text/html",
                body,
                length: body.len(),
                ..Response::media()
            }
        } else if count == 2 {
            Response::status(503)
        } else {
            Response::media()
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let page_url = server.url("/page.html");
    let info = downloader.extract_info(&page_url).unwrap();
    let directory = tempfile::tempdir().unwrap();
    downloader
        .download_with_options(
            &info,
            directory.path().join("clip.mp4"),
            DownloadOptions { retries: 1 },
        )
        .unwrap();
    assert_eq!(server.requests_for("/page.html").len(), 1);
    let requests = server.requests_for("/clip.mp4");
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|request| {
        request
            .to_ascii_lowercase()
            .contains(&format!("\r\nreferer: {page_url}\r\n"))
    }));
}

#[test]
fn extraction_failures_are_not_retried() {
    let server = Server::start(|_, _| Response::status(503));
    assert!(
        YoutubeDl::new()
            .unwrap()
            .extract_info(&server.url("/clip.mp4"))
            .is_err()
    );
    assert_eq!(server.requests_for("/clip.mp4").len(), 1);
}

#[test]
fn default_download_retries_three_times() {
    assert_eq!(DownloadOptions::default().retries, 3);
    let server = Server::start(|_, count| {
        if (2..=4).contains(&count) {
            Response::status(503)
        } else {
            Response::media()
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        downloader
            .download(&info, directory.path().join("clip.mp4"))
            .unwrap(),
        14
    );
    assert_eq!(server.requests_for("/clip.mp4").len(), 5);
}

#[test]
fn cli_zero_retries_fails_once_and_continues_to_the_next_url() {
    let server = Server::start(|path, count| {
        if path == "/failed.mp4" && count > 1 {
            Response::status(503)
        } else {
            Response::media()
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--retries",
            "0",
            "-o",
            "%(id)s.%(ext)s",
            &server.url("/failed.mp4"),
            &server.url("/success.mp4"),
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(server.requests_for("/failed.mp4").len(), 2);
    assert_eq!(server.requests_for("/success.mp4").len(), 2);
    assert_eq!(
        std::fs::read(directory.path().join("success.mp4")).unwrap(),
        b"complete media"
    );
    assert_eq!(directory_names(directory.path()), ["success.mp4"]);
}

#[test]
fn cli_rejects_retry_values_outside_bounded_integer_range() {
    for value in ["-1", "256", "infinity", "1.5"] {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args([
                &format!("--retries={value}"),
                "http://127.0.0.1:1/unused.mp4",
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "value {value}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn html5_source_that_stops_serving_media_is_not_retried() {
    let server = Server::start(|path, count| {
        if path == "/page.html" {
            let body = "<video src='/clip.mp4'></video>";
            Response {
                mime: "text/html",
                body,
                length: body.len(),
                ..Response::media()
            }
        } else if count == 1 {
            Response::media()
        } else {
            Response {
                mime: "text/html",
                ..Response::media()
            }
        }
    });
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/page.html")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert!(
        downloader
            .download_with_options(
                &info,
                directory.path().join("clip.mp4"),
                DownloadOptions { retries: 2 }
            )
            .is_err()
    );
    assert_eq!(server.requests_for("/clip.mp4").len(), 2);
    assert!(directory_names(directory.path()).is_empty());
}

#[test]
fn filesystem_failure_does_not_retry_or_make_download_requests() {
    let server = Server::start(|_, _| Response::media());
    let downloader = YoutubeDl::new().unwrap();
    let info = downloader.extract_info(&server.url("/clip.mp4")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("parent");
    std::fs::write(&parent, "existing file").unwrap();
    assert!(
        downloader
            .download_with_options(
                &info,
                parent.join("clip.mp4"),
                DownloadOptions { retries: 2 }
            )
            .is_err()
    );
    assert_eq!(server.requests_for("/clip.mp4").len(), 1);
    assert_eq!(std::fs::read(parent).unwrap(), b"existing file");
    assert_eq!(directory_names(directory.path()), ["parent"]);
}

#[test]
fn cli_one_retry_recovers_from_transient_download_failure() {
    let server = Server::start(|_, count| {
        if count == 2 {
            Response::status(503)
        } else {
            Response::media()
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--retries", "1", "-o", "clip.mp4", &server.url("/clip.mp4")])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(server.requests_for("/clip.mp4").len(), 3);
    assert_eq!(
        std::fs::read(directory.path().join("clip.mp4")).unwrap(),
        b"complete media"
    );
    assert_eq!(directory_names(directory.path()), ["clip.mp4"]);
}
