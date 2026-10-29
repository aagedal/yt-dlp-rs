//! `--growing-file` (port extension): live streams FFmpeg reads itself are
//! written as one growing QuickTime file at the final name, tagged on macOS
//! with Blackmagic's recording attribute until the recording ends, and
//! FFmpeg's `-progress` reports become the recorded time. A recording FFmpeg
//! stand-in; real FFmpeg was checked against a local live HLS stream.
#![cfg(unix)]
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const TAG: &str = "com.blackmagicdesign.metadata:recording";

/// A fake FFmpeg: answers `-version`, appends its arguments to `calls.txt`
/// (each call ended by `--`), saves the output's recording attribute (if
/// any) to `tag.txt`, writes `partial` with one `-progress` report, then
/// waits for `q` on stdin and writes `finished` with a final report.
fn fake_ffmpeg(directory: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("ffmpeg");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
             case \"$1\" in -version) echo 'ffmpeg version growing-fixture'; exit 0;; esac\n\
             dir=${{0%/ffmpeg}}\n\
             {{ for a in \"$@\"; do printf '%s\\n' \"$a\"; done; echo --; }} >> \"$dir/calls.txt\"\n\
             for out; do :; done\n\
             out=${{out#file:}}\n\
             xattr -p {TAG} \"$out\" > \"$dir/tag.txt\" 2>/dev/null\n\
             printf partial > \"$out\"\n\
             printf 'out_time_us=2000000\\ntotal_size=7\\nprogress=continue\\n'\n\
             while :; do c=$(dd bs=1 count=1 2>/dev/null); [ \"$c\" = q ] && break; [ -z \"$c\" ] && exit 9; done\n\
             printf finished > \"$out\"\n\
             printf 'out_time_us=3500000\\ntotal_size=8\\nprogress=end\\n'\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

fn live_info(directory: &Path) {
    live_info_with(directory, serde_json::json!({}));
}

/// The live info JSON with `extra` fields added.
fn live_info_with(directory: &Path, extra: serde_json::Value) {
    let mut info = serde_json::json!({
        "id": "live", "title": "Live", "ext": "mp4",
        "url": "http://127.0.0.1:9/live/index.m3u8", "protocol": "m3u8_native",
        "live_status": "is_live", "webpage_url": "https://example.test/live",
        "extractor": "fixture", "extractor_key": "Fixture",
    });
    info.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    std::fs::write(directory.join("live.info.json"), info.to_string()).unwrap();
}

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
    command
        .args(["--ignore-config", "--growing-file", "-o", "o.%(ext)s"])
        .args(["--load-info-json", "live.info.json"])
        .current_dir(directory)
        .stdin(Stdio::null());
    command
}

#[cfg(target_os = "macos")]
fn tag(path: &Path) -> Option<String> {
    let output = Command::new("xattr")
        .args(["-p", TAG])
        .arg(path)
        .output()
        .unwrap();
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[test]
fn a_live_stream_is_recorded_into_a_growing_quicktime_file() {
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(tools.path());
    let work = tempfile::tempdir().unwrap();
    live_info(work.path());
    // The default fixup policy: FixupM3u8 would otherwise remux the file.
    let child = command(work.path())
        .args(["--ffmpeg-location", &ffmpeg, "--progress", "--newline"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let file = work.path().join("o.mov");
    let deadline = Instant::now() + Duration::from_secs(20);
    while std::fs::read(&file).ok().as_deref() != Some(b"partial") {
        assert!(Instant::now() < deadline, "FFmpeg never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    #[cfg(target_os = "macos")]
    assert!(tag(&file).is_some(), "not tagged while recording");
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
    for line in [
        "[download] Destination: o.mov\n",
        "[download]      7.00B at",
        ", recorded 00:00:02\n",
        ", recorded 00:00:03\n",
        "[ffmpeg] Interrupted by user\n",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }
    // Written at its final name, and never fixed up afterwards.
    assert_eq!(std::fs::read(&file).unwrap(), b"finished");
    assert!(!work.path().join("o.mov.part").exists());
    let calls = std::fs::read_to_string(tools.path().join("calls.txt")).unwrap();
    assert_eq!(calls.matches("\n--\n").count(), 1, "{calls}");
    let args: Vec<&str> = calls.lines().collect();
    let tail = args[args.iter().position(|arg| *arg == "-c").unwrap()..].join(" ");
    let timecode = &tail["-c copy -nostats -progress pipe:1 -timecode ".len()..][..11];
    assert_eq!(
        tail,
        format!(
            "-c copy -nostats -progress pipe:1 -timecode {timecode} -movflags +frag_keyframe \
             -frag_duration 1000000 -flush_packets 1 -f mov file:o.mov --"
        )
    );
    // A time of day with whole seconds.
    assert!(timecode.ends_with(":00") && timecode.as_bytes()[2] == b':');
    // Tagged before FFmpeg wrote it, untagged once finished.
    #[cfg(target_os = "macos")]
    {
        let written = std::fs::read_to_string(tools.path().join("tag.txt")).unwrap();
        assert!(written.starts_with("{\"r\":1, \"uuid\":\""), "{written}");
        assert_eq!(tag(&file), None);
    }
}

#[test]
fn growing_live_results_are_quicktime_files() {
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(tools.path());
    let work = tempfile::tempdir().unwrap();
    live_info(work.path());
    let output = command(work.path())
        .args(["--ffmpeg-location", &ffmpeg, "--simulate", "--print", "ext"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "mov\n");
}

/// FFmpeg's MOV muxer refuses AV1 ("av1 only supported in MP4 and AVIF"),
/// so an AV1 live stream grows as fragmented MP4, Resolve's AV1 container.
#[test]
fn an_av1_live_stream_grows_as_fragmented_mp4() {
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(tools.path());
    let work = tempfile::tempdir().unwrap();
    live_info_with(
        work.path(),
        serde_json::json!({"vcodec": "av01.0.08M.08", "acodec": "mp4a.40.2"}),
    );
    let child = command(work.path())
        .args(["--ffmpeg-location", &ffmpeg])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let file = work.path().join("o.mp4");
    let deadline = Instant::now() + Duration::from_secs(20);
    while std::fs::read(&file).ok().as_deref() != Some(b"partial") {
        assert!(Instant::now() < deadline, "FFmpeg never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    #[cfg(target_os = "macos")]
    assert!(tag(&file).is_some(), "not tagged while recording");
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"finished");
    let calls = std::fs::read_to_string(tools.path().join("calls.txt")).unwrap();
    assert!(
        calls.ends_with("\n-flush_packets\n1\n-f\nmp4\nfile:o.mp4\n--\n"),
        "{calls}"
    );
}

#[test]
fn without_ffmpeg_a_live_stream_downloads_normally() {
    let work = tempfile::tempdir().unwrap();
    live_info(work.path());
    let output = command(work.path())
        .args(["--simulate", "--print", "ext"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "mp4\n");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            "WARNING: --growing-file needs ffmpeg; downloading the live stream normally\n"
        ),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
