//! Download archive CLI regressions using deterministic local media fixtures.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const MASTER: &str = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Audio\",URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360,CODECS=\"avc1.4,mp4a.40.2\",AUDIO=\"a\"\nvideo.m3u8\n";
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
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                if !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    continue;
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                let count = {
                    let mut requests = record.lock().unwrap();
                    requests.push(path.clone());
                    requests.iter().filter(|value| **value == path).count()
                };
                let (status, mime, body) = match path.as_str() {
                    "/low.mp4" => (200, "video/mp4", "low"),
                    "/medium.mp4" => (200, "video/mp4", "medium"),
                    "/high.mp4" => (200, "video/mp4", "higher"),
                    "/changing.mp4" if count == 1 => (200, "video/mp4", "probe"),
                    "/playlist.html" => (
                        200,
                        "text/html",
                        "<video><source src='/low.mp4' height='360'></video><video><source src='/high.mp4' height='1080'></video>",
                    ),
                    "/three.html" => (
                        200,
                        "text/html",
                        "<video><source src='/low.mp4' height='360'></video><video><source src='/medium.mp4' height='720'></video><video><source src='/high.mp4' height='1080'></video>",
                    ),
                    "/master.m3u8" => (200, "application/vnd.apple.mpegurl", MASTER),
                    "/video.m3u8" | "/audio.m3u8" => (200, "application/vnd.apple.mpegurl", MEDIA),
                    "/init.mp4" => (200, "video/mp4", "init"),
                    "/segment.m4s" => (200, "video/mp4", "segment"),
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
    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|value| value.as_str() == path)
            .count()
    }
    fn command(&self, archive: &Path, args: &[&str], paths: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
        command.arg("--download-archive").arg(archive).args(args);
        for path in paths {
            command.arg(self.url(path));
        }
        command
    }
    fn cli(&self, archive: &Path, args: &[&str], paths: &[&str]) -> Output {
        self.command(archive, args, paths).output().unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn succeeds(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn successful_download_records_exact_key_and_duplicate_urls_skip_downloads_in_this_and_later_runs()
{
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    let output = directory.path().join("low.mp4");
    succeeds(&server.cli(
        &archive,
        &["-o", output.to_str().unwrap()],
        &["/low.mp4", "/low.mp4"],
    ));
    assert_eq!(std::fs::read(&output).unwrap(), b"low");
    assert_eq!(std::fs::read_to_string(&archive).unwrap(), "generic low\n");
    assert_eq!(server.count("/low.mp4"), 3);
    succeeds(&server.cli(
        &archive,
        &["-f", "999", "-o", output.to_str().unwrap()],
        &["/low.mp4"],
    ));
    assert_eq!(server.count("/low.mp4"), 4);
    assert_eq!(std::fs::read_to_string(archive).unwrap(), "generic low\n");
}

#[test]
fn simulation_json_and_listing_never_create_or_append_archives() {
    for mode in [vec!["-s"], vec!["-j"], vec!["-F"], vec!["-F", "-j"]] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("archive.txt");
        succeeds(&server.cli(&archive, &mode, &["/low.mp4"]));
        assert!(!archive.exists());
        std::fs::write(&archive, "generic other\n").unwrap();
        succeeds(&server.cli(&archive, &mode, &["/low.mp4"]));
        assert_eq!(
            std::fs::read_to_string(&archive).unwrap(),
            "generic other\n"
        );
        std::fs::write(&archive, "generic low\n").unwrap();
        let skipped = server.cli(&archive, &mode, &["/low.mp4"]);
        succeeds(&skipped);
        assert!(skipped.stdout.is_empty());
        assert_eq!(server.count("/low.mp4"), 3);
    }
}

#[test]
fn failed_download_existing_destination_and_invalid_template_are_not_recorded() {
    for (path, existing, template) in [
        ("/changing.mp4", false, false),
        ("/low.mp4", true, false),
        ("/low.mp4", false, true),
    ] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("archive.txt");
        let target = directory.path().join("out.mp4");
        // The changing URL tests transport failure, independently of clobber refusal.
        if existing {
            std::fs::write(&target, b"keep").unwrap();
        }
        let output = server.cli(
            &archive,
            &[
                "--retries",
                "0",
                "-o",
                if template {
                    "%(unsupported)s"
                } else {
                    target.to_str().unwrap()
                },
            ],
            &[path],
        );
        assert!(!output.status.success());
        assert!(!archive.exists());
        if existing {
            assert_eq!(std::fs::read(target).unwrap(), b"keep");
        } else {
            assert!(!target.exists());
        }
    }
}

#[test]
fn archived_separate_hls_is_skipped_before_selection_or_ffmpeg_validation() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    std::fs::write(&archive, "generic master\n").unwrap();
    let output = server.cli(
        &archive,
        &[
            "-f",
            "999",
            "--ffmpeg-location",
            "/nonexistent-archive-ffmpeg",
        ],
        &["/master.m3u8"],
    );
    succeeds(&output);
    assert!(output.stdout.is_empty());
    assert_eq!(server.requests.lock().unwrap().len(), 3);
    assert_eq!(server.count("/init.mp4"), 0);
    assert_eq!(server.count("/segment.m4s"), 0);
}

#[test]
fn playlist_skip_precedes_selection_and_only_retained_successes_are_recorded() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    std::fs::write(&archive, "html5mediaembed playlist-1\n").unwrap();
    let template = directory.path().join("%(id)s.%(ext)s");
    succeeds(&server.cli(
        &archive,
        &["-f", "best[height>=720]", "-o", template.to_str().unwrap()],
        &["/playlist.html"],
    ));
    assert!(!directory.path().join("playlist-1.mp4").exists());
    assert_eq!(
        std::fs::read(directory.path().join("playlist-2.mp4")).unwrap(),
        b"higher"
    );
    assert_eq!(
        std::fs::read_to_string(archive).unwrap(),
        "html5mediaembed playlist-1\nhtml5mediaembed playlist-2\n"
    );
    assert_eq!(server.count("/low.mp4"), 1);
    assert_eq!(server.count("/high.mp4"), 2);
}

#[test]
fn all_retained_playlist_entries_are_selected_before_any_download_or_json() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    std::fs::write(&archive, "html5mediaembed three-1\n").unwrap();
    let template = directory.path().join("%(id)s.%(ext)s");
    for mode in [vec!["-j"], Vec::new()] {
        let mut args = mode;
        args.extend(["-f", "best[height<=720]", "-o", template.to_str().unwrap()]);
        let output = server.cli(&archive, &args, &["/three.html"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            std::fs::read_to_string(&archive).unwrap(),
            "html5mediaembed three-1\n"
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    for path in ["/low.mp4", "/medium.mp4", "/high.mp4"] {
        assert_eq!(server.count(path), 2);
    }
}

#[test]
fn archive_write_failure_keeps_published_media_and_continues_to_later_urls() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("missing-parent/archive.txt");
    let template = directory.path().join("%(id)s.%(ext)s");
    let output = server.cli(
        &archive,
        &["-o", template.to_str().unwrap()],
        &["/low.mp4", "/high.mp4"],
    );
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(directory.path().join("low.mp4")).unwrap(),
        b"low"
    );
    assert_eq!(
        std::fs::read(directory.path().join("high.mp4")).unwrap(),
        b"higher"
    );
    assert!(!archive.exists());
    assert!(!archive.parent().unwrap().exists());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr
            .matches("download completed, but archive update failed:")
            .count(),
        2,
        "{stderr}"
    );
    assert_eq!(server.count("/low.mp4"), 2);
    assert_eq!(server.count("/high.mp4"), 2);
}

#[test]
fn invalid_archive_inputs_fail_before_any_network_request() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let invalid = directory.path().join("invalid.txt");
    std::fs::write(&invalid, [0xff]).unwrap();
    for archive in [
        invalid.clone(),
        directory.path().to_path_buf(),
        invalid.join("child"),
        invalid.join("child/grandchild/archive"),
    ] {
        let output = server.cli(&archive, &["-j"], &["/low.mp4"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    assert!(server.requests.lock().unwrap().is_empty());
}

#[test]
fn a_failed_url_does_not_prevent_recording_a_later_success() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.txt");
    let target = directory.path().join("out.mp4");
    let output = server.cli(
        &archive,
        &["-o", target.to_str().unwrap()],
        &["/missing.mp4", "/high.mp4"],
    );
    assert!(!output.status.success());
    assert_eq!(std::fs::read(target).unwrap(), b"higher");
    assert_eq!(std::fs::read_to_string(archive).unwrap(), "generic high\n");
    assert_eq!(server.count("/missing.mp4"), 1);
}

fn merger() -> &'static Path {
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    HELPER.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap().keep();
        let source = directory.join("merger.rs");
        std::fs::write(&source, r#"
fn main() {
    let exe=std::env::current_exe().unwrap();
    assert!(!exe.parent().unwrap().join("archive.txt").exists(),"archive recorded before merge completed");
    let args:Vec<_>=std::env::args_os().skip(1).collect();
    if args==["-version"] { println!("ffmpeg version archive-fixture"); return; }
    let inputs:Vec<_>=args.windows(2).filter(|p|p[0]=="-i").collect();
    assert_eq!(inputs.len(),2);
    for input in inputs { assert_eq!(std::fs::read(&input[1]).unwrap(),b"initsegment"); }
    std::fs::write(args.last().unwrap(),b"merged").unwrap();
    if exe.file_stem().unwrap()=="fail" { std::process::exit(1); }
}
"#).unwrap();
        let binary = directory.join(format!("merger{}", std::env::consts::EXE_SUFFIX));
        assert!(Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into())).arg(source).args(["--edition=2024", "-Dwarnings", "-o"]).arg(&binary).status().unwrap().success());
        binary
    })
}

#[test]
fn archive_is_written_only_after_successful_merge_and_final_publication() {
    for name in ["ok", "fail"] {
        let server = Server::new();
        let directory = tempfile::tempdir().unwrap();
        let helper = directory
            .path()
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(merger(), &helper).unwrap();
        let archive = directory.path().join("archive.txt");
        let target = directory.path().join("out.mp4");
        let output = server.cli(
            &archive,
            &[
                "-f",
                "bv+ba",
                "--ffmpeg-location",
                helper.to_str().unwrap(),
                "-o",
                target.to_str().unwrap(),
            ],
            &["/master.m3u8"],
        );
        if name == "ok" {
            succeeds(&output);
            assert_eq!(std::fs::read(target).unwrap(), b"merged");
            assert_eq!(
                std::fs::read_to_string(archive).unwrap(),
                "generic master\n"
            );
        } else {
            assert!(!output.status.success());
            assert!(!target.exists());
            assert!(!archive.exists());
        }
    }
}
