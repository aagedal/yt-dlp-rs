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
use yt_dlp_rs::{Error, YoutubeDl};

const MEDIA: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nsegments/first.ts\n#EXTINF:2,\nsegments/second.ts\n#EXT-X-ENDLIST\n";
const MASTER: &str = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=400000,RESOLUTION=640x360,CODECS=\"avc1.42001e,mp4a.40.2\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=1280x720,CODECS=\"avc1.64001f,mp4a.40.2\"\nhigh.m3u8\n";

struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
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
                let mut bytes = Vec::new();
                let mut buffer = [0; 2048];
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    let length = stream.read(&mut buffer).unwrap();
                    if length == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..length]);
                }
                let request = String::from_utf8(bytes).unwrap();
                let path = request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let large = if path == "/many-child.m3u8" {
                    format!(
                        "#EXTM3U\n#EXT-X-TARGETDURATION:1\n{}#EXT-X-ENDLIST\n",
                        "#EXTINF:1,\nsegment.ts\n".repeat(400_000)
                    )
                } else {
                    String::new()
                };
                let (body, mime, extra) = match path {
                    "/master.m3u8" => (MASTER, "application/vnd.apple.mpegurl", ""),
                    "/media.m3u8" | "/low.m3u8" | "/high.m3u8" => (MEDIA, "text/plain", ""),
                    "/stream" => (MEDIA, "application/x-mpegurl", ""),
                    "/page.html" => (
                        "<title>Page title</title><video><source src='/master.m3u8' type='application/x-mpegurl'><source src='/clip.mp4'></video>",
                        "text/html",
                        "",
                    ),
                    "/single.html" => ("<audio src='/media.m3u8'></audio>", "text/html", ""),
                    "/clip.mp4" => ("clip", "video/mp4", ""),
                    "/live.m3u8" => (
                        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nsegment.ts\n",
                        "application/x-mpegurl",
                        "",
                    ),
                    "/aes.m3u8" => (
                        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:7\n#EXT-X-KEY:METHOD=AES-128,URI=\"key\"\n#EXTINF:4,\nsegment.ts\n#EXT-X-ENDLIST\n",
                        "application/x-mpegurl",
                        "",
                    ),
                    "/encrypted.m3u8" => (
                        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"key\"\n#EXTINF:4,\nsegment.ts\n#EXT-X-ENDLIST\n",
                        "application/x-mpegurl",
                        "",
                    ),
                    "/fairplay.m3u8" => (
                        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXTINF:4,\nsegment.ts\n#EXT-X-ENDLIST\n",
                        "application/x-mpegurl",
                        "",
                    ),
                    "/tagged.m3u8" => (
                        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-START:TIME-OFFSET=0\n#EXT-X-DATERANGE:ID=\"a\",START-DATE=\"2026-01-01T00:00:00Z\"\n#EXT-X-CUE-OUT:4\n#EXTINF:4,\nsegments/first.ts\n#EXT-X-CUE-IN\n#EXTINF:2,\nsegments/second.ts\n#EXT-X-ENDLIST\n",
                        "application/x-mpegurl",
                        "",
                    ),
                    "/angles.m3u8" => (
                        "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"en\",URI=\"media.m3u8\"\n#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"cam\",NAME=\"Angle 2\",URI=\"low.m3u8\"\n#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"cam\",NAME=\"Main\"\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,VIDEO=\"cam\",AUDIO=\"aud\",CLOSED-CAPTIONS=\"cc\",CODECS=\"avc1.4d401f,mp4a.40.2\"\nhigh.m3u8\n",
                        "application/x-mpegurl",
                        "",
                    ),
                    "/malformed.m3u8" => ("not a playlist", "text/plain", ""),
                    "/nested.m3u8" => (
                        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=400000\nmaster.m3u8\n",
                        "application/x-mpegurl",
                        "",
                    ),
                    "/compressed.m3u8" => {
                        (MEDIA, "application/x-mpegurl", "Content-Encoding: gzip\r\n")
                    }
                    "/charset.m3u8" => (MEDIA, "application/x-mpegurl; charset=iso-8859-1", ""),
                    "/fake.m3u8" => (MEDIA, "video/mp4", ""),
                    "/fake-html.m3u8" => (MEDIA, "text/html", ""),
                    "/bad-mime.m3u8" => ("media bytes", "video/mp4", ""),
                    "/hinted.html" => (
                        "<video src='/hinted' type='application/x-mpegurl'></video>",
                        "text/html",
                        "",
                    ),
                    "/hinted" => (MEDIA, "application/octet-stream", ""),
                    "/hinted-media.html" => (
                        "<video><source src='/hinted-media' type='application/vnd.apple.mpegurl'></video>",
                        "text/html",
                        "",
                    ),
                    "/hinted-media" => (MEDIA, "video/mp4", ""),
                    "/hinted-bad.html" => (
                        "<video src='/clip.mp4' type='application/x-mpegurl'></video>",
                        "text/html",
                        "",
                    ),
                    "/duplicate-type.m3u8" => (
                        MEDIA,
                        "application/x-mpegurl",
                        "Content-Type: application/x-mpegurl\r\n",
                    ),
                    "/duplicate-encoding.m3u8" => (
                        MEDIA,
                        "application/x-mpegurl",
                        "Content-Encoding: identity\r\nContent-Encoding: identity\r\n",
                    ),
                    "/hidden-encoding.m3u8" => (
                        MEDIA,
                        "application/x-mpegurl",
                        "Content-Encoding: identity\r\nContent-Encoding: gzip\r\n",
                    ),
                    "/invalid-type.m3u8" => (MEDIA, "application/x-mpegurl; x=\u{0080}", ""),
                    "/many-master.m3u8" => (
                        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=400000\nmany-child.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nmany-child.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1600000\nmany-child.m3u8\n",
                        "application/x-mpegurl",
                        "",
                    ),
                    "/many-child.m3u8" => (large.as_str(), "application/x-mpegurl", ""),
                    _ => ("unexpected segment request", "text/plain", ""),
                };
                record.lock().unwrap().push(request.clone());
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(body.as_bytes());
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
    fn paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| {
                request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn direct_media_and_master_extract_plans_without_fetching_segments() {
    let server = Server::new();
    let downloader = YoutubeDl::new().unwrap();
    for path in ["/media.m3u8", "/stream"] {
        let info = downloader.extract_info(&server.url(path)).unwrap();
        assert!(!info.is_direct());
        // Upstream labels HLS formats `mp4` regardless of segment containers.
        assert_eq!(info.ext, "mp4");
        // Upstream returns a media playlist as one format, numbered `0`.
        assert_eq!(info.formats.len(), 1);
        assert_eq!(info.format_id.as_deref(), Some("0"));
        assert_eq!(info.fragments.len(), 2);
        assert_eq!(
            info.fragments[0].url.as_str(),
            server.url("/segments/first.ts")
        );
        assert_eq!(info.webpage_url.as_str(), server.url(path));
    }
    let best = downloader
        .extract_info(&server.url("/master.m3u8"))
        .unwrap();
    // Upstream names unnamed variants by their bitrate in kbps.
    assert_eq!(best.format_id.as_deref(), Some("1000"));
    assert_eq!((best.width, best.height), (Some(1280), Some(720)));
    assert_eq!(best.formats[1].tbr, Some(1000.0));
    assert_eq!(best.fragments.len(), 2);
    let low = downloader
        .extract_info_with_format(&server.url("/master.m3u8"), Some("400"))
        .unwrap();
    assert_eq!(low.height, Some(360));
    assert_eq!(low.url.as_str(), server.url("/low.m3u8"));
    assert_eq!(low.fragments.len(), 2);
    assert!(
        server
            .paths()
            .iter()
            .all(|path| !path.contains("segments/"))
    );
}

#[test]
fn html_sources_flatten_master_formats_and_preserve_page_identity_and_referer() {
    let server = Server::new();
    let info = YoutubeDl::new()
        .unwrap()
        .extract_info_with_format(&server.url("/page.html"), Some("hls-1000"))
        .unwrap();
    assert_eq!(info.id, "page-1");
    assert_eq!(info.title, "Page title (1)");
    assert_eq!(info.extractor, "html5");
    assert_eq!(info.webpage_url.as_str(), server.url("/page.html"));
    assert_eq!(
        info.formats
            .iter()
            .map(|format| format.format_id.as_str())
            .collect::<Vec<_>>(),
        // Pinned upstream: the unlabeled MP4 source sorts first, numbered `0`.
        ["0", "hls-400", "hls-1000"]
    );
    assert_eq!(info.fragments.len(), 2);
    for request in server.requests.lock().unwrap().iter().skip(1) {
        assert!(
            request
                .to_ascii_lowercase()
                .contains(&format!("referer: {}\r\n", server.url("/page.html")))
        );
    }
    let single = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/single.html"))
        .unwrap();
    assert_eq!(single.format_id.as_deref(), Some("hls"));
    assert_eq!(single.fragments.len(), 2);
}

#[test]
fn json_simulation_does_not_fetch_fragments_or_create_output() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--dump-json", "--simulate", &server.url("/master.m3u8")])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["format_id"], "1000");
    assert_eq!(json["protocol"], "m3u8_native");
    assert!(
        json["formats"]
            .as_array()
            .unwrap()
            .iter()
            .all(|format| format["protocol"] == "m3u8_native")
    );
    // Like upstream, HLS formats carry no fragments; HlsFD reads the playlist.
    assert!(json.get("fragments").is_none());
    assert_eq!(json["has_drm"], false);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    // Upstream generic's "Checking m3u8 live status" refetches the first
    // `m3u8_native` format and sets duration = int(sum of EXTINF) = 4 + 2.
    assert_eq!(json["duration"], 6);
    assert_eq!(json["duration_string"], "6");
    assert_eq!(
        server.paths(),
        ["/master.m3u8", "/low.m3u8", "/high.m3u8", "/low.m3u8"]
    );
}

#[test]
fn unsupported_manifests_fail_before_any_output_or_fragment_request() {
    let server = Server::new();
    for path in [
        "/malformed.m3u8",
        "/nested.m3u8",
        "/compressed.m3u8",
        "/charset.m3u8",
    ] {
        assert!(
            matches!(
                YoutubeDl::new().unwrap().extract_info(&server.url(path)),
                Err(Error::UnsupportedManifest(_))
            ),
            "{path}"
        );
    }
    assert!(
        server
            .paths()
            .iter()
            .all(|path| !path.contains("segment.ts") && !path.contains("key"))
    );
    let directory = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg(server.url("/malformed.m3u8"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

/// Upstream `extract_media`: a VIDEO rendition with a URI is a format with
/// unknown codecs, listed with the audio renditions before the variants.
#[test]
fn external_video_renditions_become_formats() {
    let server = Server::new();
    let info = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/angles.m3u8"))
        .unwrap();
    let formats: Vec<_> = info
        .formats
        .iter()
        .map(|f| (f.format_id.as_str(), f.vcodec.as_deref(), f.fragments.len()))
        .collect();
    assert_eq!(
        formats,
        [
            ("aud-en", Some("none"), 2),
            ("cam-Angle_2", None, 2),
            ("Angle_2", Some("avc1.4d401f"), 2),
        ]
    );
    assert_eq!(info.formats[1].format_note.as_deref(), Some("Angle 2"));
    assert!(info.formats[1].audio_group.is_none());
}

/// Upstream ignores tags it does not interpret (ad markers, date ranges,
/// start offsets, ...) instead of rejecting the playlist.
#[test]
fn unknown_tags_are_ignored_like_upstream() {
    let server = Server::new();
    let info = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/tagged.m3u8"))
        .unwrap();
    let urls: Vec<_> = info.fragments.iter().map(|f| f.url.path()).collect();
    assert_eq!(urls, ["/segments/first.ts", "/segments/second.ts"]);
}

/// Upstream `HlsFD.can_download`: a `SAMPLE-AES` playlist is extracted, then
/// its download is delegated to FFmpegFD (which needs ffmpeg); a DRM one
/// is marked `has_drm` and fails like `_has_drm`.
#[cfg(unix)]
#[test]
fn unsupported_keys_delegate_to_ffmpeg_and_drm_fails() {
    use std::os::unix::fs::PermissionsExt;
    let server = Server::new();
    let info = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/encrypted.m3u8"))
        .unwrap();
    assert!(info.fragments.is_empty());
    assert_eq!(info.formats[0].extra["has_drm"], false);

    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = tools.path().join("ffmpeg");
    let log = tools.path().join("args");
    std::fs::write(
        &ffmpeg,
        format!(
            "#!/bin/sh\n\
             case \"$1\" in -version) echo 'ffmpeg version fixture'; exit 0;; esac\n\
             echo \"$@\" >> '{}'\n\
             for last; do :; done\n\
             printf media > \"${{last#file:}}\"\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ffmpeg-location", &ffmpeg.display().to_string()])
        .args(["-o", "out.%(ext)s", &server.url("/encrypted.m3u8")])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");
    assert!(
        stderr.contains(
            "WARNING: Unsupported features have been detected; extraction will be delegated to ffmpeg"
        ),
        "{stderr}"
    );
    let args = std::fs::read_to_string(&log).unwrap();
    assert!(args.contains(&server.url("/encrypted.m3u8")), "{args}");
    assert!(directory.path().join("out.mp4").exists());

    let directory = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ffmpeg-location", "/nonexistent/ffmpeg"])
        .arg(server.url("/encrypted.m3u8"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success());
    assert!(
        stderr.contains("m3u8 download detected but ffmpeg could not be found"),
        "{stderr}"
    );

    let result = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ffmpeg-location", &ffmpeg.display().to_string()])
        .arg(server.url("/fairplay.m3u8"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success());
    assert!(
        stderr.contains("ERROR: [generic] fairplay: This video is DRM protected"),
        "{stderr}"
    );
    assert!(server.paths().iter().all(|path| !path.contains("segment")));
}

#[test]
fn manifest_path_and_html_type_hints_override_misleading_media_mime() {
    let server = Server::new();
    for path in [
        "/fake.m3u8",
        "/fake-html.m3u8",
        "/hinted.html",
        "/hinted-media.html",
    ] {
        let info = YoutubeDl::new()
            .unwrap()
            .extract_info(&server.url(path))
            .unwrap();
        assert!(!info.is_direct(), "{path}");
        assert_eq!(info.fragments.len(), 2, "{path}");
    }
    for path in ["/bad-mime.m3u8", "/hinted-bad.html"] {
        assert!(
            matches!(
                YoutubeDl::new().unwrap().extract_info(&server.url(path)),
                Err(Error::UnsupportedManifest(_))
            ),
            "{path}"
        );
    }
    assert!(
        server
            .paths()
            .iter()
            .all(|path| !path.contains("segments/"))
    );
}

#[test]
fn ambiguous_manifest_headers_are_rejected() {
    let server = Server::new();
    for path in [
        "/duplicate-type.m3u8",
        "/duplicate-encoding.m3u8",
        "/hidden-encoding.m3u8",
        "/invalid-type.m3u8",
    ] {
        assert!(
            matches!(
                YoutubeDl::new().unwrap().extract_info(&server.url(path)),
                Err(Error::UnsupportedManifest(_))
            ),
            "{path}"
        );
    }
}

#[test]
fn master_retained_fragment_count_is_bounded_across_variants() {
    let server = Server::new();
    let error = YoutubeDl::new()
        .unwrap()
        .extract_info(&server.url("/many-master.m3u8"))
        .unwrap_err();
    assert!(
        matches!(error, Error::UnsupportedManifest(message) if message.contains("1000000-fragment"))
    );
    assert_eq!(
        server.paths(),
        [
            "/many-master.m3u8",
            "/many-child.m3u8",
            "/many-child.m3u8",
            "/many-child.m3u8"
        ]
    );
}

#[test]
fn encrypted_json_and_simulation_do_not_request_keys_or_segments() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    for flag in ["-j", "-s"] {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args([flag, &server.url("/aes.m3u8")])
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if flag == "-j" {
            // The key location stays in the playlist, as upstream.
            let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(info.get("fragments").is_none());
            assert!(!String::from_utf8_lossy(&output.stdout).contains("key_url"));
        }
    }
    // Per run: extraction plus the generic live-status refetch of the media
    // playlist itself. Keys and segments are never requested.
    assert_eq!(
        server.paths(),
        ["/aes.m3u8", "/aes.m3u8", "/aes.m3u8", "/aes.m3u8"]
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn cli_download_reports_upstream_fragment_count_before_destination() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--no-progress",
            "-o",
            "out.%(ext)s",
            &server.url("/master.m3u8"),
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    // This server's segments are not media; the lines precede the transfer.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[hlsnative] Total fragments: 2\n[download] Destination: out."),
        "{stdout}"
    );
}

#[test]
fn info_json_formats_have_no_fragments_and_reload_from_the_playlist() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--no-progress", "-o", "out.%(ext)s"])
            .args(args)
            .current_dir(directory.path())
            .output()
            .unwrap()
    };
    let output = cli(&[
        "--skip-download",
        "--write-info-json",
        &server.url("/master.m3u8"),
    ]);
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("out.info.json")).unwrap())
            .unwrap();
    // Upstream's HLS format dicts: no fragments, the master as manifest_url.
    for format in json["formats"].as_array().unwrap() {
        assert!(format.get("fragments").is_none());
        assert_eq!(format["manifest_url"], server.url("/master.m3u8"));
        assert_eq!(format["has_drm"], false);
    }
    // HlsFD reads the fragments from the playlist when downloading.
    let output = cli(&["--load-info-json", "out.info.json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "[hlsnative] Downloading m3u8 manifest\n[hlsnative] Total fragments: 2\n[download] Destination: out.mp4"
        ),
        "{stdout}"
    );
}

/// Upstream `_parse_m3u8_formats_and_subtitles` returns a media playlist
/// without `#EXT-X-ENDLIST` as one format without reading its segments, and
/// generic's live-status check marks it `is_live`.
#[test]
fn live_media_playlist_extracts_as_one_live_format() {
    let server = Server::new();
    let result = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["-j", &server.url("/live.m3u8")])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let formats = json["formats"].as_array().unwrap();
    assert_eq!(formats.len(), 1);
    assert_eq!(formats[0]["url"], server.url("/live.m3u8"));
    assert_eq!(formats[0]["protocol"], "m3u8_native");
    assert_eq!(json["live_status"], "is_live");
    assert_eq!(json["is_live"], true);
    assert!(json.get("fragments").is_none());
    assert!(server.paths().iter().all(|path| path == "/live.m3u8"));
}

/// Upstream `_get_suitable_downloader`: live HLS downloads through FFmpegFD.
#[cfg(unix)]
#[test]
fn live_media_playlist_downloads_through_ffmpeg() {
    use std::os::unix::fs::PermissionsExt;
    let server = Server::new();
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = tools.path().join("ffmpeg");
    let log = tools.path().join("args");
    std::fs::write(
        &ffmpeg,
        format!(
            "#!/bin/sh\n\
             case \"$1\" in -version) echo 'ffmpeg version live-fixture'; exit 0;; esac\n\
             echo \"$@\" >> '{}'\n\
             for last; do :; done\n\
             printf live > \"${{last#file:}}\"\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ffmpeg-location", &ffmpeg.display().to_string()])
        .args(["-o", "out.%(ext)s", &server.url("/live.m3u8")])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let args = std::fs::read_to_string(&log).unwrap();
    assert!(args.contains(&server.url("/live.m3u8")), "{args}");
    assert!(
        server.paths().iter().all(|path| !path.contains("segment")),
        "{:?}",
        server.paths()
    );
    assert!(directory.path().join("out.mp4").exists());
    // Without ffmpeg, see `tests/hls_live.rs`.
}
