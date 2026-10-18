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
use yt_dlp_rs::{Error, StreamKind, YoutubeDl};

const MPD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT2S">
<Period duration="PT2S">
<AdaptationSet mimeType="video/mp4" codecs="avc1.42001e" width="640" height="360">
<Representation id="video" bandwidth="400000"><SegmentList timescale="1" duration="1">
<Initialization sourceURL="video/init.mp4"/><SegmentURL media="video/0.m4s"/><SegmentURL media="video/1.m4s"/>
</SegmentList></Representation></AdaptationSet>
<AdaptationSet mimeType="audio/mp4" codecs="mp4a.40.2">
<Representation id="audio" bandwidth="128000"><SegmentList timescale="1" duration="1">
<Initialization sourceURL="audio/init.mp4"/><SegmentURL media="audio/0.m4s"/><SegmentURL media="audio/1.m4s"/>
</SegmentList></Representation></AdaptationSet>
</Period></MPD>"#;

struct Server {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (record, done) = (requests.clone(), stop.clone());
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let length = stream.read(&mut buffer).unwrap();
                    if length == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..length]);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let (body, mime, extra) = match path {
                    "/manifest.mpd" | "/nested/manifest.mpd" | "/second.mpd" => (MPD.to_owned(), "application/dash+xml", ""),
                    "/redirect" => (String::new(), "text/plain", "Location: /nested/manifest.mpd\r\n"),
                    "/renamed.mpd" => (String::new(), "text/plain", "Location: /hinted\r\n"),
                    "/hinted" => (MPD.into(), "video/mp4", ""),
                    "/plain.mpd" => (MPD.into(), "text/plain", ""),
                    "/html.mpd" => (MPD.into(), "text/html", ""),
                    "/misleading.mpd" => (MPD.into(), "video/mp4", ""),
                    "/mime-only" => (MPD.into(), "application/dash+xml", ""),
                    "/page.html" => ("<title>DASH page</title><video><source src='/manifest.mpd'><source src='/second.mpd'></video>".into(), "text/html", ""),
                    "/hinted.html" => ("<video src='/hinted' type='application/dash+xml'></video>".into(), "text/html", ""),
                    "/bad-hint.html" => ("<video><source src='/bad-media' type='application/dash+xml'></video>".into(), "text/html", ""),
                    "/bad-media" | "/malformed.mpd" => ("ordinary media bytes".into(), "video/mp4", ""),
                    "/dynamic.mpd" => (MPD.replace("type=\"static\"", "type=\"dynamic\""), "application/dash+xml", ""),
                    "/drm.mpd" => (MPD.replace("<Representation id=\"video\"", "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\"/><Representation id=\"video\""), "application/dash+xml", ""),
                    "/multi.mpd" => (MPD.replace("</Period></MPD>", "</Period><Period duration=\"PT1S\"/></MPD>"), "application/dash+xml", ""),
                    "/dtd.mpd" => (MPD.replace("<MPD ", "<!DOCTYPE MPD [<!ENTITY injected 'value'>]><MPD "), "application/dash+xml", ""),
                    "/duplicate-type.mpd" => (MPD.into(), "application/dash+xml", "Content-Type: application/dash+xml\r\n"),
                    "/duplicate-encoding.mpd" => (MPD.into(), "application/dash+xml", "Content-Encoding: identity\r\nContent-Encoding: identity\r\n"),
                    "/encoding.mpd" => (MPD.into(), "application/dash+xml", "Content-Encoding: identity\r\nContent-Encoding: gzip\r\n"),
                    "/invalid-type.mpd" => (MPD.into(), "application/dash+xml; x=\u{0080}", ""),
                    "/charset.mpd" => (MPD.into(), "application/dash+xml; charset=utf-16", ""),
                    "/oversized.mpd" | "/utf8.mpd" => (MPD.into(), "application/dash+xml", ""),
                    _ => ("unexpected initialization or media request".into(), "text/plain", ""),
                };
                let body = if path == "/utf8.mpd" {
                    vec![255]
                } else {
                    body.into_bytes()
                };
                let length = if path == "/oversized.mpd" {
                    2 * 1024 * 1024 + 1
                } else {
                    body.len()
                };
                let status = if matches!(path, "/redirect" | "/renamed.mpd") {
                    "302 Found"
                } else {
                    "200 OK"
                };
                record.lock().unwrap().push(request.clone());
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\n{extra}Content-Length: {length}\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(&body);
            }
        });
        Self {
            origin,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
    fn assert_no_fragments(&self) {
        for request in self.requests.lock().unwrap().iter() {
            let path = request.split_whitespace().nth(1).unwrap();
            assert!(
                !path.ends_with(".m4s") && !path.ends_with("init.mp4"),
                "{path}"
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("accept-encoding: identity\r\n")
            );
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn direct_and_redirected_mpd_build_separate_stream_fragment_plans() {
    let server = Server::new();
    let dl = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg("unused-explicit-ffmpeg");
    let info = dl.extract_info(&server.url("/manifest.mpd")).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("dash-0+dash-1"));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    assert_eq!(info.id, "manifest");
    assert_eq!(info.title, "manifest");
    assert!(!info.direct);
    assert_eq!(info.formats.len(), 2);
    assert_eq!(info.requested_formats[0].stream_kind, StreamKind::VideoOnly);
    assert_eq!(info.requested_formats[1].stream_kind, StreamKind::AudioOnly);
    assert_eq!(info.requested_formats[1].ext, "m4a");
    assert_eq!(info.requested_formats[0].fragments.len(), 3);
    assert!(matches!(
        YoutubeDl::new()
            .unwrap()
            .extract_info(&server.url("/manifest.mpd")),
        Err(Error::FormatSelection(_))
    ));
    let video = dl
        .extract_info_with_format(&server.url("/redirect"), Some("bestvideo"))
        .unwrap();
    assert_eq!(video.format_id.as_deref(), Some("dash-0"));
    assert_eq!(video.webpage_url.as_str(), server.url("/redirect"));
    assert_eq!(
        video.fragments[0].url.as_str(),
        server.url("/nested/video/init.mp4")
    );
    assert_eq!(
        video.fragments[1].url.as_str(),
        server.url("/nested/video/0.m4s")
    );
    server.assert_no_fragments();
}

#[test]
fn html_dash_formats_preserve_identity_group_namespace_and_referer() {
    let server = Server::new();
    let info = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg("unused-explicit-ffmpeg")
        .extract_info(&server.url("/page.html"))
        .unwrap();
    assert_eq!(info.title, "DASH page (1)");
    assert_eq!(info.id, "page-1");
    assert_eq!(info.extractor, "html5");
    assert_eq!(
        info.formats
            .iter()
            .map(|format| format.format_id.as_str())
            .collect::<Vec<_>>(),
        ["0-dash-0", "0-dash-1", "1-dash-0", "1-dash-1"]
    );
    assert_ne!(info.formats[0].audio_group, info.formats[2].audio_group);
    assert_eq!(
        info.requested_formats[0].audio_group,
        info.requested_formats[1].audio_group
    );
    for request in server.requests.lock().unwrap().iter().skip(1) {
        assert!(
            request
                .to_ascii_lowercase()
                .contains(&format!("referer: {}\r\n", server.url("/page.html")))
        );
    }
    server.assert_no_fragments();
}

#[test]
fn mpd_paths_mime_and_html_hints_override_conflicting_media_headers() {
    let server = Server::new();
    let dl = YoutubeDl::new().unwrap();
    for path in [
        "/renamed.mpd",
        "/plain.mpd",
        "/html.mpd",
        "/misleading.mpd",
        "/mime-only",
        "/hinted.html",
    ] {
        let info = dl
            .extract_info_with_format(&server.url(path), Some("bestvideo"))
            .unwrap();
        assert_eq!(info.fragments.len(), 3, "{path}");
        assert!(!info.direct, "{path}");
    }
    for path in ["/bad-hint.html", "/malformed.mpd"] {
        assert!(
            matches!(
                dl.extract_info(&server.url(path)),
                Err(Error::UnsupportedManifest(_))
            ),
            "{path}"
        );
    }
    server.assert_no_fragments();
}

#[test]
fn json_simulation_does_not_request_initialization_or_media_or_create_files() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--dump-json",
            "--simulate",
            "--ffmpeg-location",
            "unused-explicit-ffmpeg",
            &server.url("/manifest.mpd"),
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        info["requested_formats"][0]["fragments"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn unsupported_dash_and_invalid_response_metadata_fail_before_publication() {
    let server = Server::new();
    for path in [
        "/dynamic.mpd",
        "/drm.mpd",
        "/multi.mpd",
        "/dtd.mpd",
        "/duplicate-type.mpd",
        "/duplicate-encoding.mpd",
        "/encoding.mpd",
        "/invalid-type.mpd",
        "/charset.mpd",
        "/oversized.mpd",
        "/utf8.mpd",
    ] {
        assert!(
            matches!(
                YoutubeDl::new().unwrap().extract_info(&server.url(path)),
                Err(Error::UnsupportedManifest(_))
            ),
            "{path}"
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg(server.url("/drm.mpd"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    server.assert_no_fragments();
}
