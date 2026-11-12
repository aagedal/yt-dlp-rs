//! Upstream's hidden format and downloader options, compared with pinned
//! upstream on the same info JSON: `--all-formats` (`-f all` in `format`'s
//! destination) and `--hls-prefer-native`/`--hls-prefer-ffmpeg`
//! (`hls_prefer_native` in `_get_suitable_downloader`).
mod support;

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
};

const PLAYLIST: &str =
    "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.0,\nseg0.ts\n#EXT-X-ENDLIST\n";

/// Serves `/a.mp4`, `/b.mp4`, `/hls.m3u8` and its segment.
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
            let (mime, body): (&str, &[u8]) = match path.as_str() {
                "/hls.m3u8" => ("application/vnd.apple.mpegurl", PLAYLIST.as_bytes()),
                "/seg0.ts" => ("video/mp2t", b"segment"),
                _ => ("video/mp4", b"media"),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    origin
}

/// Two HTTP formats and an `m3u8_native` one, best first: `b`, `a`, `hls`.
fn info(directory: &Path, origin: &str) -> String {
    let format = |id: &str, path: &str, protocol: &str, height: u32| {
        serde_json::json!({"format_id": id, "url": format!("{origin}{path}"), "ext": "mp4",
            "vcodec": "avc1", "acodec": "mp4a", "height": height, "protocol": protocol})
    };
    let info = serde_json::json!({
        "id": "vid", "title": "Vid", "extractor": "generic", "extractor_key": "Generic",
        "webpage_url": format!("{origin}/page"),
        "formats": [format("a", "/a.mp4", "http", 120), format("b", "/b.mp4", "http", 240),
            format("hls", "/hls.m3u8", "m3u8_native", 80)],
    });
    let path = directory.join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    path.display().to_string()
}

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress"])
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn all_formats_is_format_all_and_the_last_format_option_wins() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let info = info(directory.path(), &origin);
    for (args, printed) in [
        (&["--all-formats"][..], "b\na\nhls\n"),
        (&["-f", "a", "--all-formats"], "b\na\nhls\n"),
        (&["--all-formats", "-f", "a"], "a\n"),
    ] {
        let output = run(
            directory.path(),
            &[args, &["--load-info-json", &info, "-O", "format_id"]].concat(),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), printed, "{args:?}");
    }
}

#[test]
fn hls_prefer_ffmpeg_downloads_native_hls_with_ffmpeg_unless_native_comes_last() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let info = info(directory.path(), &origin);
    let ffmpeg = support::fake_tool(
        directory.path(),
        "ffmpeg",
        "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffmpeg version test'; exit 0; fi\n\
         for a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"${0%/*}/args.txt\"\n\
         for a in \"$@\"; do last=\"$a\"; done\nprintf ffmpeg > \"${last#file:}\"\n",
    );
    let ffmpeg = ffmpeg.to_str().unwrap();
    let args_file = directory.path().join("args.txt");
    for (prefer, ffmpeg_used) in [
        (&["--hls-prefer-ffmpeg"][..], true),
        (&["--hls-prefer-ffmpeg", "--hls-prefer-native"], false),
        (&["--hls-prefer-native", "--hls-prefer-ffmpeg"], true),
        (&[], false),
    ] {
        let _ = std::fs::remove_file(&args_file);
        let _ = std::fs::remove_file(directory.path().join("o.mp4"));
        let output = run(
            directory.path(),
            &[
                prefer,
                &["--load-info-json", &info, "-f", "hls", "--fixup", "never"],
                &["--ffmpeg-location", ffmpeg, "-o", "o.%(ext)s"],
            ]
            .concat(),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.contains("[hlsnative] Downloading m3u8 manifest"),
            !ffmpeg_used,
            "{prefer:?}: {stdout}"
        );
        assert_eq!(
            std::fs::read(directory.path().join("o.mp4")).unwrap(),
            if ffmpeg_used {
                &b"ffmpeg"[..]
            } else {
                b"segment"
            },
            "{prefer:?}"
        );
        if ffmpeg_used {
            let args = std::fs::read_to_string(&args_file).unwrap();
            assert!(
                args.contains(&format!("\n-i\n{origin}/hls.m3u8\n")),
                "{args}"
            );
        }
    }
}
