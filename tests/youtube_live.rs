//! YouTube `--live-from-start` downloads (upstream `DashSegmentsFD` over
//! `_live_adaptive_fragments`): the `youtube-live-from-start` fixture's
//! pages with a test-controlled `X-Head-Seqnum` and generated fragments.
mod support;

#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::{
    path::Path,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

const BASE: &str = "https://rr1.googlevideo.test/videoplayback?itag=";

/// Fragments `ITAG:SQ;` behind a bare URL whose HEAD answers the sequence
/// numbers `heads` yields (each fragment request waits `delay`).
fn proxy(heads: Arc<dyn Fn() -> i64 + Send + Sync>, delay: Duration) -> site_proxy::SiteProxy {
    let dynamic: site_proxy::Dynamic = Arc::new(move |method: &str, url: &str| {
        let rest = url.strip_prefix(BASE)?;
        let (itag, query) = rest.split_once('&')?;
        match query.strip_prefix("v=ls") {
            Some("") if method == "HEAD" => Some((
                200,
                vec![("X-Head-Seqnum".to_owned(), heads().to_string())],
                Vec::new(),
            )),
            Some(sq) => {
                let sq = sq.strip_prefix("&sq=")?;
                std::thread::sleep(delay);
                Some((
                    200,
                    vec![("Content-Type".to_owned(), "video/mp4".to_owned())],
                    format!("{itag}:{sq};").into_bytes(),
                ))
            }
            None => None,
        }
    });
    site_proxy::SiteProxy::start_with("youtube-live-from-start", Some(dynamic))
}

fn command(proxy: &site_proxy::SiteProxy, directory: &Path) -> Command {
    configure(
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs")),
        proxy,
        directory,
    )
}

fn configure(mut command: Command, proxy: &site_proxy::SiteProxy, directory: &Path) -> Command {
    support::stoppable(&mut command)
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
            "--live-from-start",
        ])
        .args(["--no-progress", "--fixup", "never", "-o", "%(id)s.%(ext)s"])
        .current_dir(directory)
        .stdin(Stdio::null());
    command
}

fn expected(itag: &str, last: i64) -> Vec<u8> {
    (0..=last)
        .flat_map(|sq| format!("{itag}:{sq};").into_bytes())
        .collect()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn a_live_format_downloads_every_fragment_up_to_the_head_sequence_number() {
    // The head grows once, then stays: the generator stops after its
    // no-fragment score passes 30.
    let served = Arc::new(AtomicI64::new(1));
    let heads = served.clone();
    let proxy = proxy(
        Arc::new(move || heads.fetch_add(1, Ordering::SeqCst).min(2)),
        Duration::ZERO,
    );
    let work = tempfile::tempdir().unwrap();
    let output = command(&proxy, work.path())
        .args(["-f", "140"])
        .arg("https://www.youtube.com/watch?v=LiveStart01")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "[dashsegments] Total fragments: unknown (live)\n[download] Destination: LiveStart01.m4a\n"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("[download] Download completed\n"),
        "{stdout}"
    );
    let last = (served.load(Ordering::SeqCst) - 1).min(2);
    assert_eq!(
        std::fs::read(work.path().join("LiveStart01.m4a")).unwrap(),
        expected("140", last)
    );
    // No `.ytdl` state for a live download.
    let names: Vec<_> = std::fs::read_dir(work.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["LiveStart01.m4a"]);
}

/// A fake FFmpeg merging by concatenating its `-i` inputs into the output.
fn fake_ffmpeg(directory: &Path) -> String {
    let path = support::fake_tool(
        directory,
        "ffmpeg",
        "#!/bin/sh\n\
         case \"$1\" in -version) echo 'ffmpeg version live-fixture'; exit 0;; esac\n\
         inputs=''\n\
         while [ $# -gt 1 ]; do [ \"$1\" = -i ] && inputs=\"$inputs $2\"; shift; done\n\
         cat $inputs > \"${1#file:}\"\n",
    );
    path.display().to_string()
}

#[test]
fn video_and_audio_download_in_parallel_then_merge() {
    let proxy = proxy(Arc::new(|| 1), Duration::ZERO);
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(tools.path());
    let work = tempfile::tempdir().unwrap();
    let output = command(&proxy, work.path())
        .args(["-f", "137+140", "--ffmpeg-location", &ffmpeg])
        .arg("https://www.youtube.com/watch?v=LiveStart01")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in [
        "[info] LiveStart01: Downloading 1 format(s): 137+140\n\
         [dashsegments] Total fragments: unknown (live)\n\
         [download] Destination: LiveStart01.f137.mp4\n\
         [dashsegments] Total fragments: unknown (live)\n\
         [download] Destination: LiveStart01.f140.mp4\n",
        "[Merger] Merging formats into \"LiveStart01.mp4\"\n",
        "Deleting original file LiveStart01.f137.mp4 (pass -k to keep)\n",
    ] {
        assert!(stdout.contains(line), "{stdout}");
    }
    assert_eq!(stdout.matches("[download] Download completed\n").count(), 2);
    let merged = std::fs::read(work.path().join("LiveStart01.mp4")).unwrap();
    assert_eq!(merged, [expected("137", 1), expected("140", 1)].concat());
}

#[test]
fn an_interrupted_live_from_start_download_is_completed() {
    // A long stream of slow fragments, interrupted once some arrived.
    let proxy = proxy(Arc::new(|| 1000), Duration::from_millis(100));
    let work = tempfile::tempdir().unwrap();
    let child = command(&proxy, work.path())
        .args(["-f", "140"])
        .arg("https://www.youtube.com/watch?v=LiveStart01")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let part = work.path().join("LiveStart01.m4a.part");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::fs::metadata(&part).map_or(0, |metadata| metadata.len()) < 12 {
        assert!(std::time::Instant::now() < deadline, "no fragment arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
    support::interrupt(&child);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", text(&output));
    let file = std::fs::read(work.path().join("LiveStart01.m4a")).unwrap();
    let text_file = String::from_utf8(file).unwrap();
    // Whole fragments in order from the start, far fewer than the stream's.
    let count = text_file.matches(';').count() as i64;
    assert!((2..1000).contains(&count), "{text_file}");
    assert_eq!(text_file.into_bytes(), expected("140", count - 1));
    assert!(!part.exists());
}

#[test]
fn keep_partial_on_stop_still_merges_a_stopped_live_from_start_download() {
    // Issue #73: the flag's stop used to cancel the parallel downloads and
    // leave the formats unmerged. Like upstream's interrupted live download
    // (and the run without the flag), it ends the recording, merges it and
    // succeeds.
    stop_a_live_from_start_download_keeping_partial(&[]);
}

#[test]
fn keep_partial_on_stop_merges_a_stopped_live_from_start_download_with_processors() {
    // Postprocessing (here FFmpegMetadata) wraps the download in its own
    // interrupt check, which must not cancel the signalled recording either.
    stop_a_live_from_start_download_keeping_partial(&["--embed-metadata"]);
}

// Unix only: `trap` sets the ignored disposition the child inherits.
#[cfg(unix)]
#[test]
fn keep_partial_on_stop_succeeds_with_sigint_ignored() {
    // A background job of a script starts with SIGINT ignored, which the
    // live download's handler keeps; its SIGTERM stop used to be left for
    // the run's own check, ending in `ERROR: Interrupted by user`.
    let mut ignoring = Command::new("sh");
    ignoring.args([
        "-c",
        "trap '' INT; exec \"$0\" \"$@\"",
        env!("CARGO_BIN_EXE_yt-dlp-rs"),
    ]);
    stop_keeping_partial(Some(ignoring), &[]);
}

fn stop_a_live_from_start_download_keeping_partial(extra: &[&str]) {
    stop_keeping_partial(None, extra);
}

/// Stop a merged live-from-start download with `--keep-partial-on-stop`,
/// run by `program` (default: the binary itself).
fn stop_keeping_partial(program: Option<Command>, extra: &[&str]) {
    let proxy = proxy(Arc::new(|| 1000), Duration::from_millis(100));
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(tools.path());
    let work = tempfile::tempdir().unwrap();
    let child = match program {
        Some(program) => configure(program, &proxy, work.path()),
        None => command(&proxy, work.path()),
    }
    .args(["-f", "137+140", "--ffmpeg-location", &ffmpeg])
    .args([
        "--keep-partial-on-stop",
        "--no-part",
        "--progress",
        "--newline",
    ])
    .args(["--progress-template", "download:%(progress.filename)s"])
    .args(["--print", "after_move:filepath"])
    .args(extra)
    .arg("https://www.youtube.com/watch?v=LiveStart01")
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    let audio = work.path().join("LiveStart01.f140.mp4");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::fs::metadata(&audio).map_or(0, |metadata| metadata.len()) < 12 {
        assert!(std::time::Instant::now() < deadline, "no fragment arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
    support::stop(&child);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", text(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Each format's status names its file (upstream `f['filepath']`).
    for name in ["LiveStart01.f137.mp4", "LiveStart01.f140.mp4"] {
        assert!(stdout.lines().any(|line| line == name), "{stdout}");
    }
    let merged = work.path().join("LiveStart01.mp4");
    let printed = std::fs::canonicalize(&merged).unwrap();
    assert!(
        stdout
            .lines()
            .any(|line| std::fs::canonicalize(line).ok().as_ref() == Some(&printed)),
        "{stdout}"
    );
    // The fake FFmpeg concatenates whole fragments of each format.
    let text_file = String::from_utf8(std::fs::read(&merged).unwrap()).unwrap();
    assert!(text_file.starts_with("137:0;"), "{text_file}");
    assert!(text_file.contains("140:0;"), "{text_file}");
    let names: Vec<_> = std::fs::read_dir(work.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["LiveStart01.mp4"]);
}

/// `--growing-file` (port extension): a fake FFmpeg recording its arguments
/// in `calls.txt`, reporting one `-progress` block on stdout and copying its
/// `-i` inputs (the named pipes the fragment downloads feed) in order into
/// the output. Unix only: the port feeds FFmpeg through named pipes there.
#[cfg(unix)]
fn growing_ffmpeg(directory: &Path) -> String {
    let path = support::fake_tool(
        directory,
        "ffmpeg",
        "#!/bin/sh\n\
         case \"$1\" in -version) echo 'ffmpeg version live-fixture'; exit 0;; esac\n\
         for a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"${0%/*}/calls.txt\"\n\
         inputs=''\n\
         while [ $# -gt 1 ]; do [ \"$1\" = -i ] && inputs=\"$inputs ${2#file:}\"; shift; done\n\
         cat $inputs > \"${1#file:}\"\n\
         printf 'out_time_us=1500000\\ntotal_size=24\\nprogress=end\\n'\n",
    );
    path.display().to_string()
}

// Unix only: named pipes (see `growing_ffmpeg`).
#[cfg(unix)]
#[test]
fn a_growing_file_is_fed_through_named_pipes_without_a_merge() {
    let proxy = proxy(Arc::new(|| 1), Duration::ZERO);
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = growing_ffmpeg(tools.path());
    let work = tempfile::tempdir().unwrap();
    let output = command(&proxy, work.path())
        .args([
            "-f",
            "137+140",
            "--ffmpeg-location",
            &ffmpeg,
            "--growing-file",
        ])
        .args(["--progress", "--newline"])
        .env("TZ", "UTC")
        .arg("https://www.youtube.com/watch?v=LiveStart01")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "[info] LiveStart01: Downloading 1 format(s): 137+140\n\
             [dashsegments] Total fragments: unknown (live)\n\
             [dashsegments] Total fragments: unknown (live)\n\
             [download] Destination: LiveStart01.mov\n"
        ),
        "{stdout}"
    );
    assert!(stdout.contains(", recorded 00:00:01\n"), "{stdout}");
    assert!(!stdout.contains("[Merger]"), "{stdout}");
    // One file at its final name: no format files, pipes or workspace left.
    let names: Vec<_> = std::fs::read_dir(work.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["LiveStart01.mov"]);
    assert_eq!(
        std::fs::read(work.path().join("LiveStart01.mov")).unwrap(),
        [expected("137", 1), expected("140", 1)].concat()
    );
    let calls = std::fs::read_to_string(tools.path().join("calls.txt")).unwrap();
    let args: Vec<&str> = calls.lines().collect();
    let inputs: Vec<&str> = args
        .windows(2)
        .filter(|pair| pair[0] == "-i")
        .map(|pair| pair[1])
        .collect();
    assert_eq!(inputs.len(), 2, "{calls}");
    for (index, input) in inputs.iter().enumerate() {
        assert!(input.starts_with("file:"), "{calls}");
        assert!(input.ends_with(&format!("/input-{index}")), "{calls}");
        assert!(input.contains("/.yt-dlp-rs-growing-"), "{calls}");
    }
    let tail = args[args.iter().position(|arg| *arg == "-c").unwrap()..].join(" ");
    // The stream's start (2024-01-01T00:00:00-08:00) as the time of day.
    assert_eq!(
        tail,
        "-c copy -map 0:0 -map 1:0 -nostats -progress pipe:1 -timecode 08:00:00:00 \
         -movflags +frag_keyframe -frag_duration 1000000 -flush_packets 1 -f mov \
         file:LiveStart01.mov"
    );
}

// Unix only: named pipes (see `growing_ffmpeg`).
#[cfg(unix)]
#[test]
fn an_interrupted_growing_file_keeps_whole_fragments() {
    let proxy = proxy(Arc::new(|| 1000), Duration::from_millis(100));
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = growing_ffmpeg(tools.path());
    let work = tempfile::tempdir().unwrap();
    let child = command(&proxy, work.path())
        .args(["-f", "140", "--ffmpeg-location", &ffmpeg, "--growing-file"])
        .arg("https://www.youtube.com/watch?v=LiveStart01")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let file = work.path().join("LiveStart01.mov");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::fs::metadata(&file).map_or(0, |metadata| metadata.len()) < 12 {
        assert!(std::time::Instant::now() < deadline, "no fragment arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
    // How apps stop a recording: SIGTERM, which completes it like SIGINT.
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", text(&output));
    let text_file = String::from_utf8(std::fs::read(&file).unwrap()).unwrap();
    let count = text_file.matches(';').count() as i64;
    assert!((2..1000).contains(&count), "{text_file}");
    assert_eq!(text_file.into_bytes(), expected("140", count - 1));
    let names: Vec<_> = std::fs::read_dir(work.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["LiveStart01.mov"]);
}
