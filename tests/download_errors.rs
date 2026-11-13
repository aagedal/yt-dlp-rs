//! Upstream `process_info`/`HttpFD` error reporting for media and subtitle
//! downloads, checked against pinned upstream with the same saved info JSON.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

mod support;

/// A media playlist whose second segment is missing.
const GAP: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\ns0.ts\n#EXTINF:2.0,\ns1.ts\n#EXTINF:2.0,\ns2.ts\n#EXT-X-ENDLIST\n";

/// The same with an empty second segment.
const EMPTY: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\ns0.ts\n#EXTINF:2.0,\nempty.ts\n#EXTINF:2.0,\ns2.ts\n#EXT-X-ENDLIST\n";

/// The same with a second segment cut short.
const SHORT: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\ns0.ts\n#EXTINF:2.0,\nshort.ts\n#EXTINF:2.0,\ns2.ts\n#EXT-X-ENDLIST\n";

/// Two available segments.
const TWO: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\ns0.ts\n#EXTINF:2.0,\ns2.ts\n#EXT-X-ENDLIST\n";

/// A media playlist whose first segment is missing.
const HEADLESS: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\ns1.ts\n#EXTINF:2.0,\ns0.ts\n#EXTINF:2.0,\ns2.ts\n#EXT-X-ENDLIST\n";

/// Serves `/v.mp4` and the `/gap.m3u8`, `/empty.m3u8` and `/headless.m3u8` playlists; `/busy*`
/// answers 503 and anything else 404 with a non-canonical reason phrase like Python's
/// `http.server`.
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
                "/gap.m3u8" => ("200 OK", GAP.as_bytes()),
                "/empty.m3u8" => ("200 OK", EMPTY.as_bytes()),
                "/two.m3u8" => ("200 OK", TWO.as_bytes()),
                "/headless.m3u8" => ("200 OK", HEADLESS.as_bytes()),
                "/s0.ts" | "/s2.ts" => ("200 OK", b"segment"),
                "/empty.ts" => ("200 OK", b""),
                "/short.m3u8" => ("200 OK", SHORT.as_bytes()),
                "/short.ts" => {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npartial",
                    );
                    continue;
                }
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

/// `--ignore-config --no-progress -o o.%(ext)s` with `args` in `directory`.
fn fragments(directory: &Path, args: &[&str]) -> Output {
    support::output_within_a_minute(
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
            .args(args)
            .current_dir(directory),
    )
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

#[test]
fn a_raised_subtitle_failure_retries_the_loaded_info_with_its_url() {
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        "/v.mp4",
        Some("/nosub.vtt"),
        &["--write-subs"],
    );
    // The retry succeeds, so the run does too.
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(
        stdout.starts_with(
            "[info] vid: Downloading subtitles: en\n[info] vid: Downloading 1 format(s): 0\n\
             [info] Writing video subtitles to: o.en.vtt\n[generic] Extracting URL: http://"
        ),
        "{stdout}"
    );
    assert!(stdout.contains("[info] There are no subtitles for the requested languages\n"));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with(
            "\r\nWARNING: The info failed to download: Unable to download video subtitles for 'en': HTTP Error 404: File not found; trying with URL http://"
        ),
        "{stderr}"
    );
    assert!(directory.path().join("o.mp4").is_file());
}

#[test]
fn transport_errors_use_urllib_errno_text() {
    // A port that was just bound and released refuses connections.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    let directory = tempfile::tempdir().unwrap();
    let info = serde_json::json!({
        "id": "vid", "title": "Vid", "ext": "mp4",
        "webpage_url": format!("http://{address}/v.mp4"),
        "extractor": "generic", "extractor_key": "Generic",
        "formats": [{"format_id": "0", "url": format!("http://{address}/v.mp4"), "ext": "mp4",
            "protocol": "http", "vcodec": "avc1", "acodec": "mp4a.40.2"}],
    });
    let path = directory.path().join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--no-progress",
            "-R",
            "1",
            "--retry-sleep",
            "0",
        ])
        .arg("--load-info-json")
        .arg(&path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = text(&output.stdout);
    let retry = stdout.lines().nth(1).unwrap();
    let (prefix, refused) = if cfg!(windows) {
        (
            "[download] Got error: [WinError ",
            "] No connection could be made because the target machine actively refused it. Retrying (1/1)...",
        )
    } else {
        (
            "[download] Got error: [Errno ",
            "] Connection refused. Retrying (1/1)...",
        )
    };
    assert!(
        retry.starts_with(prefix) && retry.ends_with(refused),
        "{stdout}"
    );
    let gave_up = if cfg!(windows) {
        "] No connection could be made because the target machine actively refused it. Giving up after 1 retries\n"
    } else {
        "] Connection refused. Giving up after 1 retries\n"
    };
    assert!(
        text(&output.stderr).ends_with(gave_up),
        "{}",
        text(&output.stderr)
    );
}

/// Also with `-N`: the workers' retry and skip lines once deadlocked
/// against the thread holding stdout.
#[test]
fn unavailable_fragments_are_skipped_or_end_the_download() {
    let origin = serve();
    for (abort, jobs) in [(false, "1"), (true, "1"), (false, "2"), (true, "2")] {
        let directory = tempfile::tempdir().unwrap();
        let playlist = format!("{origin}/gap.m3u8");
        let mut args = vec!["--fragment-retries", "1", "--retry-sleep", "fragment:0"];
        args.extend(["-N", jobs, &playlist]);
        if abort {
            args.push("--abort-on-unavailable-fragments");
        }
        let output = fragments(directory.path(), &args);
        let stdout = text(&output.stdout);
        assert!(stdout.contains(
            "[download] Got error: HTTP Error 404: File not found. Retrying fragment 2 (1/1)...\n"
        ));
        if abort {
            assert_eq!(output.status.code(), Some(1));
            assert!(text(&output.stderr).ends_with(
                "ERROR: \r[download] Got error: HTTP Error 404: File not found. Giving up after 1 retries\n\
                 ERROR: fragment 2 not found, unable to continue\n"
            ));
        } else {
            assert!(output.status.success(), "{}", text(&output.stderr));
            assert!(stdout.contains("[download] fragment not found; Skipping fragment 2 ...\n"));
            assert_eq!(
                std::fs::read(directory.path().join("o.mp4")).unwrap(),
                b"segmentsegment"
            );
        }
    }
}

/// HttpFD's `Did not get any data blocks` for an empty fragment: reported
/// (the run exits 1), then skipped or fatal like an unavailable one.
#[test]
fn empty_fragments_are_reported_then_skipped() {
    let origin = serve();
    for (abort, jobs) in [(false, "1"), (true, "1"), (false, "2"), (true, "2")] {
        let directory = tempfile::tempdir().unwrap();
        let playlist = format!("{origin}/empty.m3u8");
        let mut args = vec!["-N", jobs, &playlist];
        if abort {
            args.push("--abort-on-unavailable-fragments");
        }
        let output = fragments(directory.path(), &args);
        let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(!stdout.contains("Retrying"), "{stdout}");
        if abort {
            assert!(
                stderr.ends_with(
                    "\n\nERROR: Did not get any data blocks\n\
                     ERROR: fragment 2 not found, unable to continue\n"
                ),
                "{stderr}"
            );
            assert!(!directory.path().join("o.mp4").exists());
        } else {
            assert!(
                stderr.ends_with("\n\nERROR: Did not get any data blocks\n"),
                "{stderr}"
            );
            assert!(stdout.contains("[download] fragment not found; Skipping fragment 2 ...\n"));
            assert_eq!(
                std::fs::read(directory.path().join("o.mp4")).unwrap(),
                b"segmentsegment"
            );
        }
    }
}

/// A fragment whose body ends early is a transport error, retried, then
/// skipped like an unavailable one (not a filesystem error that ends the
/// download).
#[test]
fn a_truncated_fragment_is_retried_then_skipped() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let playlist = format!("{origin}/short.m3u8");
    let output = fragments(
        directory.path(),
        &[
            "--fragment-retries",
            "1",
            "--retry-sleep",
            "fragment:0",
            &playlist,
        ],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains(
            "[download] Got error: 7 bytes read, 93 more expected. Retrying fragment 2 (1/1)...\n"
        ),
        "{stdout}"
    );
    assert!(stdout.contains("[download] fragment not found; Skipping fragment 2 ...\n"));
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"segmentsegment"
    );
}

/// FragmentFD's HttpFD applies `--min/max-filesize` to each fragment:
/// one out of bounds is skipped (or fatal) without a message, and with
/// every fragment skipped the download is empty and fails.
#[test]
fn filesize_limits_apply_to_each_fragment() {
    struct Case {
        args: &'static [&'static str],
        stdout_has: &'static str,
        stderr_end: &'static str,
        file: Option<&'static [u8]>,
        /// The files left, an abort's part file and state among them.
        kept: &'static [&'static str],
    }
    let origin = serve();
    let playlist = format!("{origin}/two.m3u8");
    for case in [
        Case {
            args: &["--max-filesize", "3"],
            stdout_has: "[download] fragment not found; Skipping fragment 2 ...\n",
            stderr_end: "ERROR: The downloaded file is empty\n",
            file: None,
            kept: &[],
        },
        Case {
            args: &["--max-filesize", "3", "--abort-on-unavailable-fragments"],
            stdout_has: "",
            stderr_end: "ERROR: fragment 1 not found, unable to continue\n",
            file: None,
            kept: &["o.mp4.part", "o.mp4.ytdl"],
        },
        Case {
            args: &["--min-filesize", "7"],
            stdout_has: "",
            stderr_end: "",
            file: Some(b"segmentsegment"),
            kept: &["o.mp4"],
        },
    ] {
        let args = case.args;
        let directory = tempfile::tempdir().unwrap();
        let output = fragments(directory.path(), &[args, &[&playlist]].concat());
        let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
        assert!(stdout.contains(case.stdout_has), "{args:?}: {stdout}");
        assert!(!stdout.contains("Retrying"), "{args:?}: {stdout}");
        assert!(stderr.ends_with(case.stderr_end), "{args:?}: {stderr}");
        assert_eq!(
            output.status.success(),
            case.file.is_some(),
            "{args:?}: {stderr}"
        );
        if let Some(bytes) = case.file {
            assert_eq!(
                std::fs::read(directory.path().join("o.mp4")).unwrap(),
                bytes
            );
        }
        let mut names: Vec<String> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, case.kept, "{args:?}");
    }
}

/// HlsFD may skip the first fragment too (only DashSegmentsFD never does).
#[test]
fn a_missing_first_hls_fragment_is_skipped() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let playlist = format!("{origin}/headless.m3u8");
    let output = fragments(directory.path(), &["--fragment-retries", "0", &playlist]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).contains("[download] fragment not found; Skipping fragment 1 ...\n")
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"segmentsegment"
    );
}

/// Upstream `FragmentFD` resume: an aborted download keeps `FILE.part` and
/// `FILE.ytdl`, and `--continue` skips the fragments already written.
#[test]
fn interrupted_fragment_downloads_resume_from_the_ytdl_state() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let available = Arc::new(AtomicBool::new(false));
    let first_requests = Arc::new(AtomicUsize::new(0));
    let (serve_available, serve_first) = (available.clone(), first_requests.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (status, body): (&str, &[u8]) = match path.as_str() {
                "/gap.m3u8" => ("200 OK", GAP.as_bytes()),
                "/s0.ts" => {
                    serve_first.fetch_add(1, Ordering::Relaxed);
                    ("200 OK", b"first-")
                }
                "/s1.ts" if serve_available.load(Ordering::Relaxed) => ("200 OK", b"second-"),
                "/s2.ts" => ("200 OK", b"third"),
                _ => ("404 Not Found", b"missing"),
            };
            let mime = if path.ends_with(".m3u8") {
                "application/vnd.apple.mpegurl"
            } else {
                "video/mp2t"
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
            .args([
                "--fragment-retries",
                "0",
                "--abort-on-unavailable-fragments",
            ])
            .arg(format!("{origin}/gap.m3u8"))
            .current_dir(directory.path())
            .output()
            .unwrap()
    };
    let output = run();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4.part")).unwrap(),
        b"first-"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("o.mp4.ytdl")).unwrap(),
        r#"{"downloader": {"current_fragment": {"index": 1}, "extra_state": {}}}"#
    );
    let fetched = first_requests.load(Ordering::Relaxed);
    available.store(true, Ordering::Relaxed);
    let output = run();
    assert!(
        output.status.success(),
        "{}\n{}",
        text(&output.stdout),
        text(&output.stderr)
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"first-second-third"
    );
    // The first fragment was not fetched again; the state is removed.
    assert_eq!(first_requests.load(Ordering::Relaxed), fetched);
    assert!(!directory.path().join("o.mp4.ytdl").exists());
    assert!(!directory.path().join("o.mp4.part").exists());
}

/// A run killed in the middle of a fragment (SIGKILL, a crash) leaves only
/// whole fragments in `FILE.part`, so `--continue` builds on them instead of
/// appending the fragment again after its first bytes.
#[test]
fn a_killed_fragment_download_resumes_without_its_partial_fragment() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    const SECOND: &[u8] = &[b'2'; 64 << 10];
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (stall, half_sent) = (
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
    );
    let (serve_stall, serve_half) = (stall.clone(), half_sent.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (stall, half_sent) = (serve_stall.clone(), serve_half.clone());
            thread::spawn(move || {
                let mut request = [0; 4096];
                let read = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body: &[u8] = match path.as_str() {
                    "/gap.m3u8" => GAP.as_bytes(),
                    "/s0.ts" => b"first-",
                    "/s1.ts" => SECOND,
                    _ => b"third",
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if path == "/s1.ts" && stall.load(Ordering::Relaxed) {
                    // Half the segment, then nothing until the client dies.
                    let _ = stream.write_all(&body[..body.len() / 2]);
                    let _ = stream.flush();
                    half_sent.store(true, Ordering::Relaxed);
                    thread::sleep(std::time::Duration::from_secs(60));
                    return;
                }
                let _ = stream.write_all(body);
            });
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
    command
        .args([
            "--ignore-config",
            "--no-progress",
            "--fixup",
            "never",
            "-o",
            "o.ts",
        ])
        .arg(format!("{origin}/gap.m3u8"))
        .current_dir(directory.path());
    let mut child = command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !half_sent.load(Ordering::Relaxed) {
        assert!(
            std::time::Instant::now() < deadline,
            "the stall was not reached"
        );
        thread::sleep(std::time::Duration::from_millis(20));
    }
    // Time for the received half to reach the file, as it once did.
    thread::sleep(std::time::Duration::from_millis(500));
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("o.ts.part")).unwrap(),
        b"first-"
    );
    stall.store(false, Ordering::Relaxed);
    let output = support::output_within_a_minute(&mut command);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        std::fs::read(directory.path().join("o.ts")).unwrap(),
        [&b"first-"[..], SECOND, b"third"].concat()
    );
}

/// A failure after a fragment is whole but before `FILE.ytdl` counts it
/// (here `--keep-fragments` cannot write its copy) leaves the part file at
/// the fragments the state counts, so `--continue` does not append that
/// fragment twice.
#[test]
fn a_fragment_failing_after_its_download_is_not_resumed_twice() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let body: &[u8] = match path.as_str() {
                "/gap.m3u8" => GAP.as_bytes(),
                "/s0.ts" => b"first-",
                "/s1.ts" => b"second-",
                _ => b"third",
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    for concurrency in ["1", "2"] {
        let directory = tempfile::tempdir().unwrap();
        // A directory where the second fragment's kept copy goes.
        let blocker = directory.path().join("o.ts.part-Frag2");
        std::fs::create_dir(&blocker).unwrap();
        let run = || {
            Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
                .args(["--ignore-config", "--no-progress", "--fixup", "never"])
                .args(["-N", concurrency, "--keep-fragments", "-o", "o.ts"])
                .arg(format!("{origin}/gap.m3u8"))
                .current_dir(directory.path())
                .output()
                .unwrap()
        };
        let output = run();
        assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
        assert_eq!(
            std::fs::read(directory.path().join("o.ts.part")).unwrap(),
            b"first-",
            "-N {concurrency}"
        );
        std::fs::remove_dir(&blocker).unwrap();
        let output = run();
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert_eq!(
            std::fs::read(directory.path().join("o.ts")).unwrap(),
            b"first-second-third",
            "-N {concurrency}"
        );
    }
}

/// Upstream's filesystem errors name the path: HttpFD's `unable to open for
/// writing`, FragmentFD's `Unable to download video` (its `.ytdl` comes
/// first) and `_ensure_dir_exists`' `Unable to create directory`.
#[cfg(unix)]
#[test]
fn filesystem_errors_name_the_path_like_upstream() {
    use std::os::unix::fs::PermissionsExt;
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let (root, read_only, file) = (
        directory.path(),
        directory.path().join("ro"),
        directory.path().join("afile"),
    );
    std::fs::create_dir(&read_only).unwrap();
    std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o555)).unwrap();
    std::fs::write(&file, "").unwrap();
    // Permissions do not stop root.
    let enforced = std::fs::write(read_only.join("probe"), "").is_err();
    let mut cases = vec![(
        "afile/x.%(ext)s",
        "/v.mp4",
        format!(
            "Unable to create directory: [Errno 17] File exists: '{}'",
            file.display()
        ),
    )];
    if enforced {
        cases.extend([
            (
                "ro/x.%(ext)s",
                "/v.mp4",
                format!(
                    "unable to open for writing: [Errno 13] Permission denied: '{}'",
                    read_only.join("x.mp4.part").display()
                ),
            ),
            (
                "ro/x.%(ext)s",
                "/gap.m3u8",
                format!(
                    "Unable to download video: [Errno 13] Permission denied: '{}'",
                    read_only.join("x.mp4.ytdl").display()
                ),
            ),
            (
                "ro/new/deeper/x.%(ext)s",
                "/v.mp4",
                format!(
                    "Unable to create directory: [Errno 13] Permission denied: '{}'",
                    read_only.join("new").display()
                ),
            ),
        ]);
    }
    for (template, path, message) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "--no-progress", "-o"])
            .arg(root.join(template))
            .arg(format!("{origin}{path}"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(
            text(&output.stderr).ends_with(&format!("ERROR: {message}\n")),
            "{}",
            text(&output.stderr)
        );
    }
    std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o755)).unwrap();
}
