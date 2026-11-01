//! Port extension (`sidx_fd`): a DASH `SegmentBase` representation downloads
//! as byte-range fragments from its segment index, byte-identical to the
//! whole file; an unusable index keeps the whole-file download.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{Arc, Mutex},
    thread,
};

const VIDEO: &[u8] = include_bytes!("fixtures/dash_sidx/video.mp4");

fn mpd(index_range: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT8S\"><Period duration=\"PT8S\"><AdaptationSet contentType=\"video\" mimeType=\"video/mp4\"><Representation id=\"v\" codecs=\"avc1.f4000a\" bandwidth=\"32447\" width=\"160\" height=\"120\"><BaseURL>video.mp4</BaseURL><SegmentBase indexRange=\"{index_range}\"><Initialization range=\"0-818\"/></SegmentBase></Representation></AdaptationSet></Period></MPD>"
    )
}

/// Serve the MPD and the video with byte ranges; record each `Range`.
fn serve(manifest: String) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let record = ranges.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0; 4096];
            let mut request = Vec::new();
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(length) => request.extend_from_slice(&buffer[..length]),
                }
            }
            let request = String::from_utf8_lossy(&request).into_owned();
            let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
            let (mime, body): (&str, &[u8]) = if path == "/v.mpd" {
                ("application/dash+xml", manifest.as_bytes())
            } else {
                ("video/mp4", VIDEO)
            };
            let range = request
                .lines()
                .find_map(|line| line.strip_prefix("Range: bytes="))
                .map(str::to_owned);
            let head = match range.as_deref().and_then(|range| range.split_once('-')) {
                Some((start, end)) if path != "/v.mpd" => {
                    record.lock().unwrap().push(format!("{start}-{end}"));
                    let start: usize = start.parse().unwrap();
                    let end = end
                        .parse::<usize>()
                        .map_or(body.len() - 1, |end| end.min(body.len() - 1));
                    let part = &body[start..=end];
                    let _ = stream.write_all(format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Type: {mime}\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len(),
                        part.len()
                    ).as_bytes());
                    let _ = stream.write_all(part);
                    continue;
                }
                _ => format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                ),
            };
            if path != "/v.mpd" {
                record.lock().unwrap().push("whole".into());
            }
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    (base, ranges)
}

fn download(base: &str, args: &[&str]) -> (tempfile::TempDir, std::process::Output) {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "-o", "out.%(ext)s"])
        .args(args)
        .arg(format!("{base}/v.mpd"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    (directory, output)
}

#[test]
fn segment_base_downloads_as_index_fragments() {
    let (base, ranges) = serve(mpd("819-954"));
    let (directory, output) = download(&base, &["-N", "3"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("[dashsegments] Total fragments: 9"),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        VIDEO
    );
    let mut ranges = ranges.lock().unwrap().clone();
    // The index, then the head and the eight segments (in any order).
    assert_eq!(ranges.remove(0), "0-954");
    ranges.sort_by_key(|range| range.split('-').next().unwrap().parse::<u64>().unwrap());
    assert_eq!(
        ranges,
        [
            "0-954",
            "955-5578",
            "5579-9164",
            "9165-12848",
            "12849-17147",
            "17148-21218",
            "21219-24944",
            "24945-29013",
            "29014-32582"
        ]
    );
    // JSON output is upstream's: a direct format without fragments.
    let (_, output) = download(&base, &["-j"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("fragments").is_none());
}

#[test]
fn unusable_index_keeps_the_whole_file_download() {
    // An index range that is not a `sidx` box.
    let (base, ranges) = serve(mpd("100-200"));
    let (directory, output) = download(&base, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Total fragments"));
    assert_eq!(
        std::fs::read(directory.path().join("out.mp4")).unwrap(),
        VIDEO
    );
    assert_eq!(
        ranges.lock().unwrap().last().map(String::as_str),
        Some("whole")
    );
}
