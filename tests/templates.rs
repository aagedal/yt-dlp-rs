//! Typed output templates exercised through local HTTP extraction and the CLI.
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
                let(status,mime,body)=match path.as_str(){
    "/typed.html"=>(200,"text/html","<title>Typed</title><video><source src='/media.mp4' width='1280' height='720'></video>".into()),
    "/unsafe.html"=>(200,"text/html","<title>..alpha/../../escape</title><video src='/media.mp4'></video>".into()),
    "/playlist.html"=>{let mut html="<title>Twelve entries</title>".to_owned();for i in 1..=12{html.push_str(&format!("<video src='/item/{i}.mp4'></video>"));}(200,"text/html",html)},
    "/media.mp4"=>(200,"video/mp4","media".into()),"/one.mp4"=>(200,"video/mp4","one".into()),"/12.5.mp4"=>(200,"video/mp4","numeric".into()),
    _ if path.starts_with("/item/")&&path.ends_with(".mp4")=>(200,"video/mp4",format!("entry{}",path.trim_start_matches("/item/").trim_end_matches(".mp4"))),
    _=>(404,"text/plain","missing".into())};
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
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn typed_dimensions_sizes_booleans_and_format_ids_control_downloaded_filename() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let output = server.cli(
        &["/typed.html"],
        &[
            "-o",
            "%(width)04d-%(height).1f-%(filesize)03d-%(direct)d-%(format_id)03d.%(ext)s",
        ],
        directory.path(),
    );
    success(&output);
    assert_eq!(
        std::fs::read(directory.path().join("1280-720.0-005-0-000.mp4")).unwrap(),
        b"media"
    );
    assert_eq!(server.count("/media.mp4"), 2);
}

#[test]
fn numeric_string_id_converts_to_integer_and_float_while_direct_is_numeric() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(
        &["/12.5.mp4"],
        &["-o", "%(id)05d-%(id).1f-%(direct)d.%(ext)s"],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("00012-12.5-1.mp4")).unwrap(),
        b"numeric"
    );
}

#[test]
fn missing_and_nonnumeric_fields_use_na_defaults_and_known_alternates() {
    for (extra, placeholder) in [
        (vec![], "NA"),
        (vec!["--output-na-placeholder", "MISSING"], "MISSING"),
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut args = vec![
            "-o",
            "%(height)06d-%(width|fallback)04d-%(height,title)s-%(ext)d-%(height|)s.%(ext)s",
        ];
        args.extend(extra);
        success(&server.cli(&["/media.mp4"], &args, directory.path()));
        let expected = format!("{placeholder}-fallback-media-{placeholder}-.mp4");
        assert_eq!(
            std::fs::read(directory.path().join(expected)).unwrap(),
            b"media"
        );
    }
}

#[test]
fn playlist_padding_uses_selected_original_max_and_selected_count_not_full_count() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(
        &["/playlist.html"],
        &[
            "-I",
            "3,12",
            "-o",
            "two/%(playlist_index)s-%(playlist_autonumber)s-%(n_entries)02d.%(ext)s",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("two/03-1-02.mp4")).unwrap(),
        b"entry3"
    );
    assert_eq!(
        std::fs::read(directory.path().join("two/12-2-02.mp4")).unwrap(),
        b"entry12"
    );
    success(&server.cli(
        &["/playlist.html"],
        &[
            "-I",
            "3",
            "-o",
            "one/%(playlist_index)s-%(playlist_autonumber)s.%(ext)s",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("one/3-1.mp4")).unwrap(),
        b"entry3"
    );
    success(&server.cli(
        &["/playlist.html"],
        &[
            "-o",
            "all/%(playlist_index)s-%(playlist_autonumber)s.%(ext)s",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("all/01-01.mp4")).unwrap(),
        b"entry1"
    );
    assert_eq!(
        std::fs::read(directory.path().join("all/12-12.mp4")).unwrap(),
        b"entry12"
    );
    let json = server.cli(&["/playlist.html"], &["-j", "-I", "3,12"], directory.path());
    success(&json);
    for line in String::from_utf8(json.stdout).unwrap().lines() {
        let info: Value = serde_json::from_str(line).unwrap();
        assert!(info.get("last_playlist_index").is_none());
        assert!(info.get("__last_playlist_index").is_none());
    }
}

#[test]
fn archive_skips_do_not_change_playlist_padding_context() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    std::fs::write(&archive, "html5mediaembed playlist-12\n").unwrap();
    success(&server.cli(
        &["/playlist.html"],
        &[
            "-I",
            "3,12",
            "--download-archive",
            archive.to_str().unwrap(),
            "-o",
            "%(playlist_index)s-%(playlist_autonumber)s.%(ext)s",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("03-1.mp4")).unwrap(),
        b"entry3"
    );
    assert!(!directory.path().join("3-1.mp4").exists());
    assert_eq!(server.count("/item/12.mp4"), 1);
}

#[test]
fn sanitize_before_precision_keeps_metadata_and_defaults_inside_literal_directories() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    success(&server.cli(
        &["/unsafe.html"],
        &[
            "-o",
            "literal-dir/%(title).2s-%(height|../fallback).2s.%(ext)s",
        ],
        directory.path(),
    ));
    assert_eq!(
        std::fs::read(directory.path().join("literal-dir/al-_fallback.mp4")).unwrap(),
        b"media"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    assert_eq!(
        std::fs::read_dir(directory.path().join("literal-dir"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn unsupported_malformed_and_excessive_formats_fail_after_probe_without_download_or_archive() {
    for template in [
        "%(unknown)s",
        "%(id)q",
        "%(id",
        "%(height)1000000000d",
        "%(height).9999999f",
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("archive.txt");
        let output = server.cli(
            &["/media.mp4"],
            &[
                "--download-archive",
                archive.to_str().unwrap(),
                "-o",
                template,
            ],
            directory.path(),
        );
        assert!(!output.status.success(), "{template}");
        assert_eq!(server.count("/media.mp4"), 1);
        assert!(!archive.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn readonly_modes_leave_invalid_template_and_placeholder_configuration_inert() {
    for mode in [vec!["-j"], vec!["-s"], vec!["-F"], vec!["-F", "-j"]] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let mut args = mode;
        args.extend([
            "-o",
            "%(unknown).9999999f",
            "--output-na-placeholder",
            "../unused",
        ]);
        let output = server.cli(&["/media.mp4"], &args, directory.path());
        success(&output);
        assert_eq!(server.count("/media.mp4"), 1);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn no_clobber_error_continues_to_later_typed_filename_and_records_only_success() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("NA.mp4"), b"keep").unwrap();
    let archive = directory.path().join("archive.txt");
    let output = server.cli(
        &["/one.mp4", "/12.5.mp4"],
        &[
            "--download-archive",
            archive.to_str().unwrap(),
            "-o",
            "%(title)03d.%(ext)s",
        ],
        directory.path(),
    );
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(directory.path().join("NA.mp4")).unwrap(),
        b"keep"
    );
    assert_eq!(
        std::fs::read(directory.path().join("012.mp4")).unwrap(),
        b"numeric"
    );
    assert_eq!(std::fs::read_to_string(archive).unwrap(), "generic 12.5\n");
    assert_eq!(server.count("/one.mp4"), 1);
    assert_eq!(server.count("/12.5.mp4"), 2);
}
