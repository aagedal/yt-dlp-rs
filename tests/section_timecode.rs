//! `--section-timecode` (port extension): the section's first frame gets
//! its timecode in the whole video from ffprobe's frame rate, as `-timecode`
//! on the FFmpeg cut, when the result is MP4/MOV. Recording FFmpeg/ffprobe
//! stand-ins; real FFmpeg was checked against full downloads.
#![cfg(unix)]
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let body = b"\0\0\0\x18ftypisom media bytes";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    origin
}

/// FFmpeg and ffprobe stand-ins appending their arguments to `calls.txt`
/// (one per line, each call ended by `--`). FFmpeg writes its last argument;
/// ffprobe reports 25 fps and, after a seek, a first packet at 20 s.
fn tools(directory: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let write = |name: &str, body: &str| {
        let path = directory.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nif [ \"$1\" = -version ]; then echo '{name} version test'; exit 0; fi\n\
                 {{ echo {name}; for a in \"$@\"; do printf '%s\\n' \"$a\"; done; echo --; }} >> \"${{0%/{name}}}/calls.txt\"\n{body}"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    write(
        "ffprobe",
        "case \"$*\" in *-read_intervals*) packets=',\"packets\":[{\"pts_time\":\"20.000000\"}]';; esac\n\
         echo \"{\\\"streams\\\":[{\\\"avg_frame_rate\\\":\\\"25/1\\\",\\\"r_frame_rate\\\":\\\"25/1\\\",\\\"start_time\\\":\\\"0.000000\\\"}],\\\"format\\\":{\\\"start_time\\\":\\\"0.000000\\\"}$packets}\"\n",
    );
    write(
        "ffmpeg",
        "for a in \"$@\"; do last=\"$a\"; done\nprintf 'section' > \"${last#file:}\"\n",
    )
}

/// `--load-info-json` of one format with `ext` and `vcodec`.
fn info(directory: &Path, origin: &str, ext: &str, vcodec: &str) -> String {
    let url = format!("{origin}/v.{ext}");
    let path = directory.join(format!("info-{ext}-{vcodec}.json"));
    std::fs::write(
        &path,
        serde_json::json!({"id": "tc", "title": "TC", "ext": ext, "duration": 60,
            "url": url, "webpage_url": url, "extractor": "generic", "extractor_key": "Generic",
            "format_id": "0", "vcodec": vcodec,
            "formats": [{"format_id": "0", "url": url, "ext": ext, "protocol": "http", "vcodec": vcodec}]})
        .to_string(),
    )
    .unwrap();
    path.to_str().unwrap().to_owned()
}

fn run(directory: &Path, args: &[&str]) -> Output {
    let _ = std::fs::remove_file(directory.join("calls.txt"));
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "--ffmpeg-location"])
        .arg(directory.join("ffmpeg"))
        .args(args)
        .current_dir(directory)
        .env("PATH", "")
        .output()
        .unwrap()
}

/// The recorded calls of `tool`, each as its arguments joined by newlines.
fn calls(directory: &Path, tool: &str) -> Vec<String> {
    let text = std::fs::read_to_string(directory.join("calls.txt")).unwrap_or_default();
    text.split("--\n")
        .filter_map(|call| call.strip_prefix(&format!("{tool}\n")))
        .map(str::to_owned)
        .collect()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn mp4_sections_start_at_the_first_frame_after_the_start() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    tools(directory.path());
    let info = info(directory.path(), &origin, "mp4", "avc1");
    let section = ["--download-sections", "*21.3-25", "-o", "o.%(ext)s"];
    let output = run(
        directory.path(),
        &[
            &["--section-timecode", "--load-info-json", &info][..],
            &section,
        ]
        .concat(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).contains("[download] Section timecode: 00:00:21:08 (25.0 fps)\n"),
        "{}",
        text(&output.stdout)
    );
    // An MP4 edit list starts at frame ceil(21.3 * 25) = 533; no seek probe.
    let probes = calls(directory.path(), "ffprobe");
    assert_eq!(probes.len(), 1, "{probes:?}");
    assert!(
        probes[0].ends_with(&format!("\n{origin}/v.mp4\n")),
        "{}",
        probes[0]
    );
    let cut = &calls(directory.path(), "ffmpeg")[0];
    assert!(cut.contains("\n-timecode\n00:00:21:08\n-f\nmp4\n"), "{cut}");
    // Without the flag, upstream's arguments.
    let output = run(
        directory.path(),
        &[
            &["--load-info-json", &info, "--force-overwrites"][..],
            &section,
        ]
        .concat(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(calls(directory.path(), "ffprobe").is_empty());
    assert!(!calls(directory.path(), "ffmpeg")[0].contains("-timecode"));
}

#[test]
fn webm_sections_need_a_remux_to_mp4_and_start_at_the_keyframe() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    tools(directory.path());
    let info = info(directory.path(), &origin, "webm", "vp9");
    let base = [
        "--section-timecode",
        "--load-info-json",
        &info,
        "--download-sections",
        "*21.3-25",
        "-o",
        "o.%(ext)s",
    ];
    let output = run(directory.path(), &base);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains(
        "WARNING: Not writing a section timecode: webm files cannot carry a timecode track; remux the video to mp4 or mov (e.g. --remux-video mp4)\n"
    ));
    assert!(calls(directory.path(), "ffprobe").is_empty());
    assert!(!calls(directory.path(), "ffmpeg")[0].contains("-timecode"));
    // Remuxed to MP4: the WebM cut starts at the keyframe sought to (20 s).
    let output = run(
        directory.path(),
        &[&base[..], &["--remux-video", "mp4", "--force-overwrites"]].concat(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    let probes = calls(directory.path(), "ffprobe");
    assert_eq!(probes.len(), 2, "{probes:?}");
    assert!(
        probes[1].contains("-read_intervals\n21.3%+#1\n"),
        "{}",
        probes[1]
    );
    let ffmpeg = calls(directory.path(), "ffmpeg");
    assert!(
        ffmpeg[0].contains("\n-timecode\n00:00:20:00\n-f\nwebm\n"),
        "{}",
        ffmpeg[0]
    );
    assert!(directory.path().join("o.mp4").is_file());
}

#[test]
fn audio_sections_and_missing_ffprobe_are_warned_about() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    tools(directory.path());
    let section = ["--download-sections", "*1-2", "-o", "o-%(ext)s.%(ext)s"];
    let audio = info(directory.path(), &origin, "m4a", "none");
    let output = run(
        directory.path(),
        &[
            &["--section-timecode", "--load-info-json", &audio][..],
            &section,
        ]
        .concat(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stderr)
            .contains("WARNING: Not writing a section timecode: the section has no video stream\n")
    );
    std::fs::remove_file(directory.path().join("ffprobe")).unwrap();
    let video = info(directory.path(), &origin, "mp4", "avc1");
    let output = run(
        directory.path(),
        &[
            &["--section-timecode", "--load-info-json", &video][..],
            &section,
        ]
        .concat(),
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains(
        "WARNING: Not writing a section timecode: ffprobe was not found (--ffprobe-location)\n"
    ));
    assert!(!calls(directory.path(), "ffmpeg")[0].contains("-timecode"));
}
