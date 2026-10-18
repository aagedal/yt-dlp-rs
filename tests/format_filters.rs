//! Local HTTP and CLI coverage for native format predicate selection.
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use yt_dlp_rs::YoutubeDl;

const FORMATS: &str = "<title>Filters</title><video><source src='/low.mp4' type='video/mp4' width='640' height='360'><source src='/medium.mp4' type='video/mp4' width='1280' height='720'><source src='/high.mp4' type='video/mp4' width='1920' height='1080'><source src='/unknown.mp4' type='video/mp4'><source src='/clip.webm' type='video/webm' width='1280' height='720'></video>";
const MASTER: &str = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"main\",NAME=\"English/+Main\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"english.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"main\",NAME=\"Norwegian\",LANGUAGE=\"nb\",URI=\"norwegian.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=1280x720,CODECS=\"avc1.4,mp4a.40.2\",AUDIO=\"main\"\nvideo.m3u8\n";
const MEDIA: &str =
    "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\nsegment.m4s\n#EXT-X-ENDLIST\n";

struct Server {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (done, record) = (stop.clone(), requests.clone());
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(length) => request.extend_from_slice(&buffer[..length]),
                    }
                }
                if !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    continue;
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                record.lock().unwrap().push(path.clone());
                let (status, mime, body) = match path.as_str() {
                    "/formats.html" => (200, "text/html", FORMATS),
                    "/playlist.html" => (
                        200,
                        "text/html",
                        "<title>Atomic filters</title><video><source src='/low.mp4' height='360'><source src='/medium.mp4' height='720'></video><video><source src='/high.mp4' height='1080'></video>",
                    ),
                    "/low.mp4" => (200, "video/mp4", "low"),
                    "/medium.mp4" => (200, "video/mp4", "medium"),
                    "/high.mp4" => (200, "video/mp4", "high"),
                    "/unknown.mp4" => (200, "video/mp4", "unknown"),
                    "/clip.webm" => (200, "video/webm", "webm"),
                    "/master.m3u8" => (200, "application/vnd.apple.mpegurl", MASTER),
                    "/video.m3u8" | "/english.m3u8" | "/norwegian.m3u8" => {
                        (200, "application/vnd.apple.mpegurl", MEDIA)
                    }
                    _ => (404, "text/plain", "unexpected request"),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body.as_bytes());
            }
        });
        Self {
            origin,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|value| value.as_str() == path)
            .count()
    }
    fn cli(&self, path: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(args)
            .arg(self.url(path))
            .output()
            .unwrap()
    }
    fn json(&self, path: &str, selector: &str) -> Value {
        let output = self.cli(path, &["-j", "-s", "-f", selector]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn assert_one_probe_per_progressive_source(&self) {
        for path in [
            "/low.mp4",
            "/medium.mp4",
            "/high.mp4",
            "/unknown.mp4",
            "/clip.webm",
        ] {
            assert_eq!(self.count(path), 1, "{path}");
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn cli_numeric_and_extension_filters_select_without_downloading() {
    let server = Server::new();
    let info = server.json("/formats.html", "best[height<=720][width>=1000][ext=mp4]");
    assert_eq!(info["format_id"], "1");
    assert_eq!(info["height"], 720);
    assert_eq!(info["width"], 1280);
    assert!(info["url"].as_str().unwrap().ends_with("/medium.mp4"));
    assert_eq!(info["formats"].as_array().unwrap().len(), 5);
    server.assert_one_probe_per_progressive_source();
}

#[test]
fn actual_download_uses_only_the_filtered_source_after_all_probes() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("chosen.mp4");
    let output = server.cli(
        "/formats.html",
        &[
            "-f",
            "best[height<=720][ext=mp4]",
            "-o",
            target.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(target).unwrap(), b"medium");
    assert_eq!(server.count("/medium.mp4"), 2);
    for path in ["/low.mp4", "/high.mp4", "/unknown.mp4", "/clip.webm"] {
        assert_eq!(
            server.count(path),
            1,
            "unselected source {path} was downloaded"
        );
    }
}

#[test]
fn unknown_numeric_values_require_explicit_inclusion() {
    let server = Server::new();
    let info = server.json("/formats.html", "best[height<=?100][ext=mp4]");
    assert_eq!(info["format_id"], "3");
    assert!(info.get("height").is_none());
    let output = server.cli(
        "/formats.html",
        &["-j", "-s", "-f", "best[height<=100][ext=mp4]"],
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn quoted_filter_delimiters_do_not_split_hls_pair_or_fallback_expression() {
    let server = Server::new();
    let info = server.json(
        "/master.m3u8",
        "bestvideo[height<=720]+bestaudio[format_note='English/+Main'][language=en]/best",
    );
    assert_eq!(info["format_id"], "hls-0+hls-audio-0");
    assert_eq!(info["requested_formats"][1]["language"], "en");
    assert_eq!(info["requested_formats"][1]["format_note"], "English/+Main");
    assert_eq!(
        info["requested_formats"][0]["fragments"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(server.count("/segment.m4s"), 0);
    assert_eq!(server.requests.lock().unwrap().len(), 4);
}

#[test]
fn invalid_predicates_fail_even_after_a_valid_fallback_branch() {
    let server = Server::new();
    for selector in [
        "best/best[unknown=1]",
        "best/best[height<=]",
        "best/best[height~=720]",
        "best/best[ext='mp4]",
    ] {
        let output = server.cli("/formats.html", &["-j", "-s", "-f", selector]);
        assert!(!output.status.success(), "{selector}");
        assert!(output.stdout.is_empty(), "{selector}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("format selection"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn playlist_filter_failure_is_atomic_for_json_and_downloads() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let template = directory.path().join("%(id)s.%(ext)s");
    for mode in [vec!["-j", "-s"], Vec::new()] {
        let mut args = mode;
        args.extend(["-f", "best[height<=720]", "-o", template.to_str().unwrap()]);
        let output = server.cli("/playlist.html", &args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    assert_eq!(server.count("/low.mp4"), 2);
    assert_eq!(server.count("/medium.mp4"), 2);
    assert_eq!(server.count("/high.mp4"), 2);
}

#[test]
fn direct_result_filters_use_metadata_without_inventing_a_format_id() {
    let server = Server::new();
    let info = YoutubeDl::new()
        .unwrap()
        .extract_info_with_format(
            &server.url("/medium.mp4"),
            Some("best[filesize=6][ext=mp4]"),
        )
        .unwrap();
    assert!(info.formats.is_empty());
    assert_eq!(info.format_id, None);
    assert_eq!(info.filesize, Some(6));
    let fallback = server.json(
        "/medium.mp4",
        "best[filesize<6]/worst[filesize>=6][ext=mp4]",
    );
    assert!(fallback.get("format_id").is_none_or(Value::is_null));
    assert_eq!(fallback["filesize"], 6);
    let output = server.cli("/medium.mp4", &["-j", "-s", "-f", "best[ext=webm]"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn text_predicates_chain_and_literal_id_filters_can_fall_back() {
    let server = Server::new();
    let info = server.json(
        "/formats.html",
        "2[height<720]/best[ext^=m][ext$=4][ext*=p][ext!=webm][height<1080]",
    );
    assert_eq!(info["format_id"], "1");
    assert_eq!(info["ext"], "mp4");
    server.assert_one_probe_per_progressive_source();
}
