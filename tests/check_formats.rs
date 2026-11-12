//! `--check-formats` / `--check-all-formats`: upstream `_check_formats`
//! test downloads (the first fragment here) before selection. Lines were
//! compared with pinned upstream on equivalent media.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
};

fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
            }
            let path = String::from_utf8_lossy(&request)
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .to_owned();
            let media = |name: &str| {
                format!(
                    "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:2.0,\n{name}0.ts\n#EXTINF:2.0,\n{name}1.ts\n#EXT-X-ENDLIST\n"
                )
                .into_bytes()
            };
            let (status, kind, body): (&str, &str, Vec<u8>) = match path.as_str() {
                "/master.m3u8" => (
                    "200 OK",
                    "application/vnd.apple.mpegurl",
                    b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=320x240\ngood.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=640x480\nbad.m3u8\n".to_vec(),
                ),
                "/good.m3u8" => ("200 OK", "application/vnd.apple.mpegurl", media("good")),
                "/bad.m3u8" => ("200 OK", "application/vnd.apple.mpegurl", media("gone")),
                path if path.starts_with("/good") => {
                    let mut packets = Vec::new();
                    for _ in 0..4 {
                        packets.push(0x47);
                        packets.extend([0; 187]);
                    }
                    ("200 OK", "video/mp2t", packets)
                }
                _ => ("404 Not Found", "text/plain", b"missing".to_vec()),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    origin
}

fn run(args: &[&str]) -> (String, String) {
    let origin = serve();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "-s"])
        .args(args)
        .arg(format!("{origin}/master.m3u8"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn selected_formats_are_tested_until_one_works() {
    let (stdout, stderr) = run(&["--check-formats"]);
    assert!(stdout.ends_with(
        "[info] Testing format 900\n[info] Testing format 100\n[info] master: Downloading 1 format(s): 100\n"
    ), "{stdout}");
    assert!(
        stderr.ends_with("WARNING: Unable to download format 900. Skipping...\n"),
        "{stderr}"
    );
    // Without checking, the broken best format is selected.
    let (stdout, _) = run(&["--no-check-formats"]);
    assert!(
        stdout.ends_with("[info] master: Downloading 1 format(s): 900\n"),
        "{stdout}"
    );
}

#[test]
fn all_formats_are_tested_best_first_before_selection() {
    let (stdout, _) = run(&["--check-all-formats"]);
    assert!(stdout.contains(
        "[info] Testing format 900\n[info] Unable to download format 900. Skipping...\n[info] Testing format 100\n[info] master: Downloading 1 format(s): 100\n"
    ), "{stdout}");
}

/// Formats loaded from info JSON have no fragments: HlsFD's test reads the
/// first one from the media playlist.
#[test]
fn loaded_hls_formats_are_tested_from_their_playlists() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let format = |id: &str, name: &str, height: u32| {
        serde_json::json!({"format_id": id, "url": format!("{origin}/{name}.m3u8"), "ext": "mp4",
            "protocol": "m3u8_native", "height": height})
    };
    let info = serde_json::json!({"id": "master", "title": "Master", "extractor": "generic",
        "extractor_key": "Generic", "webpage_url": format!("{origin}/master.m3u8"),
        "formats": [format("100", "good", 240), format("900", "bad", 480)]});
    let path = directory.path().join("info.json");
    std::fs::write(&path, info.to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--check-formats",
            "-O",
            "format_id",
            "--load-info-json",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "100\n");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .ends_with("WARNING: Unable to download format 900. Skipping...\n"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
