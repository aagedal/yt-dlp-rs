//! Partial downloads of indexed single-file streams (YouTube's `initRange`
//! and `indexRange`): only the initialization, index and the section's
//! segments are requested, and FFmpeg cuts the local file. Formats without a
//! usable index, servers without ranged responses and `--downloader ffmpeg`
//! keep upstream's `FFmpegFD` reading the URL.
#![cfg(unix)]
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};
use yt_dlp_rs::{ByteIndex, PyNumber, VideoInfo, YoutubeDl};

fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(body);
    bytes
}

/// A fragmented MP4: `ftyp`+`moov`, a `sidx` of six 2-second segments of
/// 1000 bytes (segment `n` filled with byte `n`), then the segments.
fn media() -> (Vec<u8>, ByteIndex) {
    let mut file = mp4_box(b"ftyp", b"iso5\0\0\0\0");
    file.extend(mp4_box(b"moov", &[0; 24]));
    let init_end = file.len() as u64 - 1;
    let mut sidx = vec![0; 8];
    sidx.extend(1000u32.to_be_bytes());
    sidx.extend([0; 8]);
    sidx.extend([0, 0, 0, 6]);
    for _ in 0..6 {
        sidx.extend(1000u32.to_be_bytes());
        sidx.extend(2000u32.to_be_bytes());
        sidx.extend(0x9000_0000u32.to_be_bytes());
    }
    file.extend(mp4_box(b"sidx", &sidx));
    let index = (init_end + 1, file.len() as u64 - 1);
    for segment in 0..6u8 {
        file.extend([segment; 1000]);
    }
    (
        file,
        ByteIndex {
            init: (0, init_end),
            index,
        },
    )
}

/// Serve `body` at any path, honouring single `Range` requests unless
/// `ranges` is false; each request's `Range` header (or `-`) is recorded.
/// Request number `cut` (zero-based) sends only half its body.
fn serve(body: Vec<u8>, ranges: bool) -> (String, Arc<Mutex<Vec<String>>>) {
    serve_cut(body, ranges, None)
}

fn serve_cut(body: Vec<u8>, ranges: bool, cut: Option<usize>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let requests = log.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => request.extend_from_slice(&buffer[..count]),
                }
            }
            let request = String::from_utf8_lossy(&request).to_lowercase();
            let range = request
                .lines()
                .find_map(|line| line.strip_prefix("range: bytes="))
                .map(str::to_owned);
            let number = {
                let mut log = log.lock().unwrap();
                log.push(range.clone().unwrap_or_else(|| "-".into()));
                log.len() - 1
            };
            let half = |length: usize| {
                if cut == Some(number) {
                    length / 2
                } else {
                    length
                }
            };
            let span = range.filter(|_| ranges).and_then(|range| {
                let (start, end) = range.split_once('-')?;
                let start: usize = start.parse().ok()?;
                let end: usize = end.parse::<usize>().ok()?.min(body.len() - 1);
                Some((start, end))
            });
            let _ = match span {
                Some((start, end)) => write!(
                    stream,
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len(),
                    end + 1 - start
                )
                .and_then(|()| stream.write_all(&body[start..start + half(end + 1 - start)])),
                None => write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .and_then(|()| stream.write_all(&body)),
            };
        }
    });
    (origin, requests)
}

/// A recording stand-in: arguments go to `args.txt` and the first input
/// (a local `file:` input, else nothing) is copied to the output.
fn fake_ffmpeg(directory: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("ffmpeg");
    std::fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'ffmpeg version test'; exit 0; fi\n\
         for a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"${0%/ffmpeg}/args.txt\"\n\
         input=; previous=; for a in \"$@\"; do\n\
           if [ \"$previous\" = -i ] && [ -z \"$input\" ]; then input=\"$a\"; fi; previous=\"$a\"; last=\"$a\"; done\n\
         case \"$input\" in file:*) cp \"${input#file:}\" \"${last#file:}\";; *) printf remote > \"${last#file:}\";; esac\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn info(url: &str, index: Option<ByteIndex>) -> VideoInfo {
    let mut info: VideoInfo = serde_json::from_value(serde_json::json!({
        "id": "v", "title": "V", "url": url, "webpage_url": url, "ext": "mp4",
        "extractor": "generic", "extractor_key": "Generic", "direct": true,
        "protocol": "http", "format_id": "v",
        "formats": [{"format_id": "v", "url": url, "ext": "mp4", "protocol": "http"}]}))
    .unwrap();
    info.formats[0].byte_index = index;
    info.section_start = Some(PyNumber::Int(5));
    info.section_end = Some(PyNumber::Int(9));
    info
}

fn args(directory: &Path) -> String {
    std::fs::read_to_string(directory.join("args.txt")).unwrap()
}

#[test]
fn only_the_sections_segments_are_fetched_and_cut_locally() {
    let (file, ranges) = media();
    let (origin, requests) = serve(file.clone(), true);
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(directory.path());
    let url = format!("{origin}/v.mp4");
    let destination = directory.path().join("o.mp4");
    YoutubeDl::new()
        .unwrap()
        .with_ffmpeg(&ffmpeg)
        .download(&info(&url, Some(ranges)), &destination)
        .unwrap();
    // 5-9 s: the segment starting at 4 s through the one starting at 10 s.
    let first = ranges.index.1 as usize + 1;
    let expected = [
        &file[..=ranges.init.1 as usize],
        &file[first + 2000..first + 6000],
    ]
    .concat();
    assert_eq!(std::fs::read(&destination).unwrap(), expected);
    assert_eq!(
        *requests.lock().unwrap(),
        [
            format!("0-{}", ranges.index.1),
            format!("{}-{}", first + 2000, first + 5999)
        ]
    );
    let args = args(directory.path());
    assert!(
        args.starts_with("-y\n-hide_banner\n-seek_timestamp\n1\n-ss\n5.0\n-t\n4\n-i\nfile:"),
        "{args}"
    );
    assert!(args.contains("\n-c\ncopy\n-f\nmp4\nfile:"), "{args}");
    assert!(!args.contains("-headers") && !args.contains(&url), "{args}");
    // The local input was a private temporary file, since removed.
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        names.iter().all(|name| !name.ends_with(".part")),
        "{names:?}"
    );
}

#[test]
fn formats_without_a_usable_index_or_ranges_let_ffmpeg_read_the_url() {
    let (file, ranges) = media();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(directory.path());
    let remote = |info: &VideoInfo, ytdl: YoutubeDl, name: &str| {
        let destination = directory.path().join(name);
        ytdl.with_ffmpeg(&ffmpeg)
            .download(info, &destination)
            .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"remote");
        let args = args(directory.path());
        assert!(
            args.contains(&format!("\n-ss\n5\n-t\n4\n-i\n{}\n", info.url)),
            "{args}"
        );
    };
    // No index: nothing is requested.
    let (origin, requests) = serve(file.clone(), true);
    let url = format!("{origin}/v.mp4");
    remote(&info(&url, None), YoutubeDl::new().unwrap(), "none.mp4");
    assert!(requests.lock().unwrap().is_empty());
    // An index that does not parse: only the head was requested.
    let wrong = ByteIndex {
        init: ranges.init,
        index: (ranges.init.1 + 1, ranges.init.1 + 4),
    };
    remote(
        &info(&url, Some(wrong)),
        YoutubeDl::new().unwrap(),
        "wrong.mp4",
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
    // A server answering ranges with the whole file.
    let (origin, requests) = serve(file, false);
    let url = format!("{origin}/v.mp4");
    remote(
        &info(&url, Some(ranges)),
        YoutubeDl::new().unwrap(),
        "whole.mp4",
    );
    assert_eq!(*requests.lock().unwrap(), [format!("0-{}", ranges.index.1)]);
    // `--downloader ffmpeg` names FFmpeg itself.
    requests.lock().unwrap().clear();
    remote(
        &info(&url, Some(ranges)),
        YoutubeDl::new().unwrap().with_downloader("default", true),
        "named.mp4",
    );
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn an_interrupted_segment_transfer_resumes_where_it_stopped() {
    let (file, ranges) = media();
    let (origin, requests) = serve_cut(file.clone(), true, Some(1));
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = fake_ffmpeg(directory.path());
    let destination = directory.path().join("o.mp4");
    YoutubeDl::new()
        .unwrap()
        .with_ffmpeg(&ffmpeg)
        .download(
            &info(&format!("{origin}/v.mp4"), Some(ranges)),
            &destination,
        )
        .unwrap();
    let first = ranges.index.1 as usize + 1;
    let expected = [
        &file[..=ranges.init.1 as usize],
        &file[first + 2000..first + 6000],
    ]
    .concat();
    assert_eq!(std::fs::read(&destination).unwrap(), expected);
    assert_eq!(
        *requests.lock().unwrap(),
        [
            format!("0-{}", ranges.index.1),
            format!("{}-{}", first + 2000, first + 5999),
            format!("{}-{}", first + 4000, first + 5999)
        ]
    );
}
