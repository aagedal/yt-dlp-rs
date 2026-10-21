//! Upstream `FFmpegFD.real_download`: interrupting a live download asks
//! FFmpeg to quit (`q` on its stdin) and completes it like a finished one.
#![cfg(unix)]
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// A fake FFmpeg: answers `-version`, writes `partial` to its output (the
/// last argument, a `file:` URL), then waits for `q` on stdin and writes `finished`.
fn fake_ffmpeg(directory: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("ffmpeg");
    std::fs::write(
        &path,
        "#!/bin/sh\n\
         case \"$1\" in -version) echo 'ffmpeg version live-fixture'; exit 0;; esac\n\
         for out; do :; done\n\
         out=${out#file:}\n\
         printf partial > \"$out\"\n\
         while :; do c=$(dd bs=1 count=1 2>/dev/null); [ \"$c\" = q ] && break; [ -z \"$c\" ] && exit 9; done\n\
         printf finished > \"$out\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

#[test]
fn an_interrupted_live_download_is_finished_and_published() {
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(directory.path());
    std::fs::write(
        directory.path().join("live.info.json"),
        serde_json::json!({
            "id": "live", "title": "Live", "ext": "mp4",
            "url": "http://127.0.0.1:9/live/index.m3u8", "protocol": "m3u8_native",
            "live_status": "is_live", "webpage_url": "https://example.test/live",
            "extractor": "fixture", "extractor_key": "Fixture",
        })
        .to_string(),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "--fixup", "never"])
        .args(["--ffmpeg-location", &ffmpeg, "-o", "o.%(ext)s"])
        .args(["--load-info-json", "live.info.json"])
        .current_dir(directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let part = directory.path().join("o.mp4.part");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !part.exists() {
        assert!(Instant::now() < deadline, "FFmpeg never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("[ffmpeg] Interrupted by user\n"),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"finished"
    );
    assert!(!part.exists());
}
