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

/// MPD with a low-resolution muxed fallback beside the separate streams.
const MUXED_MPD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
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
<AdaptationSet mimeType="video/mp4" codecs="avc1.42001e,mp4a.40.2" width="320" height="180">
<Representation id="muxed" bandwidth="300000"><SegmentList timescale="1" duration="1">
<Initialization sourceURL="muxed/init.mp4"/><SegmentURL media="muxed/0.m4s"/>
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
                    "/muxed.mpd" => (MUXED_MPD.to_owned(), "application/dash+xml", ""),
                    "/redirect" => (String::new(), "text/plain", "Location: /nested/manifest.mpd\r\n"),
                    "/renamed.mpd" => (String::new(), "text/plain", "Location: /hinted\r\n"),
                    "/hinted" => (MPD.into(), "video/mp4", ""),
                    "/plain.mpd" => (MPD.into(), "text/plain", ""),
                    "/html.mpd" => (MPD.into(), "text/html", ""),
                    "/misleading.mpd" => (MPD.into(), "video/mp4", ""),
                    "/mime-only" => (MPD.into(), "application/dash+xml", ""),
                    "/page.html" => ("<title>DASH page</title><video><source src='/manifest.mpd'><source src='/second.mpd'></video>".into(), "text/html", ""),
                    "/gzip.mpd" => (MPD.into(), "application/dash+xml", "Content-Encoding: gzip\r\n"),
                    "/gzip.html" => ("<video src='/gzip.mpd' type='application/dash+xml'></video>".into(), "text/html", ""),
                    "/hinted.html" => ("<video src='/hinted' type='application/dash+xml'></video>".into(), "text/html", ""),
                    "/bad-hint.html" => ("<video><source src='/bad-media' type='application/dash+xml'></video>".into(), "text/html", ""),
                    "/bad-media" | "/malformed.mpd" => ("ordinary media bytes".into(), "video/mp4", ""),
                    "/unknown-type.mpd" => (MPD.replace("type=\"static\"", "type=\"event\""), "application/dash+xml", ""),
                    "/drm.mpd" => (MPD.replace("<Representation id=\"video\"", "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\"/><Representation id=\"video\""), "application/dash+xml", ""),
                    "/drm-all.mpd" => (MPD.replace("<Representation ", "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\"/><Representation "), "application/dash+xml", ""),
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
                } else if path == "/gzip.mpd" {
                    let mut encoder =
                        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                    encoder.write_all(body.as_bytes()).unwrap();
                    encoder.finish().unwrap()
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
            // Probes of the input URL may be media and ask for `identity`;
            // pages and manifests negotiate compression like upstream.
            let request = request.to_ascii_lowercase();
            assert!(
                request.contains("accept-encoding: identity\r\n")
                    || request.contains("accept-encoding: gzip, deflate\r\n"),
                "{request}"
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
    // Upstream names DASH formats by representation ID.
    assert_eq!(info.format_id.as_deref(), Some("video+audio"));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    assert_eq!(info.id, "manifest");
    assert_eq!(info.title, "manifest");
    assert!(!info.is_direct());
    assert_eq!(info.formats.len(), 2);
    assert_eq!(info.requested_formats[0].stream_kind, StreamKind::VideoOnly);
    assert_eq!(info.requested_formats[1].stream_kind, StreamKind::AudioOnly);
    assert_eq!(info.requested_formats[1].ext, "m4a");
    assert_eq!(info.requested_formats[0].fragments.len(), 3);
    // Without a merger, upstream's `best/bestvideo+bestaudio` still selects
    // the pair (downloads then keep the format files unmerged).
    let unmerged = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/manifest.mpd"))
        .unwrap();
    assert_eq!(unmerged.format_id, info.format_id);
    let video = dl
        .extract_info_with_format(&server.url("/redirect"), Some("bestvideo"))
        .unwrap();
    assert_eq!(video.format_id.as_deref(), Some("video"));
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
        // Pinned upstream sorts and selects these IDs for the same page.
        ["audio-0", "audio-1", "video-0", "video-1"]
    );
    assert_eq!(info.format_id.as_deref(), Some("video-1+audio-1"));
    let group = |id: &str| {
        info.formats
            .iter()
            .find(|format| format.format_id == id)
            .unwrap()
            .audio_group
            .clone()
    };
    assert_ne!(group("video-0"), group("video-1"));
    assert_eq!(group("video-0"), group("audio-0"));
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
fn page_manifests_are_requested_and_decoded_compressed() {
    let server = Server::new();
    let info = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg("unused-explicit-ffmpeg")
        .extract_info(&server.url("/gzip.html"))
        .unwrap();
    assert_eq!(info.format_id.as_deref(), Some("video+audio"));
    let requests = server.requests.lock().unwrap();
    let encoding = |path: &str| {
        let request = requests
            .iter()
            .find(|request| request.starts_with(&format!("GET {path} ")))
            .unwrap()
            .to_ascii_lowercase();
        request
            .lines()
            .find_map(|line| line.strip_prefix("accept-encoding: ").map(str::to_owned))
    };
    // The input URL could be media; the manifest it names is not.
    assert_eq!(encoding("/gzip.html").as_deref(), Some("identity"));
    assert_eq!(encoding("/gzip.mpd").as_deref(), Some("gzip, deflate"));
    drop(requests);
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
        assert!(!info.is_direct(), "{path}");
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
    // An existing placeholder enables merging; simulation never runs it.
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = tools.path().join("ffmpeg");
    std::fs::write(&ffmpeg, "").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--dump-json",
            "--simulate",
            "--ffmpeg-location",
            ffmpeg.to_str().unwrap(),
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
    assert_eq!(info["protocol"], "http_dash_segments+http_dash_segments");
    // Upstream parse_codecs fields; merges take video and audio codecs.
    assert_eq!(
        (&info["vcodec"], &info["acodec"], &info["dynamic_range"]),
        (
            &serde_json::json!("avc1.42001e"),
            &serde_json::json!("mp4a.40.2"),
            &serde_json::json!("SDR")
        )
    );
    assert_eq!(info["requested_formats"][1]["vcodec"], "none");
    assert_eq!(info["requested_formats"][0]["format_note"], "DASH video");
    assert_eq!(info["requested_formats"][1]["format_note"], "DASH audio");
    assert_eq!(info["requested_formats"][1]["audio_ext"], "m4a");
    assert_eq!(info["requested_formats"][1]["video_ext"], "none");
    // `parse_codecs` gives audio a `None` dynamic range, as upstream prints.
    assert!(info["requested_formats"][1]["dynamic_range"].is_null());
    assert_eq!(info["requested_formats"][0]["acodec"], "none");
    assert_eq!(
        info["requested_formats"][1]["protocol"],
        "http_dash_segments"
    );
    assert_eq!(
        info["requested_formats"][0]["fragments"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    // `_merge`'s dict has no URL, so templates see none, as upstream.
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "-O",
            "%(url)s|%(format_id)s",
            "--ffmpeg-location",
            ffmpeg.to_str().unwrap(),
            &server.url("/manifest.mpd"),
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "NA|video+audio\n");
}

#[test]
fn unsupported_dash_and_invalid_response_metadata_fail_before_publication() {
    let server = Server::new();
    for path in [
        // Neither `static` nor `dynamic` (dynamic MPDs: tests/dash_dynamic.rs).
        "/unknown-type.mpd",
        // The second Period ends after the presentation.
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
    server.assert_no_fragments();
}

/// Upstream `has_drm` for an AdaptationSet or Representation with a
/// ContentProtection child: such formats are dropped unless
/// `--allow-unplayable-formats`. Outputs match pinned upstream for the same
/// manifests.
#[test]
fn protected_dash_formats_are_dropped_like_upstream() {
    let server = Server::new();
    let info = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/drm.mpd"))
        .unwrap();
    let ids: Vec<_> = info.formats.iter().map(|f| f.format_id.as_str()).collect();
    assert_eq!(ids, ["audio"]);
    assert_eq!(info.format_id.as_deref(), Some("audio"));
    let directory = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .arg("--ignore-config")
            .args(args)
            .arg(server.url("/drm-all.mpd"))
            .current_dir(directory.path())
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    let (code, stdout, stderr) = run(&[]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stdout,
        format!(
            "[generic] Extracting URL: {}\n[generic] drm-all: Downloading webpage\n\
             [generic] drm-all: Extracting information\n",
            server.url("/drm-all.mpd")
        )
    );
    assert_eq!(
        stderr,
        "WARNING: [generic] Falling back on generic information extractor\n\
         ERROR: [generic] drm-all: This video is DRM protected\n"
    );
    let (code, _, stderr) = run(&["-s", "--ignore-no-formats-error"]);
    assert_eq!(code, Some(0));
    assert!(
        stderr.ends_with(
            "WARNING: This video is DRM protected\nWARNING: Requested format is not available\n"
        ),
        "{stderr}"
    );
    let (code, stdout, _) = run(&[
        "--allow-unplayable-formats",
        "--print",
        "format_id,formats.0.has_drm",
    ]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "video+audio\nTrue\n");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    server.assert_no_fragments();
}

#[cfg(unix)]
#[test]
fn cli_ffmpeg_location_follows_upstream_determine_executables() {
    use std::os::unix::fs::PermissionsExt;
    let server = Server::new();
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = tools.path().join("ffmpeg");
    std::fs::write(&ffmpeg, "").unwrap();
    std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    let empty = tempfile::tempdir().unwrap();
    let run = |path: &std::path::Path, args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .env("PATH", path)
            .args(args)
            .args(["-j", &server.url("/muxed.mpd")])
            .output()
            .unwrap();
        let info = serde_json::from_slice::<serde_json::Value>(&output.stdout)
            .map(|info| info["format_id"].clone())
            .ok();
        (info, String::from_utf8_lossy(&output.stderr).into_owned())
    };
    // Checked with pinned upstream: with ffmpeg the default merges the
    // separate streams; without it `best/bestvideo+bestaudio` picks the muxed one.
    let merged = Some(serde_json::json!("video+audio"));
    // Without a location, ffmpeg comes from PATH; simulation never runs it.
    assert_eq!(run(tools.path(), &[]).0, merged);
    let (selected, _) = run(empty.path(), &[]);
    assert_ne!(selected, merged);
    // A directory holds the programs.
    let directory = tools.path().to_str().unwrap();
    assert_eq!(
        run(empty.path(), &["--ffmpeg-location", directory]).0,
        merged
    );
    // A missing location warns and continues without ffmpeg.
    let missing = tools.path().join("missing");
    let (selected, stderr) = run(
        tools.path(),
        &["--ffmpeg-location", missing.to_str().unwrap()],
    );
    assert_ne!(selected, merged);
    assert!(
        stderr.contains(&format!(
            "WARNING: ffmpeg-location {} does not exist! Continuing without ffmpeg",
            missing.display()
        )),
        "{stderr}"
    );
}
