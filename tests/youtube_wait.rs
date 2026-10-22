//! Upstream `--wait-for-video` for an upcoming YouTube stream: its reason is
//! a warning, `_wait_for_video` waits (here the 1s minimum, the release time
//! being later) and the video is extracted again.
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

#[test]
fn an_upcoming_video_is_waited_for_and_extracted_again() {
    let proxy = site_proxy::SiteProxy::start("youtube-live");
    let work = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
        ])
        .args([
            "--user-agent",
            "fixture",
            "--no-js-runtimes",
            "-s",
            "--no-progress",
        ])
        .args(["--wait-for-video", "1-2"])
        .arg("https://www.youtube.com/watch?v=Upcoming001")
        .current_dir(work.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut lines = Vec::new();
    for line in stdout.lines() {
        let line = line.unwrap();
        let again = line.starts_with("[youtube] Extracting URL") && !lines.is_empty();
        lines.push(line);
        if again {
            break;
        }
    }
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("WARNING: [youtube] This live event will begin in 3 hours.\n"),
        "{stderr}"
    );
    let waiting: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .filter(|line| line.starts_with("[wait]"))
        .collect();
    // The 2s maximum bounds the wait; `--no-progress` shows one countdown.
    assert_eq!(waiting.len(), 3, "{lines:?}");
    assert_eq!(
        waiting[0],
        "[wait] Waiting for 00:00:02 - Press Ctrl+C to try now"
    );
    assert!(
        waiting[1] == "[wait] Remaining time until next attempt: 00:00:02"
            || waiting[1] == "[wait] Remaining time until next attempt: 00:00:01",
        "{lines:?}"
    );
    assert_eq!(waiting[2], "[wait] Wait period ended; Re-extracting data");
    assert_eq!(
        lines.last().unwrap(),
        "[youtube] Extracting URL: https://www.youtube.com/watch?v=Upcoming001"
    );
}
