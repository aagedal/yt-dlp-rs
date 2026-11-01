//! Port extension: without ffmpeg, a live HLS stream is recorded natively by
//! polling its media playlist (upstream fails with `m3u8 download detected
//! but ffmpeg could not be found`).
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

/// A live playlist that gains a segment with each reload after the first
/// (the extraction's two reads), sliding a three-segment window, and ends
/// after six segments.
fn sliding(reload: usize) -> Option<String> {
    let count = (2 + reload.saturating_sub(2)).min(6);
    let first = count.saturating_sub(3);
    Some(playlist(first, count, count == 6))
}

fn playlist(first: usize, count: usize, ended: bool) -> String {
    let mut text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:{first}\n");
    for index in first..count {
        text.push_str(&format!("#EXTINF:1,\nseg{index}.ts\n"));
    }
    if ended {
        text.push_str("#EXT-X-ENDLIST\n");
    }
    text
}

/// Serves `/live.m3u8` from `playlist` by reload number (`None` is a 503)
/// and every other path as a segment naming itself.
fn serve(playlist: fn(usize) -> Option<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let reloads = Arc::new(AtomicUsize::new(0));
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0; 4096];
            let mut request = Vec::new();
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(length) => request.extend_from_slice(&buffer[..length]),
                }
            }
            let request = String::from_utf8_lossy(&request);
            let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
            let (status, mime, body) = if path == "/live.m3u8" {
                match playlist(reloads.fetch_add(1, Ordering::SeqCst)) {
                    Some(text) => ("200 OK", "application/x-mpegurl", text),
                    None => ("503 Service Unavailable", "text/plain", String::new()),
                }
            } else {
                (
                    "200 OK",
                    "video/mp2t",
                    format!("[{}]", path.trim_start_matches('/')),
                )
            };
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    base
}

fn record(base: &str) -> (std::process::Output, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ffmpeg-location",
            "/nonexistent/ffmpeg",
            "-o",
            "out.%(ext)s",
        ])
        .arg(format!("{base}/live.m3u8"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    (output, directory)
}

#[test]
fn live_hls_without_ffmpeg_is_recorded_natively() {
    let (output, directory) = record(&serve(sliding));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains(
            "WARNING: ffmpeg could not be found; recording the live HLS stream natively (yt-dlp-rs extension)"
        ),
        "{stderr}"
    );
    // Every segment once, in order, from the start of the first window.
    assert_eq!(
        std::fs::read_to_string(directory.path().join("out.mp4")).unwrap(),
        "[seg0.ts][seg1.ts][seg2.ts][seg3.ts][seg4.ts][seg5.ts]"
    );
}

#[test]
fn a_failed_reload_is_retried_and_an_ended_playlist_recorded_in_full() {
    // Live during extraction; the first reload fails, the next one has
    // ended with more segments than the live-edge start would take.
    let (output, directory) = record(&serve(|reload| match reload {
        0 | 1 => Some(playlist(0, 2, false)),
        2 => None,
        _ => Some(playlist(0, 6, true)),
    }));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("[hlsnative] Reloading the live playlist failed"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("out.mp4")).unwrap(),
        "[seg0.ts][seg1.ts][seg2.ts][seg3.ts][seg4.ts][seg5.ts]"
    );
}
