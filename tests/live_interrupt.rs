//! Upstream `FFmpegFD.real_download`: interrupting a live download asks
//! FFmpeg to quit (`q` on its stdin) and completes it like a finished one.
#![cfg(unix)]
use std::{
    path::Path,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

/// A fake FFmpeg: answers `-version`; for the live input it writes `partial`
/// to its output (the last argument, a `file:` URL), then waits for `q` on
/// stdin and writes `finished` (with `STUCK` set it ignores `q` and exits on
/// SIGTERM instead); other runs (post-processing) copy their first input.
fn fake_ffmpeg(directory: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("ffmpeg");
    std::fs::write(
        &path,
        "#!/bin/sh\n\
         case \"$1\" in -version) echo 'ffmpeg version live-fixture'; exit 0;; esac\n\
         for out; do :; done\n\
         out=${out#file:}\n\
         case \"$*\" in *index.m3u8*) ;; *)\n\
           while [ $# -gt 0 ] && [ \"$1\" != -i ]; do shift; done\n\
           [ $# -gt 0 ] || exit 0\n\
           cp \"${2#file:}\" \"$out\"; exit 0;;\n\
         esac\n\
         printf partial > \"$out\"\n\
         if [ -n \"$STUCK\" ]; then trap 'exit 255' TERM; while :; do sleep 1; done; fi\n\
         while :; do c=$(dd bs=1 count=1 2>/dev/null); [ \"$c\" = q ] && break; [ -z \"$c\" ] && exit 9; done\n\
         printf finished > \"$out\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

/// Download the live fixture with `args`, sending `signal` once FFmpeg
/// writes `part` (relative to the working directory).
fn interrupt(
    directory: &Path,
    args: &[&str],
    part: &str,
    signal: &str,
    stuck: bool,
) -> (Output, Duration) {
    let ffmpeg = fake_ffmpeg(directory);
    std::fs::write(
        directory.join("live.info.json"),
        serde_json::json!({
            "id": "live", "title": "Live", "ext": "mp4",
            "url": "http://127.0.0.1:9/live/index.m3u8", "protocol": "m3u8_native",
            "live_status": "is_live", "webpage_url": "https://example.test/live",
            "extractor": "fixture", "extractor_key": "Fixture",
        })
        .to_string(),
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
    command
        .args(["--ignore-config", "--no-progress", "--fixup", "never"])
        .args(["--ffmpeg-location", &ffmpeg, "-o", "o.%(ext)s"])
        .args(args)
        .args(["--load-info-json", "live.info.json"])
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stuck {
        command.env("STUCK", "1");
    }
    let child = command.spawn().unwrap();
    let part = directory.join(part);
    let deadline = Instant::now() + Duration::from_secs(20);
    while !part.exists() {
        assert!(Instant::now() < deadline, "FFmpeg never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    let signalled = Instant::now();
    let status = Command::new("kill")
        .args([&format!("-{signal}"), &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let output = child.wait_with_output().unwrap();
    (output, signalled.elapsed())
}

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != "ffmpeg" && name != "live.info.json")
        .collect();
    names.sort();
    names
}

#[test]
fn an_interrupted_live_download_is_finished_and_published() {
    // Port extension: SIGTERM is handled like SIGINT.
    for signal in ["INT", "TERM"] {
        let directory = tempfile::tempdir().unwrap();
        let (output, _) = interrupt(directory.path(), &[], "o.mp4.part", signal, false);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{signal}: {stdout}{}",
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
        assert_eq!(names(directory.path()), ["o.mp4"]);
    }
}

/// A processed live recording is written beside its destination like
/// upstream's (so a killed one stays discoverable), and the private
/// workspace only exists while processing.
#[test]
fn a_processed_live_recording_is_written_beside_its_destination() {
    let directory = tempfile::tempdir().unwrap();
    let (output, _) = interrupt(
        directory.path(),
        &[
            "--embed-metadata",
            "--verbose",
            "--print",
            "after_move:filepath",
        ],
        "o.mp4.part",
        "TERM",
        false,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}{stderr}");
    assert!(
        stderr.contains("[download] Destination: o.mp4\n"),
        "{stderr}"
    );
    assert!(stdout.ends_with("o.mp4\n"), "{stdout}");
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"finished"
    );
    assert_eq!(names(directory.path()), ["o.mp4"]);
}

/// Port extension: an FFmpeg that does not quit is terminated after ten
/// seconds, and the download completes with what it wrote.
#[test]
fn a_stuck_ffmpeg_is_terminated_and_its_output_kept() {
    let directory = tempfile::tempdir().unwrap();
    let (output, elapsed) = interrupt(directory.path(), &[], "o.mp4.part", "INT", true);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr
            .contains("WARNING: [ffmpeg] FFmpeg did not quit within 10 seconds; terminating it\n"),
        "{stderr}"
    );
    assert!(elapsed < Duration::from_secs(14), "{elapsed:?}");
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"partial"
    );
}
