//! Upstream `process_info`/`HttpFD` error reporting for media and subtitle
//! downloads, checked against pinned upstream with the same saved info JSON.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

/// Serves `/v.mp4`; `/busy*` answers 503 and anything else 404 with a
/// non-canonical reason phrase like Python's `http.server`.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (status, body): (&str, &[u8]) = match path.as_str() {
                "/v.mp4" => ("200 OK", b"media"),
                path if path.starts_with("/busy") => ("503 Service Unavailable", b"busy"),
                _ => ("404 File not found", b"missing"),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    origin
}

fn run(directory: &Path, media: &str, subtitle: Option<&str>, args: &[&str]) -> Output {
    let origin = serve();
    let mut info = serde_json::json!({
        "id": "vid", "title": "Vid", "ext": "mp4",
        "webpage_url": format!("{origin}/v.mp4"),
        "extractor": "generic", "extractor_key": "Generic",
        "formats": [{"format_id": "0", "url": format!("{origin}{media}"), "ext": "mp4",
            "protocol": "http", "vcodec": "avc1", "acodec": "mp4a.40.2"}],
    });
    if let Some(subtitle) = subtitle {
        info["subtitles"] =
            serde_json::json!({"en": [{"url": format!("{origin}{subtitle}"), "ext": "vtt"}]});
    }
    let path = directory.join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
        .arg("--load-info-json")
        .arg(&path)
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn media_http_errors_are_reported_before_any_destination() {
    let directory = tempfile::tempdir().unwrap();
    let output = run(directory.path(), "/missing.mp4", None, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stdout),
        "[info] vid: Downloading 1 format(s): 0\n"
    );
    assert_eq!(
        text(&output.stderr),
        "ERROR: unable to download video data: HTTP Error 404: File not found\n"
    );
    assert!(!directory.path().join("o.mp4").exists());
}

#[test]
fn retryable_errors_report_through_the_retry_manager() {
    for (retries, stdout, error) in [
        (
            "0",
            "",
            "ERROR: \r[download] Got error: HTTP Error 503: Service Unavailable\n",
        ),
        (
            "1",
            "[download] Got error: HTTP Error 503: Service Unavailable. Retrying (1/1)...\n",
            "ERROR: \r[download] Got error: HTTP Error 503: Service Unavailable. Giving up after 1 retries\n",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = run(
            directory.path(),
            "/busy.mp4",
            None,
            &["-R", retries, "--retry-sleep", "0"],
        );
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            text(&output.stdout),
            format!("[info] vid: Downloading 1 format(s): 0\n{stdout}")
        );
        assert_eq!(text(&output.stderr), error);
    }
}

#[test]
fn subtitle_failures_warn_under_ignore_errors() {
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        "/v.mp4",
        Some("/nosub.vtt"),
        &["--write-subs", "-i"],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    // No destination is named for the failed subtitle.
    assert_eq!(
        text(&output.stdout),
        "[info] vid: Downloading subtitles: en\n[info] vid: Downloading 1 format(s): 0\n\
         [info] Writing video subtitles to: o.en.vtt\n[download] Destination: o.mp4\n\
         [download] Download completed\n"
    );
    assert_eq!(
        text(&output.stderr),
        "WARNING: Unable to download video subtitles for 'en': HTTP Error 404: File not found\n"
    );
    assert!(directory.path().join("o.mp4").is_file());
}
