//! Upstream `YoutubeDL._wait_for_video` for any extractor: a video without
//! formats yet (PiaLive's upcoming stream, whose release time has passed)
//! is waited for (the 1s minimum) and extracted again.
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

#[test]
fn an_upcoming_video_of_any_site_is_waited_for_and_extracted_again() {
    let proxy = site_proxy::SiteProxy::start("pialive");
    let work = tempfile::tempdir().unwrap();
    let url = "https://player.pia-live.jp/stream/KEY-upcoming";
    let mut child = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--proxy", &proxy.url])
        .args(["--no-check-certificate", "-s", "--no-progress"])
        .args(["--wait-for-video", "1"])
        .arg(url)
        .current_dir(work.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut lines = Vec::new();
    for line in stdout.lines() {
        let line = line.unwrap();
        let again = line.contains("Extracting URL") && !lines.is_empty();
        lines.push(line);
        if again {
            break;
        }
    }
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("WARNING: Video should already be available according to extracted info"),
        "{stderr}"
    );
    let waiting: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .filter(|line| line.starts_with("[wait]"))
        .collect();
    assert_eq!(
        waiting.first().copied(),
        Some("[wait] Waiting for 00:00:01 - Press Ctrl+C to try now"),
        "{lines:?}"
    );
    assert_eq!(
        waiting.last().copied(),
        Some("[wait] Wait period ended; Re-extracting data"),
        "{lines:?}"
    );
    assert!(
        lines
            .last()
            .unwrap()
            .ends_with(&format!("Extracting URL: {url}")),
        "{lines:?}"
    );
}
