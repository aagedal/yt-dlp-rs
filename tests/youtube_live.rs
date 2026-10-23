//! YouTube `--live-from-start` downloads (upstream `DashSegmentsFD` over
//! `_live_adaptive_fragments`): the `youtube-live-from-start` fixture's
//! pages with a test-controlled `X-Head-Seqnum` and generated fragments.
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
    command
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
#[cfg(unix)]
fn fake_ffmpeg(directory: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("ffmpeg");
    std::fs::write(
        &path,
        "#!/bin/sh\n\
         case \"$1\" in -version) echo 'ffmpeg version live-fixture'; exit 0;; esac\n\
         inputs=''\n\
         while [ $# -gt 1 ]; do [ \"$1\" = -i ] && inputs=\"$inputs $2\"; shift; done\n\
         cat $inputs > \"${1#file:}\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

#[cfg(unix)]
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

#[cfg(unix)]
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
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
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
