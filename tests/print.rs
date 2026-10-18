//! `--print`/`-O` and `--get-*` output through local HTTP extraction and the CLI.
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
                    "/page.html" => (200, "text/html", "<title>Fixture: \"A/B\" Épisode</title><video><source src='/media.mp4' width='1280' height='720'></video>".to_owned()),
                    "/playlist.html" => {
                        let mut html = "<title>Five entries</title>".to_owned();
                        for i in 1..=5 {
                            html.push_str(&format!("<video src='/item/{i}.mp4'></video>"));
                        }
                        (200, "text/html", html)
                    }
                    "/media.mp4" => (200, "video/mp4", "media".to_owned()),
                    _ if path.starts_with("/item/") && path.ends_with(".mp4") => (200, "video/mp4", "entry".to_owned()),
                    _ => (404, "text/plain", "missing".to_owned()),
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
    fn cli(&self, paths: &[&str], args: &[&str], directory: &Path) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
        command.args(args).current_dir(directory);
        for path in paths {
            command.arg(self.url(path));
        }
        command.output().unwrap()
    }
    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v.as_str() == path)
            .count()
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

fn entries(directory: &Path) -> usize {
    std::fs::read_dir(directory).unwrap().count()
}

#[test]
fn print_templates_and_get_aliases_print_in_upstream_order_without_writing() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = server.cli(
        &["/page.html"],
        &[
            "--get-filename",
            "-g",
            "--get-id",
            "-e",
            "-O",
            "id,width",
            "--print",
            "VIDEO:%(title)s|%(height)05d|%(format_id|none)s|%(title)+j",
            "-O",
            "ext=",
            "--write-info-json",
            "-o",
            "out/%(title)s.%(ext)s",
        ],
        directory.path(),
    );
    let stdout = success(&output);
    let title = "Fixture: \"A/B\" Épisode (1)";
    assert_eq!(
        stdout,
        format!(
            "page-1\n1280\n{title}|00720|0|\"Fixture: \\\"A/B\\\" Épisode (1)\"\next = \"mp4\"\n{title}\npage-1\n{}\nout/Fixture_ _A_B_ Épisode (1).mp4\n",
            server.url("/media.mp4")
        )
    );
    // Quiet simulation: no diagnostics line, media download or info JSON.
    assert!(!String::from_utf8_lossy(&output.stderr).contains("[html5]"));
    assert_eq!(entries(directory.path()), 0);
    assert_eq!(server.count("/media.mp4"), 1);
}

#[test]
fn printing_runs_per_selected_playlist_entry_before_json() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let stdout = success(&server.cli(
        &["/playlist.html"],
        &[
            "-I",
            "4,2",
            "-O",
            "%(playlist_index)s/%(playlist_autonumber)s/%(n_entries)s",
            "-j",
        ],
        directory.path(),
    ));
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "4/1/2");
    assert_eq!(lines[2], "2/2/2");
    for line in [lines[1], lines[3]] {
        let json: Value = serde_json::from_str(line).unwrap();
        assert_eq!(json["extractor"], "html5");
    }
    assert_eq!(entries(directory.path()), 0);
}

#[test]
fn listing_with_print_keeps_selection_and_prints_after_the_table() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let stdout = success(&server.cli(
        &["/page.html"],
        &["-F", "-O", "%(format_id)s:%(urls)s"],
        directory.path(),
    ));
    let (table, printed) = stdout.trim_end().rsplit_once('\n').unwrap();
    assert!(table.contains("ID"), "{table}");
    assert_eq!(printed, format!("0:{}", server.url("/media.mp4")));
}

#[test]
fn filename_errors_only_fail_templates_that_use_filename() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let stdout = success(&server.cli(
        &["/page.html"],
        &["-O", "%(id)s", "-o", "-"],
        directory.path(),
    ));
    assert_eq!(stdout, "page-1\n");
    let output = server.cli(
        &["/page.html", "/media.mp4"],
        &["-e", "--get-filename", "-o", "%(width|)s"],
        directory.path(),
    );
    assert!(!output.status.success());
    // The direct file has no width, so its empty filename fails and prints nothing.
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Fixture: \"A/B\" Épisode (1)\n1280\n"
    );
    let output = server.cli(
        &["/page.html"],
        &["-e", "--get-filename", "-o", "-"],
        directory.path(),
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("output filename failed"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unsupported_print_stages_and_templates_fail_before_requests() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    for spec in [
        "playlist:title",
        "after_move:filepath",
        "{id,title}",
        "missing_field",
        "%(id)q",
    ] {
        let output = server.cli(&["/page.html"], &["-O", spec], directory.path());
        assert!(!output.status.success(), "{spec}");
        assert!(output.stdout.is_empty(), "{spec}");
    }
    assert_eq!(server.count("/page.html"), 0);
}

#[test]
fn print_to_file_appends_before_downloading_without_implying_simulation() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let args = [
        "--print-to-file",
        "video:id,width",
        "lists/%(extractor)s.txt",
        "--print-to-file",
        "%(filename)s",
        "names.txt",
        "-o",
        "%(id)s.%(ext)s",
    ];
    let output = server.cli(&["/page.html"], &args, directory.path());
    assert_eq!(success(&output), "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[info] Writing \"%(id)s\\n%(width)s\" to: lists/html5.txt"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read(directory.path().join("page-1.mp4")).unwrap(),
        b"media"
    );
    // A second run appends; the existing media now fails after the write.
    let output = server.cli(&["/page.html"], &args, directory.path());
    assert!(!output.status.success());
    let read = |name: &str| std::fs::read_to_string(directory.path().join(name)).unwrap();
    assert_eq!(read("lists/html5.txt"), "page-1\n1280\npage-1\n1280\n");
    assert_eq!(read("names.txt"), "page-1.mp4\npage-1.mp4\n");
}

#[test]
fn print_to_file_runs_under_print_simulation_and_validates_at_startup() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = server.cli(
        &["/playlist.html"],
        &[
            "-I",
            "2:3",
            "-O",
            "id",
            "--print-to-file",
            "%(playlist_index)s",
            "%(extractor)s.txt",
        ],
        directory.path(),
    );
    assert_eq!(success(&output), "playlist-2\nplaylist-3\n");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("[info] Writing"));
    assert_eq!(entries(directory.path()), 1);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("html5.txt")).unwrap(),
        "2\n3\n"
    );
    for args in [
        ["--print-to-file", "playlist:id", "x.txt"],
        ["--print-to-file", "id", "%(missing)s.txt"],
        ["--print-to-file", "{id}", "x.txt"],
    ] {
        let output = server.cli(&["/page.html"], &args, directory.path());
        assert!(!output.status.success(), "{args:?}");
    }
    assert_eq!(server.count("/page.html"), 0);
}
