//! Media body read failures in upstream's (requests handler) words,
//! compared with pinned upstream: a body that ends early is
//! `IncompleteRead`'s `N bytes read, M more expected`, a stalled one
//! `HTTPConnectionPool(host='H', port=P): Read timed out.`; both are
//! retried like a `TransportError`.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
    time::Duration,
};

/// `/short.mp4` sends 7 of 100 bytes and closes; `/stall.mp4` sends them
/// and then nothing.
fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 100\r\nConnection: close\r\n\r\npartial",
                );
                let _ = stream.flush();
                if String::from_utf8_lossy(&request).starts_with("GET /stall") {
                    thread::sleep(Duration::from_secs(20));
                }
            });
        }
    });
    port
}

#[test]
fn body_read_failures_use_upstream_transport_error_text() {
    let port = serve();
    for (path, text) in [
        ("short.mp4", "7 bytes read, 93 more expected".to_owned()),
        (
            "stall.mp4",
            format!("HTTPConnectionPool(host='127.0.0.1', port={port}): Read timed out."),
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "--no-progress", "--retries", "1"])
            .args(["--retry-sleep", "0", "--socket-timeout", "1", "-o", "o.mp4"])
            .arg(format!("http://127.0.0.1:{port}/{path}"))
            .current_dir(directory.path())
            .output()
            .unwrap();
        let (stdout, stderr) = (
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(
            stdout.contains(&format!(
                "[download] Got error: {text}. Retrying (1/1)...\n"
            )),
            "{stdout}"
        );
        assert!(
            stderr.contains(&format!(
                "[download] Got error: {text}. Giving up after 1 retries"
            )),
            "{stderr}"
        );
        assert!(!directory.path().join("o.mp4").exists());
    }
}

/// A chunked resume the server answers with the whole file (`200`, no
/// range) restarts at byte 0, and a read failure counts from there.
#[test]
fn a_chunked_resume_without_a_range_counts_from_the_restart() {
    let port = serve();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("o.mp4.part"), [0; 50]).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "--retries", "0"])
        .args(["--http-chunk-size", "30", "--continue", "-o", "o.mp4"])
        .arg(format!("http://127.0.0.1:{port}/short.mp4"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stdout.contains("[download] Unable to resume\n"), "{stdout}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(
        stderr.contains("7 bytes read, 93 more expected"),
        "{stderr}"
    );
}
