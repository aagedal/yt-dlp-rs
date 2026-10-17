use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use yt_dlp_rs::{Error, YoutubeDl, output::render_template};

fn html_fixture(path: &str) -> Option<&'static str> {
    Some(match path {
        "/video.html" => include_str!("fixtures/html5/video.html"),
        "/audio.html" => include_str!("fixtures/html5/audio.html"),
        "/source.html" => include_str!("fixtures/html5/source.html"),
        "/nested/relative.html" => "<title> Relative  title </title><video src='clip.mp4'></video>",
        "/base.html" => "<base href='/media/'><title>Base</title><video src='clip.mp4'></video>",
        "/og.html" => {
            "<title>Fallback</title><meta property='og:title' content='Chosen &amp; title'><video src='/clip.mp4'></video>"
        }
        "/untitled.html" => "<audio src='/audio'></audio>",
        "/referrer.html" => "<video src='/referrer.mp4'></video>",
        "/changing.html" => "<video src='/changing.mp4'></video>",
        "/source-redirect.html" => "<video src='/media-redirect'></video>",
        "/manifest-source.html" => "<video src='/stream'></video>",
        "/multiple.html" => "<video src='/clip.mp4'></video><audio src='/audio'></audio>",
        "/formats.html" => "<video><source src='/clip.mp4'><source src='/playlist.m3u8'></video>",
        "/src-and-source.html" => "<video src='/clip.mp4'><source src='/audio'></video>",
        "/manifest.html" => "<video src='/playlist.m3u8'></video>",
        "/manifest-hint.html" => "<video src='/clip.mp4' type='application/dash+xml'></video>",
        "/blob.html" => "<video src='blob:https://example.invalid/id'></video>",
        "/source-is-html.html" => "<video src='/page.mp4'></video>",
        "/source-is-partial.html" => "<video src='/partial.mp4'></video>",
        "/source-is-missing.html" => "<video src='/missing'></video>",
        "/empty.html" => "<video src=' '><source src=''></video>",
        "/fake.html" => {
            "<!-- <video src='/clip.mp4'></video> --><script>var x = '<video src=\"/clip.mp4\"></video>';</script><p>&lt;video src='/clip.mp4'&gt;</p>"
        }
        _ => return None,
    })
}

// Local-only fixture: no live sites, Python installation, or external test services.
struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
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
                stream.set_nonblocking(false).unwrap();
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
                recorded.lock().unwrap().push(request.to_string());
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let changing_count = recorded
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|request| request.starts_with("GET /changing.mp4 "))
                    .count();
                let oversized = if path.starts_with("/large") {
                    " ".repeat(2 * 1024 * 1024 + 1)
                } else {
                    String::new()
                };
                let (status, mime, body, extra, length) = match path {
                    "/compressed.html" => {
                        ("200 OK", "text/html", "", "Content-Encoding: gzip\r\n", 0)
                    }
                    "/invalid-utf8.html" => ("200 OK", "text/html", "x", "", 1),
                    "/stream" => ("200 OK", "application/dash+xml", "<MPD/>", "", 6),
                    "/media-redirect" => (
                        "302 Found",
                        "text/plain",
                        "",
                        "Location: /referrer.mp4\r\n",
                        0,
                    ),
                    "/changing.mp4" if changing_count == 1 => {
                        ("200 OK", "video/mp4", "media bytes", "", 11)
                    }
                    "/changing.mp4" => ("200 OK", "text/html", "<html>login</html>", "", 18),
                    "/page-redirect" => (
                        "302 Found",
                        "text/plain",
                        "",
                        "Location: /nested/relative.html\r\n",
                        0,
                    ),
                    "/media/clip.mp4?one=1&two=2" | "/media/clip.mp4" | "/nested/clip.mp4" => {
                        ("200 OK", "video/mp4", "media bytes", "", 11)
                    }
                    "/media/tone.mp3" => ("200 OK", "audio/mpeg", "audio", "", 5),
                    "/referrer.mp4"
                        if request.contains(&format!(
                            "referer: http://{}/referrer.html\r\n",
                            stream.local_addr().unwrap()
                        )) || request.contains(&format!(
                            "referer: http://{}/source-redirect.html\r\n",
                            stream.local_addr().unwrap()
                        )) =>
                    {
                        ("200 OK", "video/mp4", "media bytes", "", 11)
                    }
                    "/large.html" | "/large-stream.html" => (
                        "200 OK",
                        "text/html",
                        oversized.as_str(),
                        "",
                        oversized.len(),
                    ),
                    "/latin.html" => (
                        "200 OK",
                        "text/html; charset=iso-8859-1",
                        "<video src='/clip.mp4'></video>",
                        "",
                        30,
                    ),
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
                    _ => match html_fixture(path) {
                        Some(body) => ("200 OK", "text/html; charset=utf-8", body, "", body.len()),
                        None => ("404 Not Found", "text/plain", "missing", "", 7),
                    },
                };
                let content_length = if path == "/large-stream.html" {
                    String::new()
                } else {
                    format!("Content-Length: {length}\r\n")
                };
                let mut response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{content_length}{extra}Connection: close\r\n\r\n{body}"
                ).into_bytes();
                if path == "/invalid-utf8.html" {
                    *response.last_mut().unwrap() = 0xff;
                }
                // Extraction intentionally closes a response after inspecting headers.
                let _ = stream.write_all(&response);
            }
        });
        Self {
            base,
            stop,
            worker: Some(worker),
            requests,
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
fn extracts_saved_html5_fixtures_and_downloads_media() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    for (page, title, ext, source, body) in [
        (
            "video",
            "A & B",
            "mp4",
            "/media/clip.mp4?one=1&two=2",
            b"media bytes".as_slice(),
        ),
        (
            "audio",
            "Audio fixture",
            "mp3",
            "/media/tone.mp3",
            b"audio".as_slice(),
        ),
        (
            "source",
            "Source fixture",
            "mp4",
            "/media/clip.mp4",
            b"media bytes".as_slice(),
        ),
    ] {
        let page_url = server.url(&format!("/{page}.html"));
        let info = dl.extract_info(&page_url).unwrap();
        assert_eq!(info.id, format!("{page}-1"));
        assert_eq!(info.title, format!("{title} (1)"));
        assert_eq!(info.ext, ext);
        assert_eq!(info.url.as_str(), server.url(source));
        assert_eq!(info.webpage_url.as_str(), page_url);
        assert_eq!(info.filesize, Some(body.len() as u64));
        assert!(!info.direct);
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join(format!("media.{ext}"));
        dl.download(&info, &destination).unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), body);
    }
}

#[test]
fn html5_resolves_redirects_base_urls_and_titles() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let redirected = dl.extract_info(&server.url("/page-redirect")).unwrap();
    assert_eq!(redirected.url.as_str(), server.url("/nested/clip.mp4"));
    assert_eq!(
        redirected.webpage_url.as_str(),
        server.url("/nested/relative.html")
    );
    assert_eq!(redirected.id, "relative-1");
    assert_eq!(redirected.title, "Relative title (1)");
    let base = dl.extract_info(&server.url("/base.html")).unwrap();
    assert_eq!(base.url.as_str(), server.url("/media/clip.mp4"));
    assert_eq!(
        dl.extract_info(&server.url("/og.html")).unwrap().title,
        "Chosen & title (1)"
    );
    assert_eq!(
        dl.extract_info(&server.url("/untitled.html"))
            .unwrap()
            .title,
        "untitled (1)"
    );
}

#[test]
fn html5_preserves_referrer_for_probe_and_download() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let info = dl.extract_info(&server.url("/referrer.html")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        dl.download(&info, directory.path().join("media.mp4"))
            .unwrap(),
        11
    );
    let redirected = dl
        .extract_info(&server.url("/source-redirect.html"))
        .unwrap();
    assert_eq!(redirected.url.as_str(), server.url("/referrer.mp4"));
    assert_eq!(
        redirected.webpage_url.as_str(),
        server.url("/source-redirect.html")
    );
    assert_eq!(
        dl.download(&redirected, directory.path().join("redirect.mp4"))
            .unwrap(),
        11
    );
}

#[test]
fn html5_source_changing_to_html_cannot_be_published_as_media() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    let info = dl.extract_info(&server.url("/changing.html")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        dl.download(&info, directory.path().join("media.mp4")),
        Err(Error::UnsupportedHtml5(_))
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn html5_rejects_unsupported_sources_and_ambiguous_selection() {
    let server = Server::start();
    let dl = YoutubeDl::new().unwrap();
    for path in [
        "/multiple.html",
        "/formats.html",
        "/src-and-source.html",
        "/manifest.html",
        "/manifest-hint.html",
        "/source-is-html.html",
        "/latin.html",
        "/compressed.html",
        "/invalid-utf8.html",
        "/manifest-source.html",
    ] {
        assert!(
            matches!(
                dl.extract_info(&server.url(path)),
                Err(Error::UnsupportedHtml5(_))
            ),
            "{path}"
        );
    }
    for path in ["/empty.html", "/fake.html"] {
        assert!(
            matches!(
                dl.extract_info(&server.url(path)),
                Err(Error::UnsupportedUrl(_))
            ),
            "{path}"
        );
    }
    assert!(matches!(
        dl.extract_info(&server.url("/blob.html")),
        Err(Error::UnsupportedScheme(_))
    ));
    assert!(matches!(
        dl.extract_info(&server.url("/source-is-partial.html")),
        Err(Error::PartialResponse)
    ));
    assert!(matches!(
        dl.extract_info(&server.url("/source-is-missing.html")),
        Err(Error::Http(_))
    ));
    for path in ["/large.html", "/large-stream.html"] {
        let result = dl.extract_info(&server.url(path));
        assert!(
            matches!(result, Err(Error::HtmlTooLarge(_))),
            "{path}: {result:?}"
        );
    }
}

#[test]
fn direct_media_uses_one_extraction_request() {
    let server = Server::start();
    YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/clip.mp4"))
        .unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[test]
fn cli_html5_json_and_simulation_do_not_download() {
    let server = Server::start();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("video.mp4");
    for flag in ["--dump-json", "--simulate"] {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args([
                flag,
                "-o",
                target.to_str().unwrap(),
                &server.url("/video.html"),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!target.exists());
        if flag == "--dump-json" {
            let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(info["direct"], false);
            assert_eq!(info["title"], "A & B (1)");
        }
    }
    assert_eq!(server.requests.lock().unwrap().len(), 4);
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
