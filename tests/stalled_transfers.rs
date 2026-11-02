//! Interruptible transfers: a stop, cancel or SIGINT/SIGTERM ends a request
//! blocked on the network promptly instead of after the socket timeout
//! (20 s by default), keeping the same outcome as at a progress checkpoint.
//! Local servers accept and then never answer, stall inside the response
//! headers, or stall in the middle of the body.
mod support;

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    ops::ControlFlow,
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use url::Url;
use yt_dlp_rs::{
    DownloadControl, DownloadOptions, Error, ExtractResult, ExtractionOptions, Interrupt, Manifest,
    ManifestKind, VideoInfo, YoutubeDl,
};

/// How long an interrupted call may take to return (the transfer notices
/// within 100 ms; the rest is slack for loaded CI machines).
const BOUND: Duration = Duration::from_secs(3);

/// Body bytes sent before a mid-body stall, of [`LENGTH`] announced.
const SENT: usize = 1024;
const LENGTH: usize = 64 * 1024;

/// A server answering by path:
/// - `/silent*`: reads the request, never answers (also a TLS client's
///   handshake),
/// - `/midheaders*`: half a status line and header block, then nothing,
/// - `/stalled*`: whole headers and [`SENT`] bytes of [`LENGTH`], then nothing,
/// - `/sN.ts`: one MPEG-TS packet, except that `/s2.ts` stalls mid-body,
/// - `/index.m3u8`: an HLS playlist of four of those,
/// - `/page*`: a small HTML page.
///
/// Stalled connections stay open for a minute. Records requested paths.
struct Server {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let record = requests.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let record = record.clone();
                thread::spawn(move || {
                    let mut first = [0];
                    if stream.peek(&mut first).unwrap_or(0) == 0 {
                        return;
                    }
                    if first[0] == 0x16 {
                        // A TLS ClientHello: never answer.
                        record.lock().unwrap().push("tls".into());
                        thread::sleep(Duration::from_secs(60));
                        return;
                    }
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0
                            || header.trim().is_empty()
                        {
                            break;
                        }
                    }
                    record.lock().unwrap().push(path.clone());
                    let stall = |stream: &mut std::net::TcpStream, bytes: &[u8]| {
                        let _ = stream.write_all(bytes);
                        let _ = stream.flush();
                        thread::sleep(Duration::from_secs(60));
                    };
                    let kind = if path.ends_with(".m3u8") {
                        "application/vnd.apple.mpegurl"
                    } else if path.ends_with(".ts") {
                        "video/mp2t"
                    } else if path.contains("page") {
                        "text/html"
                    } else {
                        "video/mp4"
                    };
                    let head = |length: usize| {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\n\
                             Content-Length: {length}\r\nConnection: close\r\n\r\n"
                        )
                        .into_bytes()
                    };
                    if path.starts_with("/silent") {
                        stall(&mut stream, b"");
                    } else if path.starts_with("/midheaders") {
                        stall(&mut stream, b"HTTP/1.1 200 OK\r\nContent-Type: vid");
                    } else if path.starts_with("/stalled") || path == "/s2.ts" {
                        let mut bytes = head(LENGTH);
                        bytes.extend(vec![0x47; SENT]);
                        stall(&mut stream, &bytes);
                    } else if let Some(index) = path
                        .strip_prefix("/s")
                        .and_then(|rest| rest.strip_suffix(".ts"))
                        .and_then(|index| index.parse::<u8>().ok())
                    {
                        let mut bytes = head(188);
                        bytes.extend(segment(index));
                        let _ = stream.write_all(&bytes);
                    } else if path == "/index.m3u8" {
                        let text = playlist(port);
                        let mut bytes = head(text.len());
                        bytes.extend(text.into_bytes());
                        let _ = stream.write_all(&bytes);
                    } else if path.starts_with("/page") {
                        let page = b"<html><head><title>Page</title></head></html>";
                        let mut bytes = head(page.len());
                        bytes.extend(page);
                        let _ = stream.write_all(&bytes);
                    } else {
                        let _ = stream.write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                });
            }
        });
        Self { port, requests }
    }

    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://127.0.0.1:{}{path}", self.port)).unwrap()
    }

    fn wait_for(&self, path: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|seen| seen == path)
        {
            assert!(
                Instant::now() < deadline,
                "{path}: {:?}",
                self.requests.lock().unwrap()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn playlist(port: u16) -> String {
    let segments: String = (0..4)
        .map(|index| format!("#EXTINF:2.0,\nhttp://127.0.0.1:{port}/s{index}.ts\n"))
        .collect();
    format!("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n{segments}#EXT-X-ENDLIST\n")
}

fn segment(index: u8) -> Vec<u8> {
    let mut packet = vec![0x47; 188];
    packet[1] = index;
    packet
}

fn direct(url: &Url) -> VideoInfo {
    let mut info: VideoInfo = serde_json::from_value(serde_json::json!({
        "id": "v", "title": "V", "ext": "mp4", "protocol": url.scheme(),
        "url": url.as_str(), "webpage_url": url.as_str(),
        "extractor": "generic", "extractor_key": "Generic", "direct": true,
    }))
    .unwrap();
    info.formats.clear();
    info
}

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Run `call` while another thread waits for `path` to be requested, waits
/// a little longer (the transfer is blocked by then) and calls `interrupt`.
/// Returns the result and how long the call took after the interrupt.
fn interrupted<T: std::fmt::Debug>(
    server: &Server,
    path: &str,
    interrupt: impl FnOnce() + Send,
    call: impl FnOnce() -> T,
) -> (T, Duration) {
    let sent = Mutex::new(None);
    thread::scope(|scope| {
        scope.spawn(|| {
            server.wait_for(path);
            thread::sleep(Duration::from_millis(300));
            *sent.lock().unwrap() = Some(Instant::now());
            interrupt();
        });
        let result = call();
        let sent = sent
            .lock()
            .unwrap()
            .unwrap_or_else(|| panic!("returned before the interrupt: {result:?}"));
        (result, sent.elapsed())
    })
}

fn download(
    client: &YoutubeDl,
    info: &VideoInfo,
    destination: &Path,
    interrupt: &Interrupt,
) -> Result<u64, Error> {
    let control = DownloadControl {
        interrupt: Some(interrupt.clone()),
        ..Default::default()
    };
    client.download_with_control(
        info,
        destination,
        DownloadOptions::default(),
        &control,
        |_| ControlFlow::Continue(()),
    )
}

#[test]
fn a_stop_ends_a_stalled_body_and_keeps_the_received_bytes() {
    let server = Server::new();
    let client = YoutubeDl::new().unwrap();
    let info = direct(&server.url("/stalled.mp4"));
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("v.mp4");
    let interrupt = Interrupt::new();
    let (result, took) = interrupted(
        &server,
        "/stalled.mp4",
        || interrupt.stop(),
        || download(&client, &info, &destination, &interrupt),
    );
    assert!(took < BOUND, "{took:?}");
    match result {
        Err(Error::DownloadStopped { path }) => assert_eq!(path, destination),
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read(&destination).unwrap(), vec![0x47; SENT]);
    assert_eq!(names(directory.path()), ["v.mp4"]);
}

#[test]
fn a_cancel_ends_a_stalled_body_and_deletes_it() {
    let server = Server::new();
    let client = YoutubeDl::new().unwrap();
    let info = direct(&server.url("/stalled-cancel.mp4"));
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("v.mp4");
    let interrupt = Interrupt::new();
    let (result, took) = interrupted(
        &server,
        "/stalled-cancel.mp4",
        || interrupt.cancel(),
        || download(&client, &info, &destination, &interrupt),
    );
    assert!(took < BOUND, "{took:?}");
    assert!(
        matches!(result, Err(Error::DownloadCancelled)),
        "{result:?}"
    );
    assert!(names(directory.path()).is_empty());
}

#[test]
fn an_interrupt_ends_a_wait_for_response_headers() {
    for (path, stop) in [("/silent.mp4", false), ("/midheaders.mp4", true)] {
        let server = Server::new();
        let client = YoutubeDl::new().unwrap();
        let info = direct(&server.url(path));
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("v.mp4");
        let interrupt = Interrupt::new();
        let (result, took) = interrupted(
            &server,
            path,
            || {
                if stop {
                    interrupt.stop();
                } else {
                    interrupt.cancel();
                }
            },
            || download(&client, &info, &destination, &interrupt),
        );
        assert!(took < BOUND, "{path}: {took:?}");
        // A stop before any data arrived is a cancel.
        assert!(
            matches!(result, Err(Error::DownloadCancelled)),
            "{path}: {result:?}"
        );
        assert!(names(directory.path()).is_empty(), "{path}");
    }
}

#[test]
fn a_cancel_ends_a_stalled_tls_handshake() {
    let server = Server::new();
    let client = YoutubeDl::new().unwrap();
    let url = Url::parse(&format!("https://127.0.0.1:{}/silent.mp4", server.port)).unwrap();
    let info = direct(&url);
    let directory = tempfile::tempdir().unwrap();
    let interrupt = Interrupt::new();
    let (result, took) = interrupted(
        &server,
        "tls",
        || interrupt.cancel(),
        || download(&client, &info, &directory.path().join("v.mp4"), &interrupt),
    );
    assert!(took < BOUND, "{took:?}");
    assert!(
        matches!(result, Err(Error::DownloadCancelled)),
        "{result:?}"
    );
    assert!(names(directory.path()).is_empty());
}

/// An HLS download whose third fragment stalls mid-body, with `concurrency`
/// fragment workers.
fn stalled_fragments(concurrency: usize, stop: bool) {
    let server = Server::new();
    let text = playlist(server.port);
    let client = YoutubeDl::new()
        .unwrap()
        .with_concurrent_fragments(concurrency);
    let mut result = client
        .extract_manifest(
            &Manifest::inline(ManifestKind::Hls, server.url("/index.m3u8"), text),
            ExtractionOptions::default(),
        )
        .unwrap();
    client.select_result(&mut result, None).unwrap();
    let ExtractResult::Video(info) = result else {
        panic!("expected a video")
    };
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("v.mp4");
    let interrupt = Interrupt::new();
    let (result, took) = interrupted(
        &server,
        "/s2.ts",
        || {
            if stop {
                interrupt.stop();
            } else {
                interrupt.cancel();
            }
        },
        || download(&client, &info, &destination, &interrupt),
    );
    assert!(took < BOUND, "-N {concurrency}: {took:?}");
    if stop {
        match result {
            Err(Error::DownloadStopped { path }) => assert_eq!(path, destination),
            other => panic!("-N {concurrency}: {other:?}"),
        }
        // Whole fragments only: the stalled one is dropped.
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            [segment(0), segment(1)].concat()
        );
        assert_eq!(names(directory.path()), ["v.mp4"]);
    } else {
        assert!(
            matches!(result, Err(Error::DownloadCancelled)),
            "-N {concurrency}: {result:?}"
        );
        assert!(names(directory.path()).is_empty());
    }
}

#[test]
fn a_stop_ends_a_stalled_fragment_and_keeps_the_whole_ones() {
    stalled_fragments(1, true);
}

#[test]
fn a_stop_ends_stalled_concurrent_fragments() {
    stalled_fragments(3, true);
}

#[test]
fn a_cancel_ends_stalled_concurrent_fragments() {
    stalled_fragments(3, false);
}

#[test]
fn an_interrupt_ends_a_stalled_extraction_request() {
    for path in ["/silent-page", "/stalled-page"] {
        let server = Server::new();
        let client = YoutubeDl::new().unwrap();
        let interrupt = Interrupt::new();
        let url = server.url(path);
        let (result, took) = interrupted(
            &server,
            path,
            || interrupt.stop(),
            || {
                client.extract_result_unselected_with_options(
                    url.as_str(),
                    ExtractionOptions {
                        interrupt: Some(interrupt.clone()),
                        ..Default::default()
                    },
                )
            },
        );
        assert!(took < BOUND, "{path}: {took:?}");
        assert!(
            matches!(result, Err(Error::ExtractionCancelled)),
            "{path}: {result:?}"
        );
    }
    // A fetched manifest that stalls mid-body.
    let server = Server::new();
    let client = YoutubeDl::new().unwrap();
    let interrupt = Interrupt::new();
    let (result, took) = interrupted(
        &server,
        "/stalled.m3u8",
        || interrupt.cancel(),
        || {
            client.extract_manifest(
                &Manifest::new(ManifestKind::Hls, server.url("/stalled.m3u8")),
                ExtractionOptions {
                    interrupt: Some(interrupt.clone()),
                    ..Default::default()
                },
            )
        },
    );
    assert!(took < BOUND, "{took:?}");
    assert!(
        matches!(result, Err(Error::ExtractionCancelled)),
        "{result:?}"
    );
}

#[test]
fn healthy_transfers_are_unaffected_by_an_interrupt_handle() {
    // The same 1 MB body arrives whole through the watched transport.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let body: Vec<u8> = (0..1024 * 1024).map(|index| (index % 251) as u8).collect();
    let served = body.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let served = served.clone();
            thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n",
                    served.len()
                );
                let _ = stream.write_all(head.as_bytes());
                for chunk in served.chunks(7000) {
                    let _ = stream.write_all(chunk);
                }
            });
        }
    });
    let url = Url::parse(&format!("http://127.0.0.1:{port}/video.mp4")).unwrap();
    let client = YoutubeDl::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("v.mp4");
    let size = download(&client, &direct(&url), &destination, &Interrupt::new()).unwrap();
    assert_eq!(size, body.len() as u64);
    assert_eq!(std::fs::read(&destination).unwrap(), body);
}

/// SIGINT/SIGTERM to the CLI (Ctrl+Break on Windows). SIGTERM is sent, as
/// background test processes may inherit an ignored SIGINT.
mod signals {
    use super::*;
    use std::process::{Child, Command, Stdio};

    fn spawn(directory: &Path, args: &[&str]) -> Child {
        support::stoppable(&mut Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs")))
            .args([
                "--ignore-config",
                "--fixup",
                "never",
                "-o",
                "%(id)s.%(ext)s",
            ])
            .args(args)
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    /// Signal the run once `path` was requested and the transfer blocks;
    /// returns the exit code, stderr, stdout and the directory's names.
    fn interrupted_run(
        server: &Server,
        path: &str,
        args: &[&str],
    ) -> (Option<i32>, String, String, Vec<String>) {
        let directory = tempfile::tempdir().unwrap();
        let child = spawn(directory.path(), args);
        server.wait_for(path);
        thread::sleep(Duration::from_millis(300));
        let sent = Instant::now();
        support::stop(&child);
        let output = child.wait_with_output().unwrap();
        let took = sent.elapsed();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(took < BOUND, "{path}: {took:?}\n{stderr}");
        (
            output.status.code(),
            stderr,
            String::from_utf8_lossy(&output.stdout).into_owned(),
            names(directory.path()),
        )
    }

    fn assert_interrupted(code: Option<i32>, stderr: &str) {
        assert_eq!(code, Some(1), "{stderr}");
        assert!(stderr.ends_with("ERROR: Interrupted by user\n"), "{stderr}");
    }

    #[test]
    fn a_signal_ends_a_wait_for_response_headers() {
        for path in ["/silent.mp4", "/midheaders.mp4"] {
            let server = Server::new();
            let url = server.url(path);
            let (code, stderr, _, names) = interrupted_run(&server, path, &[url.as_str()]);
            assert_interrupted(code, &stderr);
            // Upstream opens the part file once data arrives.
            assert!(names.is_empty(), "{path}: {names:?}");
        }
    }

    #[test]
    fn a_signal_ends_a_stalled_body_and_keeps_the_part_file() {
        let server = Server::new();
        let url = server.url("/stalled.mp4");
        let (code, stderr, _, names) = interrupted_run(&server, "/stalled.mp4", &[url.as_str()]);
        assert_interrupted(code, &stderr);
        assert_eq!(names, ["stalled.mp4.part"]);
    }

    #[test]
    fn keep_partial_on_stop_publishes_a_stalled_body_at_once() {
        let server = Server::new();
        let url = server.url("/stalled.mp4");
        let (code, stderr, stdout, names) = interrupted_run(
            &server,
            "/stalled.mp4",
            &[
                "--keep-partial-on-stop",
                "--print",
                "after_move:filepath",
                url.as_str(),
            ],
        );
        assert_interrupted(code, &stderr);
        assert_eq!(names, ["stalled.mp4"]);
        assert!(stdout.trim_end().ends_with("stalled.mp4"), "{stdout}");
    }

    #[test]
    fn a_signal_ends_stalled_concurrent_fragments() {
        let server = Server::new();
        let url = server.url("/index.m3u8");
        let (code, stderr, _, _) = interrupted_run(
            &server,
            "/s2.ts",
            &["-N", "3", "--hls-prefer-native", url.as_str()],
        );
        assert_interrupted(code, &stderr);
    }

    #[test]
    fn a_signal_ends_a_sleep_between_downloads() {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let first = server.url("/s0.ts");
        let mut child = spawn(
            directory.path(),
            &["--sleep-interval", "60", first.as_str()],
        );
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        while !line.contains("Sleeping") {
            line.clear();
            assert!(
                stdout.read_line(&mut line).unwrap() > 0,
                "no sleep announced"
            );
        }
        let sent = Instant::now();
        support::stop(&child);
        let output = child.wait_with_output().unwrap();
        assert!(sent.elapsed() < BOUND, "{:?}", sent.elapsed());
        assert_interrupted(
            output.status.code(),
            &String::from_utf8_lossy(&output.stderr),
        );
    }
}
