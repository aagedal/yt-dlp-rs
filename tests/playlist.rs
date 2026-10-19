//! CLI playlist selection operates on fully extracted local HTML5 playlists.
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
const PAGE: &str = "<title>Playlist selection</title><video><source src='/one.mp4' height='360'></video><video><source src='/two.mp4' height='720'></video><video><source src='/three.mp4' height='1080'></video><video><source src='/four.mp4' height='2160'></video>";
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
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                if !request.windows(4).any(|v| v == b"\r\n\r\n") {
                    continue;
                }
                let text = String::from_utf8(request).unwrap();
                let path = text.split_whitespace().nth(1).unwrap().to_owned();
                record.lock().unwrap().push(path.clone());
                let (status, mime, body) = match path.as_str() {
                    "/playlist.html" => (200, "text/html", PAGE),
                    "/bad.html" => (
                        200,
                        "text/html",
                        "<video src='/one.mp4'></video><video src='/missing.mp4'></video>",
                    ),
                    "/one.mp4" => (200, "video/mp4", "one"),
                    "/two.mp4" => (200, "video/mp4", "two"),
                    "/three.mp4" => (200, "video/mp4", "three"),
                    "/four.mp4" => (200, "video/mp4", "four"),
                    _ => (404, "text/plain", "missing"),
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
    fn cli(&self, path: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(args)
            .arg(self.url(path))
            .output()
            .unwrap()
    }
    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v.as_str() == path)
            .count()
    }
    fn assert_probed_once(&self) {
        for path in ["/one.mp4", "/two.mp4", "/three.mp4", "/four.mp4"] {
            assert_eq!(self.count(path), 1, "{path}")
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn success(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}
fn entries(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter(|v| v.starts_with('{'))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn assert_metadata(items: &[Value], indices: &[usize]) {
    assert_eq!(items.len(), indices.len());
    for (position, (info, index)) in items.iter().zip(indices).enumerate() {
        assert_eq!(info["id"], format!("playlist-{index}"));
        assert_eq!(info["playlist_index"], *index);
        assert_eq!(info["playlist_autonumber"], position + 1);
        assert_eq!(info["playlist_count"], 4);
        assert_eq!(info["n_entries"], indices.len());
    }
}
fn archive_args<'a>(archive: &'a Path, args: &[&'a str]) -> Vec<&'a str> {
    let mut values = vec!["--download-archive", archive.to_str().unwrap()];
    values.extend_from_slice(args);
    values
}

#[test]
fn request_order_ranges_negative_indices_dedup_reverse_and_default_metadata_reach_json() {
    for (args, indices, warning) in [
        (vec![], vec![1, 2, 3, 4], false),
        (
            vec!["-I", "3,1:4:2,-1,3", "--playlist-reverse"],
            vec![4, 1, 3],
            false,
        ),
        (vec!["--playlist-items=-1:-4:-2"], vec![4, 2], false),
        (vec!["-I", "2:inf:2"], vec![2, 4], false),
        (vec!["-I", ":infinite:-1"], vec![4, 3, 2, 1], false),
        (vec!["-I", ":inf:-1"], vec![], false),
        (vec!["-I", "0"], vec![], false),
        (
            vec!["--playlist-start", "2", "--playlist-end", "3"],
            vec![2, 3],
            false,
        ),
        (
            vec!["-I", "2-3", "--playlist-start", "1", "--playlist-end", "4"],
            vec![2, 3],
            true,
        ),
        (vec!["-I", "2-3", "--playlist-end=-1"], vec![2, 3], false),
    ] {
        let server = Server::new();
        let mut flags = vec!["-j"];
        flags.extend(args);
        let output = server.cli("/playlist.html", &flags);
        let stdout = success(&output);
        assert_eq!(
            String::from_utf8_lossy(&output.stderr)
                .contains("WARNING: Ignoring --playlist-start and --playlist-end"),
            warning
        );
        assert_metadata(&entries(&stdout), &indices);
        server.assert_probed_once();
    }
}

#[test]
fn playlist_flags_do_not_filter_or_annotate_a_single_direct_video() {
    let server = Server::new();
    let stdout = success(&server.cli("/one.mp4", &["-j", "-I", "99", "--playlist-reverse"]));
    let info = entries(&stdout).remove(0);
    assert_eq!(info["id"], "one");
    for key in [
        "playlist_index",
        "playlist_autonumber",
        "playlist_count",
        "n_entries",
    ] {
        assert!(info.get(key).is_none(), "{key}")
    }
    assert_eq!(server.count("/one.mp4"), 1);
}

#[test]
fn invalid_playlist_options_fail_preflight_even_when_items_override_the_range() {
    let server = Server::new();
    for args in [
        vec!["-j", "-I", "1:4:0"],
        vec!["-j", "-I", "x"],
        vec!["-j", "--playlist-start=-1"],
        vec!["-j", "--playlist-start", "0"],
        vec!["-j", "-I", "99", "-f", "best[unknown=1]"],
        vec![
            "-j",
            "-I",
            "1",
            "--playlist-start",
            "3",
            "--playlist-end",
            "2",
        ],
    ] {
        let output = server.cli("/playlist.html", &args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    assert!(server.requests.lock().unwrap().is_empty());
}

#[test]
fn selected_downloads_follow_request_order_without_downloading_excluded_entries() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let template = directory.path().join("%(id)s.%(ext)s");
    success(&server.cli(
        "/playlist.html",
        &["-I", "3,1", "-o", template.to_str().unwrap()],
    ));
    assert_eq!(
        std::fs::read(directory.path().join("playlist-3.mp4")).unwrap(),
        b"three"
    );
    assert_eq!(
        std::fs::read(directory.path().join("playlist-1.mp4")).unwrap(),
        b"one"
    );
    assert!(!directory.path().join("playlist-2.mp4").exists());
    assert!(!directory.path().join("playlist-4.mp4").exists());
    assert_eq!(
        &server.requests.lock().unwrap()[5..],
        &["/three.mp4", "/one.mp4"]
    );
}

#[test]
fn excluded_format_failures_are_ignored_but_retained_failures_are_atomic() {
    let server = Server::new();
    let stdout = success(&server.cli(
        "/playlist.html",
        &["-j", "-I", "1:2", "-f", "best[height<=720]"],
    ));
    assert_metadata(&entries(&stdout), &[1, 2]);
    server.assert_probed_once();
    let failure_server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let template = directory.path().join("%(id)s.%(ext)s");
    let failed = failure_server.cli(
        "/playlist.html",
        &[
            "-I",
            "1,3",
            "-f",
            "best[height<=720]",
            "-o",
            template.to_str().unwrap(),
        ],
    );
    assert!(!failed.status.success());
    // No JSON is requested: stdout holds only bracketed screen lines.
    let stdout = String::from_utf8_lossy(&failed.stdout);
    assert!(
        stdout
            .lines()
            .all(|line| line.trim_start_matches('\r').starts_with('[')),
        "{stdout}"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    failure_server.assert_probed_once();
    let extraction_server = Server::new();
    let failed = extraction_server.cli("/bad.html", &["-j", "-I", "1"]);
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert_eq!(extraction_server.count("/missing.mp4"), 1);
}

#[test]
fn archive_filtering_keeps_selected_request_order_indices_and_counts() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    std::fs::write(&archive, "html5mediaembed playlist-3\n").unwrap();
    let stdout = success(&server.cli(
        "/playlist.html",
        &archive_args(&archive, &["-j", "-I", "3,1,4"]),
    ));
    let items = entries(&stdout);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["playlist_index"], 1);
    assert_eq!(items[0]["playlist_autonumber"], 2);
    assert_eq!(items[1]["playlist_index"], 4);
    assert_eq!(items[1]["playlist_autonumber"], 3);
    for info in &items {
        assert_eq!(info["playlist_count"], 4);
        assert_eq!(info["n_entries"], 3)
    }
    assert_eq!(
        std::fs::read_to_string(archive).unwrap(),
        "html5mediaembed playlist-3\n"
    );
    server.assert_probed_once();
}

#[test]
fn listing_obeys_playlist_order_and_empty_selection_succeeds_without_output() {
    let server = Server::new();
    let stdout = success(&server.cli("/playlist.html", &["-F", "-j", "-I", "4,2"]));
    assert_metadata(&entries(&stdout), &[4, 2]);
    assert!(
        stdout.find("Available formats for playlist-4").unwrap()
            < stdout.find("Available formats for playlist-2").unwrap()
    );
    server.assert_probed_once();
    let empty = Server::new();
    let output = empty.cli("/playlist.html", &["-F", "-j", "-I", "99"]);
    success(&output);
    assert!(output.stdout.is_empty());
    empty.assert_probed_once();
}

#[test]
fn later_urls_continue_after_playlist_format_failure_and_keep_their_own_selection() {
    let server = Server::new();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "-j",
            "-I",
            "3",
            "-f",
            "best[height<=720]/best[height<=?100]",
        ])
        .arg(server.url("/playlist.html"))
        .arg(server.url("/one.mp4"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let items = entries(&String::from_utf8(output.stdout).unwrap());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "one");
    assert!(items[0].get("playlist_index").is_none());
}

#[test]
fn dump_single_json_prints_one_object_per_url_with_selected_entries() {
    let server = Server::new();
    let stdout = success(&server.cli("/playlist.html", &["-J", "-I", "3,2"]));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout}");
    let value: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(value["_type"], "playlist");
    assert_eq!(value["playlist_count"], 4);
    assert_eq!(value["requested_entries"], serde_json::json!([3, 2]));
    let entries = value["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["playlist_index"], 3);
    assert!(entries[0]["format_id"].is_string(), "entries are selected");
    // Only extraction probes; -J implies simulation.
    assert_eq!(server.count("/three.mp4"), 1);
    // A single video prints the same object as -j, and both flags print both.
    let single = success(&server.cli("/one.mp4", &["-J"]));
    let json = success(&server.cli("/one.mp4", &["-j"]));
    assert_eq!(single, json);
    let both = success(&server.cli("/one.mp4", &["-j", "-J"]));
    assert_eq!(both, format!("{json}{json}"));
}

#[test]
fn playlist_progress_lines_follow_upstream_process_playlist() {
    let server = Server::new();
    let output = server.cli("/playlist.html", &["-s", "-I", "3,1"]);
    let stdout = success(&output);
    let lines: Vec<_> = stdout
        .lines()
        .filter(|line| {
            line.starts_with("[download] ")
                || line.starts_with("[generic] Playlist ")
                || line.contains("format(s)")
        })
        .collect();
    let title = lines[0]
        .strip_prefix("[download] Downloading playlist: ")
        .expect("playlist header");
    assert_eq!(
        lines[1..],
        [
            format!("[generic] Playlist {title}: Downloading 2 items of 4"),
            "[download] Downloading item 1 of 2".into(),
            "[info] playlist-3: Downloading 1 format(s): 0".into(),
            "[download] Downloading item 2 of 2".into(),
            "[info] playlist-1: Downloading 1 format(s): 0".into(),
            format!("[download] Finished downloading playlist: {title}"),
        ]
    );
}
