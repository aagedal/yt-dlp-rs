//! SIGINT/SIGTERM during a run (upstream's `KeyboardInterrupt`): the run
//! ends at the next checkpoint with `ERROR: Interrupted by user` (exit 1),
//! later URLs are not processed, the upstream `.part` file is kept for
//! `--continue` (also when postprocessors are enabled) and no private
//! temporary files remain. A second signal exits at once. SIGTERM is sent,
//! since background test processes may inherit an ignored SIGINT.
#![cfg(unix)]
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const SIZE: usize = 64 * 1024;

/// A server for `/slow.mp4` (a 188-byte packet every 50 ms), `/stalled.mp4`
/// (headers, a little data, then nothing) and `/fast*` (all at once),
/// recording paths.
fn server() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let record = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let record = record.clone();
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
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {SIZE}\r\nConnection: close\r\n\r\n"
                );
                if stream.write_all(head.as_bytes()).is_err() {
                    return;
                }
                let body = vec![0x47; SIZE];
                if path.starts_with("/fast") {
                    let _ = stream.write_all(&body);
                    return;
                }
                if path == "/stalled.mp4" {
                    let _ = stream.write_all(&body[..1024]);
                    let _ = stream.flush();
                    thread::sleep(Duration::from_secs(60));
                    return;
                }
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
            });
        }
    });
    (port, requests)
}

fn spawn(directory: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
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

fn wait_for(requests: &Arc<Mutex<Vec<String>>>, path: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !requests
        .lock()
        .unwrap()
        .iter()
        .any(|request| request == path)
    {
        assert!(Instant::now() < deadline, "{:?}", requests.lock().unwrap());
        thread::sleep(Duration::from_millis(20));
    }
}

fn terminate(child: &Child) {
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

/// Exit status code, stderr and the directory's file names.
fn finish(child: Child, directory: &Path) -> (Option<i32>, String, Vec<String>) {
    let output = child.wait_with_output().unwrap();
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        names,
    )
}

fn part_size(directory: &Path, name: &str) -> u64 {
    std::fs::metadata(directory.join(name)).unwrap().len()
}

#[test]
fn a_signal_ends_the_run_and_keeps_the_part_file() {
    let (port, requests) = server();
    let directory = tempfile::tempdir().unwrap();
    let slow = format!("http://127.0.0.1:{port}/slow.mp4");
    let next = format!("http://127.0.0.1:{port}/next.mp4");
    let child = spawn(directory.path(), &[&slow, &next]);
    wait_for(&requests, "/slow.mp4");
    thread::sleep(Duration::from_millis(300));
    terminate(&child);
    let (code, stderr, names) = finish(child, directory.path());
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.ends_with("\nERROR: Interrupted by user\n"),
        "{stderr}"
    );
    assert!(!stderr.contains("cancelled"), "{stderr}");
    assert_eq!(names, ["slow.mp4.part"]);
    assert!(part_size(directory.path(), "slow.mp4.part") > 0);
    // The run ended: the next URL was not requested.
    assert!(
        !requests
            .lock()
            .unwrap()
            .iter()
            .any(|path| path == "/next.mp4")
    );
}

#[test]
fn a_signal_keeps_the_part_file_of_a_download_to_be_processed() {
    let (port, requests) = server();
    let directory = tempfile::tempdir().unwrap();
    // An FFmpeg that only answers version checks: the signal comes first.
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = tools.path().join("ffmpeg");
    std::fs::write(
        &ffmpeg,
        "#!/bin/sh\necho 'ffmpeg version 7.1 Copyright (c) 2000-2024 the FFmpeg developers'\n",
    )
    .unwrap();
    std::fs::set_permissions(&ffmpeg, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let slow = format!("http://127.0.0.1:{port}/slow.mp4");
    let child = spawn(
        directory.path(),
        &[
            "--embed-metadata",
            "--ffmpeg-location",
            ffmpeg.to_str().unwrap(),
            &slow,
        ],
    );
    wait_for(&requests, "/slow.mp4");
    thread::sleep(Duration::from_millis(300));
    terminate(&child);
    let (code, stderr, names) = finish(child, directory.path());
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.ends_with("\nERROR: Interrupted by user\n"),
        "{stderr}"
    );
    // Upstream's `NAME.part`, not a hidden private workspace.
    assert_eq!(names, ["slow.mp4.part"]);
    assert!(part_size(directory.path(), "slow.mp4.part") > 0);
}

#[test]
fn a_second_signal_exits_without_waiting_for_a_stalled_read() {
    let (port, requests) = server();
    let directory = tempfile::tempdir().unwrap();
    let stalled = format!("http://127.0.0.1:{port}/stalled.mp4");
    let mut child = spawn(directory.path(), &[&stalled]);
    wait_for(&requests, "/stalled.mp4");
    thread::sleep(Duration::from_millis(300));
    terminate(&child);
    // The first signal waits for the blocked read (the socket timeout).
    thread::sleep(Duration::from_millis(500));
    assert!(child.try_wait().unwrap().is_none());
    let sent = Instant::now();
    terminate(&child);
    let (code, stderr, _) = finish(child, directory.path());
    assert!(sent.elapsed() < Duration::from_secs(5));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.ends_with("\nERROR: Interrupted by user\n"),
        "{stderr}"
    );
}

#[test]
fn a_signal_at_a_prompt_exits_at_once() {
    let (port, requests) = server();
    let directory = tempfile::tempdir().unwrap();
    let slow = format!("http://127.0.0.1:{port}/slow.mp4");
    // `-f -` asks for a selector on stdin, which stays open.
    let mut child = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "-f", "-", &slow])
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut prompt = Vec::new();
    while !String::from_utf8_lossy(&prompt).contains("Ctrl+C to quit") {
        let mut byte = [0];
        assert_eq!(std::io::Read::read(&mut stdout, &mut byte).unwrap(), 1);
        prompt.push(byte[0]);
    }
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .all(|path| path == "/slow.mp4")
    );
    let sent = Instant::now();
    terminate(&child);
    let (code, stderr, names) = finish(child, directory.path());
    assert!(sent.elapsed() < Duration::from_secs(5));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.ends_with("\nERROR: Interrupted by user\n"),
        "{stderr}"
    );
    assert!(names.is_empty(), "{names:?}");
}

#[test]
fn a_killed_runs_finished_download_is_recovered_by_the_next_run() {
    let (port, _) = server();
    let directory = tempfile::tempdir().unwrap();
    // An FFmpeg that answers version checks and hangs when processing.
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = tools.path().join("ffmpeg");
    std::fs::write(
        &ffmpeg,
        "#!/bin/sh\ncase \"$1\" in -version|-bsfs) echo 'ffmpeg version 7.1';; *) sleep 30;; esac\n",
    )
    .unwrap();
    std::fs::set_permissions(&ffmpeg, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let fast = format!("http://127.0.0.1:{port}/fast.mp4");
    let mut child = spawn(
        directory.path(),
        &[
            "--embed-metadata",
            "--ffmpeg-location",
            ffmpeg.to_str().unwrap(),
            &fast,
        ],
    );
    // Wait for the downloaded source inside the processing workspace.
    let deadline = Instant::now() + Duration::from_secs(30);
    let workspace_source = || {
        std::fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".yt-dlp-rs-process-")
            })
            .any(|entry| entry.path().join("source.mp4").exists())
    };
    while !workspace_source() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    // The next download into the directory recovers it.
    let other = format!("http://127.0.0.1:{port}/fast-other.mp4");
    let (code, stderr, names) = finish(spawn(directory.path(), &[&other]), directory.path());
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stderr.contains("Recovered "), "{stderr}");
    assert_eq!(names, ["fast-other.mp4", "fast.mp4"]);
    assert_eq!(part_size(directory.path(), "fast.mp4"), SIZE as u64);
}
