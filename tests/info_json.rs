//! Info JSON sidecars for local direct, HTML5 and playlist results.
use serde_json::{Value, json};
use std::{
    fs,
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
const SINGLE: &str = "<title>Single</title><video><source src='/one.mp4' height='360'></video>";
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
                    "/single.html" => (200, "text/html", SINGLE),
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
    fn cli(&self, dir: &Path, path: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .current_dir(dir)
            .args(args)
            .arg(self.url(path))
            .output()
            .unwrap()
    }
    fn count(&self, path: &str) -> usize {
        let requests = self.requests.lock().unwrap();
        requests.iter().filter(|v| v.as_str() == path).count()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn run(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stderr.clone()).unwrap()
}
fn read(path: &Path) -> (String, Value) {
    let text = fs::read_to_string(path).unwrap();
    let value = serde_json::from_str(&text).unwrap();
    (text, value)
}
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn video_info_json_is_cleaned_dumped_metadata_written_before_media() {
    let server = Server::new();
    let dir = tempfile::tempdir().unwrap();
    let dumped: Value =
        serde_json::from_slice(&server.cli(dir.path(), "/single.html", &["-j"]).stdout).unwrap();
    let stderr = run(&server.cli(dir.path(), "/single.html", &["--write-info-json"]));
    let id = dumped["id"].as_str().unwrap();
    let base = format!("{} [{id}]", dumped["title"].as_str().unwrap());
    assert_eq!(
        names(dir.path()),
        [format!("{base}.info.json"), format!("{base}.mp4")]
    );
    let (text, mut value) = read(&dir.path().join(format!("{base}.info.json")));
    assert!(
        text.starts_with("{\"id\": ") && text.contains("\", \""),
        "{text}"
    );
    assert!(!text.ends_with('\n'));
    assert!(value["epoch"].as_u64().unwrap() > 1_700_000_000);
    assert_eq!(value["_type"], "video");
    let object = value.as_object_mut().unwrap();
    object.remove("epoch");
    object.remove("_type");
    assert_eq!(value, dumped);
    let info = stderr
        .find(&format!(
            "[info] Writing video metadata as JSON to: {base}.info.json"
        ))
        .unwrap();
    assert!(
        info < stderr.find("[download] Destination").unwrap(),
        "{stderr}"
    );
}

#[test]
fn skip_download_writes_only_info_json_and_keeps_existing_files() {
    let server = Server::new();
    let dir = tempfile::tempdir().unwrap();
    let args = [
        "--skip-download",
        "--write-info-json",
        "-o",
        "out/%(id)s.%(ext)s",
    ];
    run(&server.cli(dir.path(), "/one.mp4", &args));
    let path = dir.path().join("out/one.info.json");
    assert_eq!(names(&dir.path().join("out")), ["one.info.json"]);
    assert_eq!(read(&path).1["id"], "one");
    fs::write(&path, "kept").unwrap();
    let mut keep = args.to_vec();
    keep.push("-w");
    let stderr = run(&server.cli(dir.path(), "/one.mp4", &keep));
    assert!(
        stderr.contains("[info] Video metadata is already present"),
        "{stderr}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "kept");
    // By default upstream overwrites metadata files.
    let stderr = run(&server.cli(dir.path(), "/one.mp4", &args));
    assert!(
        stderr.contains("[info] Writing video metadata as JSON to:"),
        "{stderr}"
    );
    assert_eq!(read(&path).1["id"], "one");
    assert_eq!(names(&dir.path().join("out")), ["one.info.json"]);
    assert_eq!(
        server.count("/one.mp4"),
        3,
        "probes only, no media download"
    );
}

#[test]
fn simulation_listing_json_output_and_negation_write_nothing() {
    let server = Server::new();
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["-j"],
        vec!["-s"],
        vec!["-F"],
        vec!["--list-subs"],
        vec!["--no-write-info-json", "--skip-download"],
    ] {
        let mut flags = vec!["--write-info-json"];
        flags.extend(args);
        run(&server.cli(dir.path(), "/playlist.html", &flags));
        assert!(names(dir.path()).is_empty(), "{flags:?}");
    }
    run(&server.cli(
        dir.path(),
        "/one.mp4",
        &[
            "--no-write-info-json",
            "--write-info-json",
            "--skip-download",
        ],
    ));
    assert_eq!(names(dir.path()), ["one [one].info.json"]);
}

#[test]
fn playlist_info_json_precedes_entries_with_upstream_counts_and_name() {
    let server = Server::new();
    let dir = tempfile::tempdir().unwrap();
    let stderr = run(&server.cli(
        dir.path(),
        "/playlist.html",
        &[
            "--write-info-json",
            "--skip-download",
            "-I",
            "3,2",
            "-o",
            "%(title)s-%(playlist_index)s-%(n_entries)s.%(ext)s",
        ],
    ));
    let names = names(dir.path());
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(
        names.contains(&"Playlist selection-0-2.info.json".to_owned()),
        "{names:?}"
    );
    let (_, mut playlist) = read(&dir.path().join("Playlist selection-0-2.info.json"));
    assert!(playlist["epoch"].is_u64());
    playlist.as_object_mut().unwrap().remove("epoch");
    let id = playlist["id"].clone();
    assert_eq!(
        playlist,
        json!({
            "id": id, "title": "Playlist selection",
            "webpage_url": server.url("/playlist.html"),
            "extractor": "generic", "extractor_key": "Generic",
            "playlist_count": 4, "_type": "playlist",
        })
    );
    let entry = names
        .iter()
        .find(|name| name.ends_with("-3-2.info.json"))
        .unwrap();
    let (_, entry) = read(&dir.path().join(entry));
    assert_eq!(
        (
            entry["playlist_index"].as_u64(),
            entry.get("playlist_autonumber")
        ),
        (Some(3), None)
    );
    let playlist_line = stderr.find("Writing playlist metadata").unwrap();
    assert!(
        playlist_line < stderr.find("Writing video metadata").unwrap(),
        "{stderr}"
    );
}

#[test]
fn fully_archived_playlists_still_write_playlist_info_json() {
    let server = Server::new();
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("archive.txt");
    let archive_arg = archive.to_str().unwrap();
    run(&server.cli(
        dir.path(),
        "/playlist.html",
        &[
            "--download-archive",
            archive_arg,
            "-o",
            "media/%(id)s.%(ext)s",
        ],
    ));
    let stderr = run(&server.cli(
        dir.path(),
        "/playlist.html",
        &[
            "--download-archive",
            archive_arg,
            "--write-info-json",
            "-o",
            "meta/%(title)s.%(ext)s",
        ],
    ));
    assert!(stderr.contains("has already been recorded in the archive"));
    assert_eq!(
        names(&dir.path().join("meta")),
        ["Playlist selection.info.json"]
    );
}

#[test]
fn reserved_and_colliding_paths_fail_before_writes_and_skip_the_playlist() {
    let server = Server::new();
    let dir = tempfile::tempdir().unwrap();
    let output = server.cli(
        dir.path(),
        "/one.mp4",
        &[
            "--write-info-json",
            "--download-archive",
            "one.info.json",
            "-o",
            "%(id)s.%(ext)s",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("collides with cookie file or download archive")
    );
    assert!(names(dir.path()).is_empty());
    let output = server.cli(
        dir.path(),
        "/playlist.html",
        &[
            "--write-info-json",
            "--cookies",
            "Playlist selection.info.json",
            "-o",
            "%(title)s.%(ext)s",
        ],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("collides with cookie file"));
    // Only the (empty) cookie jar is saved at exit; no metadata replaced it.
    assert_eq!(names(dir.path()), ["Playlist selection.info.json"]);
    let cookies = fs::read_to_string(dir.path().join("Playlist selection.info.json")).unwrap();
    assert!(!cookies.contains("_type"), "{cookies}");
    assert_eq!(server.count("/two.mp4"), 1, "entries are not downloaded");
}

fn load(dir: &Path, file: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .current_dir(dir)
        .arg("--load-info-json")
        .arg(file)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn load_info_json_reselects_and_downloads_without_extraction() {
    let server = Server::new();
    let source = tempfile::tempdir().unwrap();
    run(&server.cli(
        source.path(),
        "/playlist.html",
        &[
            "--write-info-json",
            "--skip-download",
            "-o",
            "%(id)s.%(ext)s",
        ],
    ));
    let pages = server.count("/playlist.html");
    // A video dictionary is processed directly: no page request, normal
    // selection over its saved formats, and the media download.
    let dir = tempfile::tempdir().unwrap();
    let entry = source.path().join("playlist-1.info.json");
    let stderr = run(&load(
        dir.path(),
        &entry,
        &["-o", "out.%(ext)s", "extra-url"],
    ));
    assert!(stderr.contains("WARNING: URLs are ignored due to --load-info-json"));
    assert!(stderr.contains("[info] playlist-1: Downloading 1 format(s): 0"));
    assert_eq!(fs::read(dir.path().join("out.mp4")).unwrap(), b"one");
    assert_eq!(server.count("/playlist.html"), pages);
    // A list of dictionaries is processed in order, with -j output.
    let list = dir.path().join("list.json");
    let entries: Vec<Value> = ["playlist-1", "playlist-2"]
        .iter()
        .map(|id| read(&source.path().join(format!("{id}.info.json"))).1)
        .collect();
    fs::write(&list, serde_json::to_vec(&entries).unwrap()).unwrap();
    let output = load(dir.path(), &list, &["-j"]);
    let ids: Vec<String> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(ids, ["playlist-1", "playlist-2"]);
    // Playlists lose their entries when loaded, so the URL is re-extracted.
    let playlist = source.path().join("playlist.info.json");
    let stderr = run(&load(dir.path(), &playlist, &["-s"]));
    assert!(stderr.contains(&format!(
        "WARNING: The info failed to download: There are no entries; trying with URL {}",
        server.url("/playlist.html")
    )));
    assert_eq!(server.count("/playlist.html"), pages + 1);
    // Invalid files fail without a URL error.
    let invalid = dir.path().join("invalid.json");
    fs::write(&invalid, "{").unwrap();
    let output = load(dir.path(), &invalid, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("ERROR: info JSON failed: "));
}
