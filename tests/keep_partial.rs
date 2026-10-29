//! `--keep-partial-on-stop` (port extension): SIGTERM during a download
//! stops it and keeps a usable file at the final name (whole fragments; a
//! merged download's streams so far, audio first, merged), prints the
//! `after_move` path and exits with "Interrupted by user". Local servers
//! that trickle the later data so the signal lands mid-transfer.
#![cfg(unix)]
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// How a path is answered: all at once, or in 188-byte packets every 50 ms.
enum Body {
    Whole(Vec<u8>),
    Trickle(Vec<u8>),
}

/// A local server for `routes`, recording requested paths.
fn server(
    routes: impl Fn(&str) -> Option<Body> + Send + Sync + 'static,
) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (record, routes) = (requests.clone(), Arc::new(routes));
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (record, routes) = (record.clone(), routes.clone());
            thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                        break;
                    }
                }
                record.lock().unwrap().push(path.clone());
                let kind = if path.ends_with(".m3u8") {
                    "application/vnd.apple.mpegurl"
                } else if path.ends_with(".ts") {
                    "video/mp2t"
                } else {
                    "video/mp4"
                };
                let (body, trickle) = match routes(&path) {
                    Some(Body::Whole(body)) => (body, false),
                    Some(Body::Trickle(body)) => (body, true),
                    None => {
                        let _ = stream.write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                        return;
                    }
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if stream.write_all(head.as_bytes()).is_err() {
                    return;
                }
                if trickle {
                    for chunk in body.chunks(188) {
                        if stream
                            .write_all(chunk)
                            .and_then(|()| stream.flush())
                            .is_err()
                        {
                            return;
                        }
                        thread::sleep(Duration::from_millis(50));
                    }
                } else {
                    let _ = stream.write_all(&body);
                }
                let mut rest = Vec::new();
                let _ = stream.read_to_end(&mut rest);
            });
        }
    });
    (port, requests)
}

fn packets(tag: u8, count: usize) -> Vec<u8> {
    let mut packet = [0x47; 188];
    packet[1] = tag;
    packet.repeat(count)
}

/// Run yt-dlp-rs in `directory`, send SIGTERM once `ready` holds for the
/// requested paths, and return its exit success and stdout.
fn stop_when(
    directory: &Path,
    args: &[&str],
    requests: &Arc<Mutex<Vec<String>>>,
    ready: impl Fn(&[String]) -> bool,
) -> (bool, String, String) {
    let child = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--keep-partial-on-stop",
            "--fixup",
            "never",
        ])
        .args(["--print", "after_move:filepath", "-o", "o.%(ext)s"])
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready(&requests.lock().unwrap()) {
        assert!(Instant::now() < deadline, "{:?}", requests.lock().unwrap());
        thread::sleep(Duration::from_millis(20));
    }
    // Some data of the trickled response first.
    thread::sleep(Duration::from_millis(300));
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let output = child.wait_with_output().unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_stopped_hls_download_keeps_its_whole_fragments_at_the_final_name() {
    let (port, requests) = server(|path| match path {
        "/index.m3u8" => Some(Body::Whole(
            "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\ns0.ts\n#EXTINF:2.0,\ns1.ts\n\
             #EXTINF:2.0,\ns2.ts\n#EXT-X-ENDLIST\n"
                .into(),
        )),
        "/s0.ts" => Some(Body::Whole(packets(0, 4))),
        "/s1.ts" => Some(Body::Whole(packets(1, 4))),
        "/s2.ts" => Some(Body::Trickle(packets(2, 400))),
        _ => None,
    });
    let work = tempfile::tempdir().unwrap();
    let url = format!("http://127.0.0.1:{port}/index.m3u8");
    let (success, stdout, stderr) = stop_when(work.path(), &[&url], &requests, |paths| {
        paths.iter().any(|path| path == "/s2.ts")
    });
    assert!(!success);
    assert!(stderr.contains("ERROR: Interrupted by user"), "{stderr}");
    let file = work.path().join("o.mp4");
    assert_eq!(
        std::fs::canonicalize(stdout.trim_end()).unwrap(),
        std::fs::canonicalize(&file).unwrap(),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read(&file).unwrap(),
        [packets(0, 4), packets(1, 4)].concat()
    );
    assert_eq!(names(work.path()), ["o.mp4"]);
}

#[test]
fn a_stopped_merged_download_merges_the_audio_and_the_partial_video() {
    use std::os::unix::fs::PermissionsExt;
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = tools.path().join("ffmpeg");
    // Concatenates its inputs into the output (the last argument).
    std::fs::write(
        &ffmpeg,
        "#!/bin/sh\n\
         case \"$1\" in -version) echo 'ffmpeg version keep-partial-fixture'; exit 0;; esac\n\
         prev=; inputs=\n\
         for a; do [ \"$prev\" = -i ] && inputs=\"$inputs ${a#file:}\"; prev=$a; out=$a; done\n\
         cat $inputs > \"${out#file:}\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (port, requests) = server(|path| match path {
        "/video" => Some(Body::Trickle(packets(9, 400))),
        "/audio" => Some(Body::Whole(b"audio-bytes".to_vec())),
        _ => None,
    });
    let work = tempfile::tempdir().unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let formats = serde_json::json!([
        {"format_id": "v", "url": format!("{base}/video"), "ext": "mp4",
         "vcodec": "avc1", "acodec": "none", "protocol": "http"},
        {"format_id": "a", "url": format!("{base}/audio"), "ext": "m4a",
         "vcodec": "none", "acodec": "mp4a.40.2", "protocol": "http"},
    ]);
    std::fs::write(
        work.path().join("m.info.json"),
        serde_json::json!({
            "id": "m", "title": "M", "ext": "mp4",
            "webpage_url": format!("{base}/"), "extractor": "fixture",
            "extractor_key": "Fixture", "formats": formats,
        })
        .to_string(),
    )
    .unwrap();
    let ffmpeg = ffmpeg.to_string_lossy().into_owned();
    let (success, stdout, stderr) = stop_when(
        work.path(),
        &[
            "--ffmpeg-location",
            &ffmpeg,
            "--load-info-json",
            "m.info.json",
            "-f",
            "v+a",
        ],
        &requests,
        |paths| paths.iter().any(|path| path == "/video"),
    );
    assert!(!success);
    assert!(stderr.contains("ERROR: Interrupted by user"), "{stderr}");
    // The audio was fetched first.
    let requested = requests.lock().unwrap().clone();
    assert_eq!(requested[..2], ["/audio", "/video"], "{stderr}");
    let file = work.path().join("o.mp4");
    assert_eq!(
        std::fs::canonicalize(stdout.trim_end()).unwrap(),
        std::fs::canonicalize(&file).unwrap(),
        "{stderr}"
    );
    // Merge inputs in selection order: the partial video, then the audio.
    let merged = std::fs::read(&file).unwrap();
    let video = &merged[..merged.len() - b"audio-bytes".len()];
    assert!(merged.ends_with(b"audio-bytes"));
    assert!(
        !video.is_empty() && video.len() < 400 * 188,
        "{}",
        video.len()
    );
    assert_eq!(video, &packets(9, 400)[..video.len()]);
    assert_eq!(names(work.path()), ["m.info.json", "o.mp4"]);
}
